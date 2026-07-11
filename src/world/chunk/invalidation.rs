use std::collections::{HashMap, HashSet};

use super::{
    components::ChunkContentCounts,
    coords::{ChunkPos, LocalBlockPos},
    neighborhood::NeighborOffset,
    state::CellDelta,
};

const SAVE: u8 = 1 << 0;
const MESH: u8 = 1 << 1;
const COLLIDER: u8 = 1 << 2;
const LIGHT_REBUILD: u8 = 1 << 3;
const FLUID_STEP: u8 = 1 << 4;
const RENDER_LIGHT_UPLOAD: u8 = 1 << 5;

/// Coalesced derived work for one chunk.
///
/// The flags are intentionally opaque so callers consume the plan through
/// named effects rather than depending on its representation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkInvalidationEffects(u8);

impl ChunkInvalidationEffects {
    pub const NONE: Self = Self(0);

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn needs_save(self) -> bool {
        self.contains(SAVE)
    }

    pub const fn needs_mesh_rebuild(self) -> bool {
        self.contains(MESH)
    }

    pub const fn needs_collider_rebuild(self) -> bool {
        self.contains(COLLIDER)
    }

    pub const fn needs_light_rebuild(self) -> bool {
        self.contains(LIGHT_REBUILD)
    }

    pub const fn needs_fluid_step(self) -> bool {
        self.contains(FLUID_STEP)
    }

    pub const fn needs_render_light_upload(self) -> bool {
        self.contains(RENDER_LIGHT_UPLOAD)
    }

    const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    const fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    fn insert(&mut self, effects: Self) {
        self.0 |= effects.0;
    }
}

/// Classifies the direct effects of changing one cell, before spatial fanout.
pub fn classify_cell_delta(delta: CellDelta) -> ChunkInvalidationEffects {
    if delta.old == delta.new {
        return ChunkInvalidationEffects::NONE;
    }

    let old_meta = delta.old.hot_meta();
    let new_meta = delta.new.hot_meta();
    let mut bits = SAVE;

    if (
        old_meta.render_id,
        old_meta.mesh_flags,
        old_meta.fluid_level,
    ) != (
        new_meta.render_id,
        new_meta.mesh_flags,
        new_meta.fluid_level,
    ) {
        bits |= MESH;
    }
    if delta.old.is_solid() != delta.new.is_solid() {
        bits |= COLLIDER;
    }
    if (old_meta.light_opacity, old_meta.light_emission)
        != (new_meta.light_opacity, new_meta.light_emission)
    {
        bits |= LIGHT_REBUILD;
    }
    if delta.old.as_fluid() != delta.new.as_fluid() || delta.old.is_solid() != delta.new.is_solid()
    {
        bits |= FLUID_STEP;
    }

    ChunkInvalidationEffects::from_bits(bits)
}

/// The XZ address of a vertical chunk column whose lighting may be stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkColumn {
    x: i32,
    z: i32,
}

impl ChunkColumn {
    pub const fn new(x: i32, z: i32) -> Self {
        Self { x, z }
    }

    pub const fn from_chunk(chunk: ChunkPos) -> Self {
        let position = chunk.as_ivec3();
        Self::new(position.x, position.z)
    }

    pub const fn x(self) -> i32 {
        self.x
    }

    pub const fn z(self) -> i32 {
        self.z
    }

    pub const fn chunk(self, y: i32) -> ChunkPos {
        ChunkPos::new(self.x, y, self.z)
    }
}

