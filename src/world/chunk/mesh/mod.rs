pub mod binary;
mod blocks;
pub mod vertex_pulling;

pub use blocks::ChunkMeshBlocks;
pub use vertex_pulling::{
    ChunkMeshDescriptors, VertexPullingLight, VertexPullingMesh, VpTextureState,
};

use std::sync::Arc;

use bevy::{camera::primitives::Aabb, platform::collections::HashMap, prelude::*, utils::Parallel};
use strum::IntoEnumIterator;

use crate::block::{
    BLOCK_FLAG_CUTOUT, BLOCK_FLAG_EMITS_INTERNAL_FACES, BLOCK_FLAG_FULL_CUBE, BLOCK_FLAG_RENDERED,
    BLOCK_FLAG_TRANSLUCENT, BlockMaterialLayer, BlockTextureLayer, BlockTextureMap, BlockType,
    RENDER_ID_COUNT, WATER_RENDER_ID, from_render_id, render_id_to_colour,
};
use crate::quad::Direction;
use crate::textures::{BlockTextures, TextureState};

use super::{
    CHUNK_ISIZE, CHUNK_SIZE, CHUNK_VOLUME, Chunk, ChunkLight, ChunkNeedsLightUpload,
    ChunkNeedsMeshRebuild, ChunkPerfCounters, ChunkPosition, ambient_occlusion::AO_BRIGHTNESS,
};

pub(crate) const PADDED_CHUNK_SIZE: usize = CHUNK_SIZE + 2;
pub(crate) const PADDED_CHUNK_VOLUME: usize =
    PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
pub(crate) const PADDED_CHUNK_LAYER_SIZE: usize = PADDED_CHUNK_SIZE * PADDED_CHUNK_SIZE;
pub(crate) const DIRECTION_COUNT: usize = 6;
pub(crate) const DIRECTION_INDEX_OFFSETS: [isize; DIRECTION_COUNT] = [
    -1,
    1,
    -(PADDED_CHUNK_LAYER_SIZE as isize),
    PADDED_CHUNK_LAYER_SIZE as isize,
    -(PADDED_CHUNK_SIZE as isize),
    PADDED_CHUNK_SIZE as isize,
];
pub(crate) const VERTEX_AO: [u32; 8] = [3, 2, 2, 0, 2, 1, 1, 0];

pub(crate) const FACE_AO_SAMPLE_COUNT: usize = 8;

#[derive(Clone, Copy)]
pub(crate) enum FaceAoOrder {
    Ab,
    Ba,
}

pub(crate) const FACE_AO_ORDERS: [FaceAoOrder; DIRECTION_COUNT] = [
    FaceAoOrder::Ab,
    FaceAoOrder::Ab,
    FaceAoOrder::Ba,
    FaceAoOrder::Ab,
    FaceAoOrder::Ba,
    FaceAoOrder::Ba,
];

pub(crate) const FACE_AO_SAMPLE_OFFSETS: [[isize; FACE_AO_SAMPLE_COUNT]; DIRECTION_COUNT] = [
    [-325, 323, 17, -19, -307, -343, 341, 305],
    [-323, 325, -17, 19, -341, -305, 307, 343],
    [-325, -323, -306, -342, -307, -343, -305, -341],
    [323, 325, 342, 306, 341, 305, 343, 307],
    [-19, -17, -342, 306, -343, 305, -341, 307],
    [19, 17, -306, 342, -305, 343, -307, 341],
];

#[inline(always)]
pub(crate) fn block_mesh_flags(rid: u16) -> u8 {
    match rid {
        0 => 0,
        WATER_RENDER_ID => BLOCK_FLAG_RENDERED | BLOCK_FLAG_TRANSLUCENT,
        _ => BlockType::from_repr(rid - 1).unwrap().mesh_flags(),
    }
}

#[inline(always)]
pub(crate) const fn material_layer_index_from_flags(flags: u8) -> usize {
    if flags & BLOCK_FLAG_TRANSLUCENT != 0 {
        BlockMaterialLayer::Translucent.index()
    } else if flags & BLOCK_FLAG_CUTOUT != 0 {
        BlockMaterialLayer::Cutout.index()
    } else {
        BlockMaterialLayer::Opaque.index()
    }
}