impl From<ChunkPos> for ChunkColumn {
    fn from(chunk: ChunkPos) -> Self {
        Self::from_chunk(chunk)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ChunkWork {
    effects: ChunkInvalidationEffects,
}

/// A pure, coalescing description of chunk work for an ECS adapter to apply.
///
/// Chunk iteration covers directly addressed and halo chunks. Lighting columns
/// are exposed separately because a cell can invalidate every loaded Y chunk in
/// nearby XZ columns, which cannot be expanded without dimension ownership data.
#[derive(Debug, Default, Clone)]
pub struct ChunkInvalidationPlan {
    chunks: HashMap<ChunkPos, ChunkWork>,
    light_columns: HashSet<ChunkColumn>,
}

impl ChunkInvalidationPlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty() && self.light_columns.is_empty()
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn light_column_count(&self) -> usize {
        self.light_columns.len()
    }

    pub fn effects_for(&self, chunk: ChunkPos) -> Option<ChunkInvalidationEffects> {
        self.chunks.get(&chunk).map(|work| work.effects)
    }

    pub fn chunks(
        &self,
    ) -> impl ExactSizeIterator<Item = (ChunkPos, ChunkInvalidationEffects)> + '_ {
        self.chunks
            .iter()
            .map(|(&chunk, work)| (chunk, work.effects))
    }

    pub fn light_columns(&self) -> impl ExactSizeIterator<Item = ChunkColumn> + '_ {
        self.light_columns.iter().copied()
    }

    pub fn light_column_is_dirty(&self, column: ChunkColumn) -> bool {
        self.light_columns.contains(&column)
    }

    pub fn clear(&mut self) {
        self.chunks.clear();
        self.light_columns.clear();
    }

    pub fn record_cell_delta(
        &mut self,
        chunk: ChunkPos,
        local: LocalBlockPos,
        delta: CellDelta,
    ) -> ChunkInvalidationEffects {
        let effects = classify_cell_delta(delta);
        self.mark(chunk, effects);

        if effects.needs_light_rebuild() {
            self.record_cell_light_columns(chunk);
        }

        let mut halo = 0;
        if effects.needs_mesh_rebuild() {
            halo |= MESH;
        }
        if effects.needs_fluid_step() {
            halo |= FLUID_STEP;
        }
        if halo != 0 {
            let halo_effects = ChunkInvalidationEffects::from_bits(halo);
            for offset in NeighborOffset::touching(local) {
                self.mark_neighbor(chunk, offset, halo_effects);
            }
        }

        effects
    }

    pub fn record_chunk_loaded(&mut self, chunk: ChunkPos, contents: ChunkContentCounts) {
        let mut own = LIGHT_REBUILD;
        if contents.rendered > 0 {
            own |= MESH;
        }
        if contents.solid > 0 {
            own |= COLLIDER;
        }
        if contents.fluids > 0 {
            own |= FLUID_STEP;
        }

        self.mark(chunk, ChunkInvalidationEffects::from_bits(own));
        self.light_columns.insert(chunk.into());
        self.record_topology_neighbor_fanout(chunk);
    }

    pub fn record_chunk_unloaded(&mut self, chunk: ChunkPos) {
        self.chunks.remove(&chunk);
        self.light_columns.insert(chunk.into());
        self.record_topology_neighbor_fanout(chunk);
    }

    /// Records consumers of a newly calculated chunk-light halo.
    pub fn record_render_light_changed(&mut self, chunk: ChunkPos) {
        let upload = ChunkInvalidationEffects::from_bits(RENDER_LIGHT_UPLOAD);
        self.mark(chunk, upload);
        for offset in NeighborOffset::all() {
            self.mark_neighbor(chunk, offset, upload);
        }
    }

    fn record_topology_neighbor_fanout(&mut self, chunk: ChunkPos) {
        let effects = ChunkInvalidationEffects::from_bits(MESH | FLUID_STEP | RENDER_LIGHT_UPLOAD);
        for offset in NeighborOffset::all() {
            self.mark_neighbor(chunk, offset, effects);
        }
    }

    fn record_cell_light_columns(&mut self, chunk: ChunkPos) {
        self.light_columns.insert(chunk.into());
        for offset in NeighborOffset::all().filter(|offset| offset.as_ivec3().y == 0) {
            self.light_columns
                .insert(chunk.offset(offset.as_ivec3()).into());
        }
    }

    fn mark_neighbor(
        &mut self,
        chunk: ChunkPos,
        offset: NeighborOffset,
        effects: ChunkInvalidationEffects,
    ) {
        self.mark(chunk.offset(offset.as_ivec3()), effects);
    }

    fn mark(&mut self, chunk: ChunkPos, effects: ChunkInvalidationEffects) {
        if effects.is_empty() {
            return;
        }
        self.chunks
            .entry(chunk)
            .or_default()
            .effects
            .insert(effects);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::block::BlockType;
    use crate::world::chunk::state::{ChunkCell, FluidProfile};

    fn delta(old: ChunkCell, new: ChunkCell) -> CellDelta {
        CellDelta { old, new }
    }

    fn effects(plan: &ChunkInvalidationPlan, chunk: ChunkPos) -> ChunkInvalidationEffects {
        plan.effects_for(chunk)
            .unwrap_or(ChunkInvalidationEffects::NONE)
    }

    fn neighbor(origin: ChunkPos, offset: bevy::math::IVec3) -> ChunkPos {
        origin.offset(NeighborOffset::try_new(offset).unwrap().as_ivec3())
    }

    #[test]
    fn cell_classifier_uses_subsystem_specific_signatures() {
        let air = ChunkCell::EMPTY;
        let stone = ChunkCell::block(BlockType::Stone);
        let dirt = ChunkCell::block(BlockType::Dirt);
        let glass = ChunkCell::block(BlockType::Glass);
        let source = ChunkCell::fluid(FluidProfile::WATER.source());
        let falling = ChunkCell::fluid(FluidProfile::WATER.falling());

        assert!(classify_cell_delta(delta(air, air)).is_empty());

        let solid_to_solid = classify_cell_delta(delta(stone, dirt));
        assert!(solid_to_solid.needs_save());
        assert!(solid_to_solid.needs_mesh_rebuild());
        assert!(!solid_to_solid.needs_collider_rebuild());
        assert!(!solid_to_solid.needs_light_rebuild());
        assert!(!solid_to_solid.needs_fluid_step());

        let source_to_falling = classify_cell_delta(delta(source, falling));
        assert!(source_to_falling.needs_save());
        assert!(!source_to_falling.needs_mesh_rebuild());
        assert!(!source_to_falling.needs_collider_rebuild());
        assert!(!source_to_falling.needs_light_rebuild());
        assert!(source_to_falling.needs_fluid_step());

        let solid_to_transparent = classify_cell_delta(delta(stone, glass));
        assert!(solid_to_transparent.needs_save());
        assert!(solid_to_transparent.needs_mesh_rebuild());
        assert!(!solid_to_transparent.needs_collider_rebuild());
        assert!(solid_to_transparent.needs_light_rebuild());
        assert!(!solid_to_transparent.needs_fluid_step());

        let placed_solid = classify_cell_delta(delta(air, stone));
        assert!(placed_solid.needs_save());
        assert!(placed_solid.needs_mesh_rebuild());
        assert!(placed_solid.needs_collider_rebuild());
        assert!(placed_solid.needs_light_rebuild());
        assert!(placed_solid.needs_fluid_step());
    }

    #[test]
    fn cell_fanout_matches_interior_face_edge_and_corner_topology() {
        let origin = ChunkPos::new(4, 5, 6);
        let change = delta(ChunkCell::EMPTY, BlockType::Stone.into());

        let cases = [
            (LocalBlockPos::new(1, 2, 3), vec![]),
            (LocalBlockPos::new(0, 2, 3), vec![bevy::math::IVec3::NEG_X]),
            (
                LocalBlockPos::new(0, 0, 3),
                vec![
                    bevy::math::IVec3::NEG_X,
                    bevy::math::IVec3::NEG_Y,
                    bevy::math::IVec3::new(-1, -1, 0),
                ],
            ),
            (
                LocalBlockPos::ZERO,
                vec![
                    bevy::math::IVec3::NEG_X,
                    bevy::math::IVec3::NEG_Y,
                    bevy::math::IVec3::NEG_Z,
                    bevy::math::IVec3::new(-1, -1, 0),
                    bevy::math::IVec3::new(-1, 0, -1),
                    bevy::math::IVec3::new(0, -1, -1),
                    bevy::math::IVec3::NEG_ONE,
                ],
            ),
            (
                LocalBlockPos::MAX,
                vec![
                    bevy::math::IVec3::X,
                    bevy::math::IVec3::Y,
                    bevy::math::IVec3::Z,
                    bevy::math::IVec3::new(1, 1, 0),
                    bevy::math::IVec3::new(1, 0, 1),
                    bevy::math::IVec3::new(0, 1, 1),
                    bevy::math::IVec3::ONE,
                ],
            ),
        ];

        for (local, neighbor_offsets) in cases {
            let mut plan = ChunkInvalidationPlan::new();
            plan.record_cell_delta(origin, local, change);
            let expected = std::iter::once(origin)
                .chain(
                    neighbor_offsets
                        .into_iter()
                        .map(|offset| origin.offset(offset)),
                )
                .collect::<HashSet<_>>();
            assert_eq!(
                plan.chunks()
                    .map(|(chunk, _)| chunk)
                    .collect::<HashSet<_>>(),
                expected
            );
        }
    }

    #[test]
    fn boundary_neighbors_receive_only_halo_dependent_cell_work() {
        let origin = ChunkPos::new(4, 5, 6);
        let mut plan = ChunkInvalidationPlan::new();
        plan.record_cell_delta(
            origin,
            LocalBlockPos::ZERO,
            delta(ChunkCell::EMPTY, BlockType::Stone.into()),
        );

        let own = effects(&plan, origin);
        assert!(own.needs_save());
        assert!(own.needs_collider_rebuild());
        assert!(own.needs_light_rebuild());

        let diagonal = effects(&plan, neighbor(origin, bevy::math::IVec3::NEG_ONE));
        assert!(diagonal.needs_mesh_rebuild());
        assert!(diagonal.needs_fluid_step());
        assert!(!diagonal.needs_save());
        assert!(!diagonal.needs_collider_rebuild());
        assert!(!diagonal.needs_light_rebuild());
        assert!(!diagonal.needs_render_light_upload());
    }

    #[test]
    fn mesh_and_fluid_halos_are_classified_independently() {
        let origin = ChunkPos::ZERO;
        let boundary = LocalBlockPos::ZERO;

        let mut mesh_plan = ChunkInvalidationPlan::new();
        mesh_plan.record_cell_delta(
            origin,
            boundary,
            delta(BlockType::Stone.into(), BlockType::Dirt.into()),
        );
        let mesh_neighbor = effects(&mesh_plan, neighbor(origin, bevy::math::IVec3::NEG_X));
        assert!(mesh_neighbor.needs_mesh_rebuild());
        assert!(!mesh_neighbor.needs_fluid_step());

        let mut fluid_plan = ChunkInvalidationPlan::new();
        fluid_plan.record_cell_delta(
            origin,
            boundary,
            delta(
                ChunkCell::fluid(FluidProfile::WATER.source()),
                ChunkCell::fluid(FluidProfile::WATER.falling()),
            ),
        );
        let fluid_neighbor = effects(&fluid_plan, neighbor(origin, bevy::math::IVec3::NEG_X));
        assert!(!fluid_neighbor.needs_mesh_rebuild());
        assert!(fluid_neighbor.needs_fluid_step());
    }

    #[test]
    fn chunk_load_initializes_self_and_all_topology_neighbors() {
        let origin = ChunkPos::new(-2, 3, 7);
        let mut plan = ChunkInvalidationPlan::new();
        plan.record_chunk_loaded(
            origin,
            ChunkContentCounts {
                rendered: 1,
                solid: 1,
                fluids: 1,
                ..Default::default()
            },
        );

        assert_eq!(plan.chunk_count(), 27);
        assert_eq!(plan.light_column_count(), 1);
        assert!(plan.light_column_is_dirty(origin.into()));

        let own = effects(&plan, origin);
        assert!(own.needs_mesh_rebuild());
        assert!(own.needs_collider_rebuild());
        assert!(own.needs_light_rebuild());
        assert!(own.needs_fluid_step());
        assert!(!own.needs_save());
        assert!(!own.needs_render_light_upload());

        for offset in NeighborOffset::all() {
            let adjacent = effects(&plan, origin.offset(offset.as_ivec3()));
            assert!(adjacent.needs_mesh_rebuild());
            assert!(adjacent.needs_fluid_step());
            assert!(adjacent.needs_render_light_upload());
            assert!(!adjacent.needs_collider_rebuild());
            assert!(!adjacent.needs_light_rebuild());
            assert!(!adjacent.needs_save());
        }
    }

    #[test]
    fn empty_chunk_load_skips_content_dependent_self_work() {
        let origin = ChunkPos::ZERO;
        let mut plan = ChunkInvalidationPlan::new();
        plan.record_chunk_loaded(origin, ChunkContentCounts::default());

        let own = effects(&plan, origin);
        assert!(!own.needs_mesh_rebuild());
        assert!(!own.needs_collider_rebuild());
        assert!(!own.needs_fluid_step());
        assert!(own.needs_light_rebuild());
    }

    #[test]
    fn chunk_unload_discards_self_work_and_invalidates_all_neighbors() {
        let origin = ChunkPos::new(1, 2, 3);
        let mut plan = ChunkInvalidationPlan::new();
        plan.record_cell_delta(
            origin,
            LocalBlockPos::new(1, 1, 1),
            delta(ChunkCell::EMPTY, BlockType::Stone.into()),
        );
        plan.record_chunk_unloaded(origin);

        assert_eq!(plan.effects_for(origin), None);
        assert_eq!(plan.chunk_count(), 26);
        assert!(plan.light_column_is_dirty(origin.into()));
        for offset in NeighborOffset::all() {
            let adjacent = effects(&plan, origin.offset(offset.as_ivec3()));
            assert!(adjacent.needs_mesh_rebuild());
            assert!(adjacent.needs_fluid_step());
            assert!(adjacent.needs_render_light_upload());
        }
    }

    #[test]
    fn render_light_change_invalidates_self_and_all_halo_consumers() {
        let origin = ChunkPos::new(8, -1, 4);
        let mut plan = ChunkInvalidationPlan::new();
        plan.record_render_light_changed(origin);

        assert_eq!(plan.chunk_count(), 27);
        for (_, effects) in plan.chunks() {
            assert!(effects.needs_render_light_upload());
            assert!(!effects.needs_mesh_rebuild());
            assert!(!effects.needs_light_rebuild());
        }
    }

    #[test]
    fn cell_light_changes_dirty_exact_three_by_three_column_region() {
        let lower = ChunkPos::new(2, -3, 5);
        let upper = ChunkPos::new(2, 9, 5);
        let mut plan = ChunkInvalidationPlan::new();
        let change = delta(ChunkCell::EMPTY, BlockType::Stone.into());

        plan.record_cell_delta(lower, LocalBlockPos::new(1, 1, 1), change);
        plan.record_cell_delta(lower, LocalBlockPos::new(1, 1, 1), change);
        plan.record_cell_delta(upper, LocalBlockPos::new(1, 1, 1), change);

        assert_eq!(plan.chunk_count(), 2);
        assert_eq!(plan.light_column_count(), 9);
        assert_eq!(
            plan.light_columns().collect::<HashSet<_>>(),
            (-1..=1)
                .flat_map(|x| {
                    (-1..=1).map(move |z| ChunkColumn::new(lower.as_ivec3().x + x, 5 + z))
                })
                .collect::<HashSet<_>>()
        );

        plan.clear();
        assert!(plan.is_empty());
    }
}