fn drop_uploaded_descriptors(
    mut commands: Commands,
    descriptors: Query<(Entity, Ref<ChunkMeshDescriptors>)>,
) {
    for (entity, desc_ref) in &descriptors {
        if !desc_ref.is_added() {
            commands.entity(entity).remove::<ChunkMeshDescriptors>();
        }
    }
}

pub struct ChunkMeshPlugin;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkMaterialLayerMarker(BlockMaterialLayer);

impl Plugin for ChunkMeshPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            sync_vp_texture_state.run_if(in_state(TextureState::Finished)),
        )
        .add_systems(
            FixedPreUpdate,
            (rebuild_chunk_meshes, upload_chunk_lights)
                .chain()
                .run_if(in_state(TextureState::Finished)),
        )
        .add_systems(PostUpdate, drop_uploaded_descriptors);
    }
}

fn sync_vp_texture_state(
    mut commands: Commands,
    block_textures: Res<BlockTextures>,
    block_texture_map: Res<BlockTextureMap>,
    current: Option<Res<VpTextureState>>,
) {
    if current.is_some() && !block_textures.is_changed() && !block_texture_map.is_changed() {
        return;
    }

    commands.insert_resource(build_vp_texture_state(&block_textures, &block_texture_map));
}

fn build_vp_texture_state(
    block_textures: &BlockTextures,
    block_texture_map: &BlockTextureMap,
) -> VpTextureState {
    let entry_count = RENDER_ID_COUNT * DIRECTION_COUNT;
    let mut texture_layers = Vec::with_capacity(entry_count);
    let mut tint_colors = Vec::with_capacity(entry_count);
    let mut emission_factors = Vec::with_capacity(entry_count);

    for rid in 0..RENDER_ID_COUNT as u16 {
        let emission = match rid {
            0 | WATER_RENDER_ID => 0.0,
            _ => f32::from(from_render_id(rid).unwrap().light_emission()) / 15.0,
        };

        for side in Direction::iter() {
            if rid == 0 {
                texture_layers.push(pack_texture_layer(BlockTextureLayer::default(), 1));
                tint_colors.push([0.0; 4]);
            } else {
                let animation = block_texture_map.render_id_to_texture_animation(rid, side);
                let color = render_id_to_colour(rid, side);
                texture_layers.push(pack_texture_layer(
                    animation.base_layer(),
                    animation.frame_count(),
                ));
                tint_colors.push([color.x, color.y, color.z, color.w]);
            }
            emission_factors.push(emission);
        }
    }

    VpTextureState {
        terrain_texture_handle: block_textures.terrain.clone(),
        texture_layers,
        tint_colors,
        emission_factors,
        ao_brightness: AO_BRIGHTNESS,
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn rebuild_chunk_meshes(
    mut commands: Commands,
    mut perf: Option<ResMut<ChunkPerfCounters>>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), (With<Chunk>, With<ChunkNeedsMeshRebuild>)>,
    all_chunks_q: Query<(&ChunkPosition, &Chunk)>,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    children_q: Query<&Children>,
    vp_children_q: Query<(Entity, &ChunkMaterialLayerMarker), With<VertexPullingMesh>>,
    mut vp_mesh_q: Query<&mut VertexPullingMesh>,
    vp_light_q: Query<&VertexPullingLight>,
    chunk_transform_q: Query<&Transform>,
) {
    if dirty_chunks_q.is_empty() {
        return;
    }

    let mut chunks_by_pos = HashMap::with_capacity(all_chunks_q.iter().len());
    for (positions, chunks) in all_chunks_q
        .contiguous_iter()
        .expect("chunk mesh position map query should stay dense")
    {
        chunks_by_pos.extend(
            positions
                .iter()
                .zip(chunks.iter())
                .map(|(pos, chunk)| (pos.0, chunk)),
        );
    }

    let mut lights_by_pos: HashMap<IVec3, &ChunkLight> =
        HashMap::with_capacity(light_q.iter().len());
    for (positions, lights) in light_q
        .contiguous_iter()
        .expect("chunk mesh light map query should stay dense")
    {
        lights_by_pos.extend(
            positions
                .iter()
                .zip(lights.iter())
                .map(|(pos, light)| (pos.0, light)),
        );
    }

    let mut build_queue = Parallel::<Vec<VpChunkBuild>>::default();
    dirty_chunks_q.par_iter().for_each_init(
        || build_queue.borrow_local_mut(),
        |builds, (chunk_pos, chunk_entity)| {
            let blocks = ChunkMeshBlocks::from_chunks(chunk_pos.0, &chunks_by_pos);
            let layers = binary::build_descriptors_hybrid(&blocks);
            builds.push(VpChunkBuild {
                entity: chunk_entity,
                chunk_pos: chunk_pos.0,
                layers,
            });
        },
    );
    let mut builds = Vec::new();
    build_queue.drain_into(&mut builds);
    let rebuilt_count = builds.len();
    for build in builds {
        let origin = chunk_transform_q
            .get(build.entity)
            .map(|t| t.translation)
            .unwrap_or(Vec3::ZERO);
        update_chunk_vp_children(
            &mut commands,
            &vp_children_q,
            &mut vp_mesh_q,
            build.entity,
            build.chunk_pos,
            children_q.get(build.entity).ok(),
            build.layers,
            origin,
            &lights_by_pos,
            &vp_light_q,
        );
        commands
            .entity(build.entity)
            .remove::<ChunkNeedsMeshRebuild>();
    }
    if let Some(perf) = perf.as_deref_mut() {
        perf.mesh_rebuilds += rebuilt_count;
    }
}

fn pack_texture_layer(layer: BlockTextureLayer, frame_count: u32) -> u32 {
    layer.index() | (frame_count.min(255) << 24)
}

struct VpChunkBuild {
    entity: Entity,
    chunk_pos: IVec3,
    layers: Vec<(BlockMaterialLayer, Vec<vertex_pulling::FaceDescriptor>)>,
}

#[allow(clippy::too_many_arguments)]
fn update_chunk_vp_children(
    commands: &mut Commands,
    vp_children_q: &Query<(Entity, &ChunkMaterialLayerMarker), With<VertexPullingMesh>>,
    vp_mesh_q: &mut Query<&mut VertexPullingMesh>,
    chunk_entity: Entity,
    chunk_pos: IVec3,
    children: Option<&Children>,
    layers: Vec<(BlockMaterialLayer, Vec<vertex_pulling::FaceDescriptor>)>,
    chunk_origin: Vec3,
    lights_by_pos: &HashMap<IVec3, &ChunkLight>,
    vp_light_q: &Query<&VertexPullingLight>,
) {
    let existing = children
        .map(|children| {
            vp_children_q
                .iter_many(children)
                .map(|(entity, marker)| (marker.0, entity))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    let mut updated = Vec::new();
    let mut shared_light_data = existing_child_light_data(&existing, vp_light_q);
    for (layer, descriptors) in layers {
        let face_count = descriptors.len() as u32;
        updated.push(layer);

        if let Some(entity) = existing.get(&layer) {
            if let Ok(mut mesh) = vp_mesh_q.get_mut(*entity) {
                mesh.face_count = face_count;
                mesh.material_layer = layer;
                mesh.chunk_origin = chunk_origin;
            }
            commands
                .entity(*entity)
                .insert((ChunkMeshDescriptors(descriptors), chunk_render_aabb()));
            continue;
        }

        let light_data =
            light_data_for_new_vp_child(&mut shared_light_data, chunk_pos, lights_by_pos);

        commands.spawn((
            ChildOf(chunk_entity),
            ChunkMaterialLayerMarker(layer),
            Transform::default(),
            Visibility::default(),
            chunk_render_aabb(),
            VertexPullingMesh {
                face_count,
                material_layer: layer,
                chunk_origin,
            },
            ChunkMeshDescriptors(descriptors),
            VertexPullingLight {
                light_data: light_data.clone(),
            },
        ));
    }

    for (layer, entity) in existing {
        if updated.contains(&layer) {
            continue;
        }
        commands.entity(entity).despawn();
    }
}

fn existing_child_light_data(
    existing: &HashMap<BlockMaterialLayer, Entity>,
    vp_light_q: &Query<&VertexPullingLight>,
) -> Option<Arc<[u32]>> {
    existing.values().find_map(|entity| {
        vp_light_q
            .get(*entity)
            .ok()
            .map(|light| light.light_data.clone())
    })
}

fn light_data_for_new_vp_child(
    shared_light_data: &mut Option<Arc<[u32]>>,
    chunk_pos: IVec3,
    lights_by_pos: &HashMap<IVec3, &ChunkLight>,
) -> Arc<[u32]> {
    shared_light_data
        .get_or_insert_with(|| {
            Arc::from(ChunkLight::build_padded_light_data(
                chunk_pos,
                lights_by_pos,
            ))
        })
        .clone()
}

fn upload_chunk_lights(
    mut commands: Commands,
    mut perf: Option<ResMut<ChunkPerfCounters>>,
    dirty_chunks_q: Query<(&ChunkPosition, Entity), With<ChunkNeedsLightUpload>>,
    light_q: Query<(&ChunkPosition, &ChunkLight)>,
    children_q: Query<&Children>,
    mut vp_light_q: Query<&mut VertexPullingLight>,
) {
    if dirty_chunks_q.is_empty() {
        return;
    }
    let upload_count = dirty_chunks_q.iter().len();

    let mut lights_by_pos: HashMap<IVec3, &ChunkLight> =
        HashMap::with_capacity(light_q.iter().len());
    for (positions, lights) in light_q
        .contiguous_iter()
        .expect("chunk light upload map query should stay dense")
    {
        lights_by_pos.extend(
            positions
                .iter()
                .zip(lights.iter())
                .map(|(pos, light)| (pos.0, light)),
        );
    }

    for (chunk_pos, chunk_entity) in &dirty_chunks_q {
        let light_data: Arc<[u32]> =
            ChunkLight::build_padded_light_data(chunk_pos.0, &lights_by_pos).into();
        if let Ok(children) = children_q.get(chunk_entity) {
            for child in children {
                if let Ok(mut light) = vp_light_q.get_mut(*child) {
                    light.light_data = light_data.clone();
                }
            }
        }

        commands
            .entity(chunk_entity)
            .remove::<ChunkNeedsLightUpload>();
    }
    if let Some(perf) = perf.as_deref_mut() {
        perf.light_uploads += upload_count;
    }
}

const fn chunk_render_aabb() -> Aabb {
    let half = CHUNK_SIZE as f32 / 2.0;
    let v = vec3a(half, half, half);
    Aabb {
        center: v,
        half_extents: v,
    }
}

#[inline(always)]
pub(crate) fn should_emit_face_from_flags(
    cell: u16,
    block_flags: u8,
    neighbor: u16,
    neighbor_flags: u8,
) -> bool {
    if neighbor_flags & BLOCK_FLAG_RENDERED == 0 {
        return true;
    }

    if neighbor_flags & BLOCK_FLAG_FULL_CUBE != 0 && block_flags & BLOCK_FLAG_TRANSLUCENT == 0 {
        return false;
    }

    if cell == neighbor && block_flags & BLOCK_FLAG_FULL_CUBE == 0 {
        return block_flags & BLOCK_FLAG_EMITS_INTERNAL_FACES != 0;
    }

    true
}

/// Translucent-specific face culling (water, ice).
///
/// Same rules as the scalar pass except translucent blocks are culled by
/// full-cube face occluders (stone, dirt, grass, etc.). Same-fluid-type
/// faces are also culled.
#[inline(always)]
pub(crate) fn should_emit_translucent_face(
    cell: u16,
    _block_flags: u8,
    neighbor: u16,
    neighbor_flags: u8,
) -> bool {
    if neighbor_flags & BLOCK_FLAG_RENDERED == 0 {
        return true;
    }

    if neighbor_flags & BLOCK_FLAG_FULL_CUBE != 0 {
        return false;
    }

    if cell == neighbor {
        return false;
    }

    true
}

/// Compute the 4 corner water heights for a water cell's top surface.
///
/// Heights are packed in vanilla-style ninths: flowing/source water owns up to
/// 8/9 of a block, while any water directly above forces the column to 9/9.
/// Corners use the vanilla weighted average: heights >= 0.8 are weighted x10,
/// air contributes 0, and solid/non-water cells are ignored.
///
/// Returns (h00, h10, h01, h11) where:
///   h00 = corner at (x+0, z+0)  h10 = corner at (x+1, z+0)
///   h01 = corner at (x+0, z+1)  h11 = corner at (x+1, z+1)
pub(crate) fn water_corner_heights(
    self_level: u8,
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
) -> (u32, u32, u32, u32) {
    let base = padded_index as isize;
    let pcs = PADDED_CHUNK_SIZE as isize;
    let n1 = base - 1;
    let p1 = base + 1;
    let nz = base - pcs;
    let pz = base + pcs;
    let self_height = water_height_at(blocks, base).unwrap_or_else(|| self_level.min(8) as i32);
    if self_height >= 9 {
        return (9, 9, 9, 9);
    }
    let h00 = water_corner_height(self_height, blocks, n1, nz, n1 - pcs);
    let h10 = water_corner_height(self_height, blocks, p1, nz, p1 - pcs);
    let h01 = water_corner_height(self_height, blocks, n1, pz, n1 + pcs);
    let h11 = water_corner_height(self_height, blocks, p1, pz, p1 + pcs);
    (h00, h10, h01, h11)
}

fn water_corner_height(
    self_height: i32,
    blocks: &ChunkMeshBlocks,
    adjacent_a: isize,
    adjacent_b: isize,
    diagonal: isize,
) -> u32 {
    let adjacent_a = water_height_at(blocks, adjacent_a);
    let adjacent_b = water_height_at(blocks, adjacent_b);

    if adjacent_a == Some(9) || adjacent_b == Some(9) {
        return 9;
    }

    let mut weighted = WeightedWaterHeight::default();
    if adjacent_a.is_some_and(|height| height > 0) || adjacent_b.is_some_and(|height| height > 0) {
        let diagonal = water_height_at(blocks, diagonal);
        if diagonal == Some(9) {
            return 9;
        }
        weighted.add(diagonal);
    }

    weighted.add(Some(self_height));
    weighted.add(adjacent_a);
    weighted.add(adjacent_b);
    weighted.average()
}

fn water_height_at(blocks: &ChunkMeshBlocks, padded_index: isize) -> Option<i32> {
    let index = padded_index as usize;
    let cell = unsafe { *blocks.blocks.get_unchecked(index) };
    if cell == WATER_RENDER_ID {
        let above_index = (padded_index + DIRECTION_INDEX_OFFSETS[3]) as usize;
        let above = unsafe { *blocks.blocks.get_unchecked(above_index) };
        if above == WATER_RENDER_ID {
            return Some(9);
        }
        return Some(blocks.get_fluid_level(index).min(8) as i32);
    }
    (cell == 0).then_some(0)
}

#[derive(Default)]
struct WeightedWaterHeight {
    total: i32,
    weight: i32,
}

impl WeightedWaterHeight {
    fn add(&mut self, height: Option<i32>) {
        let Some(height) = height else { return };
        let weight = if height >= 8 { 10 } else { 1 };
        self.total += height * weight;
        self.weight += weight;
    }

    fn average(self) -> u32 {
        if self.weight == 0 {
            return 0;
        }
        ((self.total + self.weight / 2) / self.weight).clamp(0, 9) as u32
    }
}

/// Return the (lo, hi) lower-water corner height pair for a given side-index
/// bottom vertex.  lo = corner at qi=0 in quad space, hi = corner at qi=1.
/// For the y-facing indices (down=2, up=3) both values are zero.
#[inline(always)]
pub(crate) fn water_below_pair(
    side_index: usize,
    bh00: u32,
    bh10: u32,
    bh01: u32,
    bh11: u32,
) -> (u32, u32) {
    match side_index {
        0 => (bh00, bh01), // -X / Left
        1 => (bh10, bh11), // +X / Right
        4 => (bh00, bh10), // -Z / Back
        5 => (bh01, bh11), // +Z / Forward
        _ => (0, 0),       // DOWN, UP
    }
}

/// Quantized horizontal water-flow direction for top-face UV orientation.
///
/// Codes are zero for still/no horizontal flow, then clockwise in X/Z space:
/// `1=+X`, `2=+X+Z`, `3=+Z`, `4=-X+Z`, `5=-X`, `6=-X-Z`, `7=-Z`, `8=+X-Z`.
pub(crate) fn water_flow_code(
    self_level: u8,
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
) -> u32 {
    let mut dx = 0i32;
    let mut dz = 0i32;
    for (offset, vx, vz) in [
        (-1isize, -1, 0),
        (1, 1, 0),
        (-(PADDED_CHUNK_SIZE as isize), 0, -1),
        (PADDED_CHUNK_SIZE as isize, 0, 1),
    ] {
        let neighbor_index = (padded_index as isize + offset) as usize;
        let neighbor = unsafe { *blocks.blocks.get_unchecked(neighbor_index) };
        let neighbor_level = if neighbor == WATER_RENDER_ID {
            blocks.get_fluid_level(neighbor_index)
        } else if neighbor == 0 {
            0
        } else {
            continue;
        };

        let drop = self_level.saturating_sub(neighbor_level) as i32;
        if drop > 0 {
            dx += vx * drop;
            dz += vz * drop;
        }
    }

    quantized_water_flow_code(dx, dz)
}

fn quantized_water_flow_code(dx: i32, dz: i32) -> u32 {
    if dx == 0 && dz == 0 {
        return 0;
    }

    let ax = dx.abs();
    let az = dz.abs();
    let sx = dx.signum();
    let sz = dz.signum();

    if az * 2 <= ax {
        return if sx > 0 { 1 } else { 5 };
    }
    if ax * 2 <= az {
        return if sz > 0 { 3 } else { 7 };
    }

    match (sx, sz) {
        (1, 1) => 2,
        (-1, 1) => 4,
        (-1, -1) => 6,
        (1, -1) => 8,
        _ => 0,
    }
}

#[inline(always)]
pub(crate) fn face_ao_key_from_indices(
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    side_index: usize,
) -> u32 {
    let [a0, a1, b0, b1, c00, c01, c10, c11] = FACE_AO_SAMPLE_OFFSETS[side_index];

    face_ao_key_from_sample_bits(
        FACE_AO_ORDERS[side_index],
        block_occludes_ambient_light_bit(blocks, padded_index, a0),
        block_occludes_ambient_light_bit(blocks, padded_index, a1),
        block_occludes_ambient_light_bit(blocks, padded_index, b0),
        block_occludes_ambient_light_bit(blocks, padded_index, b1),
        block_occludes_ambient_light_bit(blocks, padded_index, c00),
        block_occludes_ambient_light_bit(blocks, padded_index, c01),
        block_occludes_ambient_light_bit(blocks, padded_index, c10),
        block_occludes_ambient_light_bit(blocks, padded_index, c11),
    )
}

#[inline(always)]
pub(crate) fn face_ao_key_from_sample_bits(
    order: FaceAoOrder,
    a0: u32,
    a1: u32,
    b0: u32,
    b1: u32,
    c00: u32,
    c01: u32,
    c10: u32,
    c11: u32,
) -> u32 {
    match order {
        FaceAoOrder::Ab => {
            vertex_ao_key(a0, b0, c00)
                | (vertex_ao_key(a0, b1, c01) << 2)
                | (vertex_ao_key(a1, b0, c10) << 4)
                | (vertex_ao_key(a1, b1, c11) << 6)
        }
        FaceAoOrder::Ba => {
            vertex_ao_key(a0, b0, c00)
                | (vertex_ao_key(a1, b0, c10) << 2)
                | (vertex_ao_key(a0, b1, c01) << 4)
                | (vertex_ao_key(a1, b1, c11) << 6)
        }
    }
}

#[inline(always)]
#[cfg(test)]
pub(crate) fn face_ao_from_indices(
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    side_index: usize,
) -> [u8; 4] {
    let key = face_ao_key_from_indices(blocks, padded_index, side_index);
    [
        (key & 0x3) as u8,
        ((key >> 2) & 0x3) as u8,
        ((key >> 4) & 0x3) as u8,
        ((key >> 6) & 0x3) as u8,
    ]
}

#[inline(always)]
pub(crate) fn vertex_ao_key(s1: u32, s2: u32, corner: u32) -> u32 {
    VERTEX_AO[(s1 | (s2 << 1) | (corner << 2)) as usize]
}

#[inline(always)]
fn block_occludes_ambient_light_bit(
    blocks: &ChunkMeshBlocks,
    padded_index: usize,
    offset: isize,
) -> u32 {
    let index = (padded_index as isize + offset) as usize;
    debug_assert!(index < PADDED_CHUNK_VOLUME);
    unsafe {
        ((block_mesh_flags(*blocks.blocks.get_unchecked(index)) & BLOCK_FLAG_FULL_CUBE) >> 1) as u32
    }
}

pub(crate) fn padded_chunk_index(x: usize, y: usize, z: usize) -> usize {
    x + PADDED_CHUNK_SIZE * (z + PADDED_CHUNK_SIZE * y)
}

#[cfg(test)]
mod tests;
