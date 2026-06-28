use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use super::{
    CHUNK_VOLUME, Chunk, ChunkBlockCounts, ChunkCell, ChunkHasActiveFluids, ChunkNeedsMeshRebuild,
    ChunkNeedsSave, ChunkPosition, FluidProfile, chunk_neighbor_offsets,
    fluid_sim::{FluidSnapshot, simulate_fluid_step, world_to_chunk_local},
};

pub(crate) struct ChunkFluidPlugin;

#[derive(Resource, Debug, Clone, Copy)]
pub struct FluidStepBudget(pub usize);

impl Default for FluidStepBudget {
    fn default() -> Self {
        Self(64)
    }
}

/// Counter that increments every FixedUpdate.  Fluids are only stepped
/// when counter % 5 == 0, giving 4 updates/second — matching Minecraft’s
/// 4 blocks/second water spread rate.
#[derive(Resource, Debug, Default)]
struct FluidTickCounter(u32);

#[derive(Debug, Default)]
struct FluidScanCursor(usize);

impl Plugin for ChunkFluidPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FluidStepBudget>()
            .init_resource::<FluidTickCounter>()
            .add_systems(FixedUpdate, tick_counter)
            .add_systems(FixedUpdate, step_chunk_fluids.after(tick_counter));
    }
}

fn tick_counter(mut counter: ResMut<FluidTickCounter>) {
    counter.0 = counter.0.wrapping_add(1);
}

fn step_chunk_fluids(
    mut commands: Commands,
    budget: Res<FluidStepBudget>,
    counter: Res<FluidTickCounter>,
    mut scan_cursor: Local<FluidScanCursor>,
    mut param_set: ParamSet<(
        Query<(
            Entity,
            &ChunkPosition,
            &Chunk,
            Option<&ChunkHasActiveFluids>,
        )>,
        Query<(
            Entity,
            &ChunkPosition,
            &mut Chunk,
            &mut ChunkBlockCounts,
            Option<&ChunkHasActiveFluids>,
        )>,
    )>,
) {
    // Water in Minecraft spreads at 1 block per 5 ticks (4/sec).
    if counter.0 % 5 != 0 {
        return;
    }

    if budget.0 == 0 {
        return;
    }

    let mut chunks_by_pos = HashMap::new();
    let mut snapshot_chunks = HashMap::new();
    let mut active_source_chunks = Vec::new();
    let mut inactive_source_chunks = Vec::new();

    for (entity, pos, chunk, active) in &param_set.p0() {
        chunks_by_pos.insert(pos.0, entity);
        snapshot_chunks.insert(pos.0, Box::new(chunk.to_cell_buffer()));
        if active.is_some() && chunk.has_fluids() {
            active_source_chunks.push(pos.0);
        } else if chunk.has_fluids() {
            inactive_source_chunks.push(pos.0);
        } else if active.is_some() {
            commands.entity(entity).remove::<ChunkHasActiveFluids>();
        }
    }

    if active_source_chunks.is_empty() && inactive_source_chunks.is_empty() {
        scan_cursor.0 = 0;
        return;
    }

    active_source_chunks.sort_by_key(|pos| (pos.x, pos.y, pos.z));
    inactive_source_chunks.sort_by_key(|pos| (pos.x, pos.y, pos.z));
    let source_chunks = select_source_chunks(
        active_source_chunks,
        &inactive_source_chunks,
        budget.0,
        &mut scan_cursor,
    );
    if source_chunks.is_empty() {
        return;
    }
    let processed_entities = source_chunks
        .iter()
        .filter_map(|pos| chunks_by_pos.get(pos).copied())
        .collect::<HashSet<_>>();

    let snapshot = FluidSnapshot::new(snapshot_chunks);
    let step = simulate_fluid_step(&snapshot, &source_chunks, FluidProfile::WATER);
    if step.is_empty() {
        for entity in processed_entities {
            commands.entity(entity).remove::<ChunkHasActiveFluids>();
        }
        return;
    }

    let mut old_cells_by_entity: HashMap<Entity, Box<[ChunkCell; CHUNK_VOLUME]>> = HashMap::new();
    for update in step.updates {
        let (chunk_pos, local) = world_to_chunk_local(update.pos);
        let Some(entity) = chunks_by_pos.get(&chunk_pos).copied() else {
            continue;
        };

        let mut chunks_q = param_set.p1();
        let Ok((_, _, mut chunk, _, _)) = chunks_q.get_mut(entity) else {
            continue;
        };
        old_cells_by_entity
            .entry(entity)
            .or_insert_with(|| Box::new(chunk.to_cell_buffer()));
        chunk.set_cell(local, update.cell);
    }

    let changed_entities = old_cells_by_entity.keys().copied().collect::<HashSet<_>>();
    let mut neighbor_mesh_dirty = HashSet::new();

    for (entity, old_cells) in old_cells_by_entity {
        let mut chunks_q = param_set.p1();
        let Ok((_, pos, chunk, mut counts, _)) = chunks_q.get_mut(entity) else {
            continue;
        };
        let result = chunk.fluid_step_result_from(&old_cells);
        let mut entity_commands = commands.entity(entity);
        if result.changed {
            *counts = chunk.compute_block_counts();
            entity_commands.insert((ChunkNeedsSave, ChunkNeedsMeshRebuild));
            if chunk.has_fluids() {
                entity_commands.insert(ChunkHasActiveFluids);
            } else {
                entity_commands.remove::<ChunkHasActiveFluids>();
            }
        } else {
            entity_commands.remove::<ChunkHasActiveFluids>();
        }

        if result.boundary_changed {
            for offset in chunk_neighbor_offsets() {
                let Some(entity) = chunks_by_pos.get(&(pos.0 + offset)).copied() else {
                    continue;
                };
                neighbor_mesh_dirty.insert(entity);
            }
        }
    }

    for entity in processed_entities.difference(&changed_entities) {
        commands.entity(*entity).remove::<ChunkHasActiveFluids>();
    }

    for entity in neighbor_mesh_dirty {
        commands.entity(entity).insert(ChunkNeedsMeshRebuild);
    }
}

fn select_source_chunks(
    active_source_chunks: Vec<IVec3>,
    inactive_source_chunks: &[IVec3],
    budget: usize,
    cursor: &mut FluidScanCursor,
) -> Vec<IVec3> {
    let mut selected = active_source_chunks
        .into_iter()
        .take(budget)
        .collect::<Vec<_>>();
    let remaining = budget.saturating_sub(selected.len());
    if remaining == 0 || inactive_source_chunks.is_empty() {
        return selected;
    }

    let start = cursor.0 % inactive_source_chunks.len();
    let count = remaining.min(inactive_source_chunks.len());
    selected.extend(
        (0..count)
            .map(|index| inactive_source_chunks[(start + index) % inactive_source_chunks.len()]),
    );
    cursor.0 = (start + count) % inactive_source_chunks.len();
    selected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockType;
    use crate::world::chunk::{CHUNK_SIZE, ChunkCell};

    #[test]
    fn fluid_step_marks_changed_chunks_dirty_and_updates_counts() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(FluidStepBudget(1))
            .insert_resource(FluidTickCounter(4))
            .add_systems(Update, tick_counter)
            .add_systems(Update, step_chunk_fluids.after(tick_counter));

        let mut chunk = Chunk::default();
        chunk.set_cell(uvec3(8, 1, 8), ChunkCell::water_source());
        chunk.set_block(uvec3(8, 0, 8), BlockType::Stone);
        let counts = chunk.compute_block_counts();
        let entity = app
            .world_mut()
            .spawn((
                ChunkPosition(IVec3::ZERO),
                chunk,
                counts,
                ChunkHasActiveFluids,
            ))
            .id();

        app.update();

        let world = app.world();
        assert!(world.get::<ChunkNeedsSave>(entity).is_some());
        assert!(world.get::<ChunkNeedsMeshRebuild>(entity).is_some());
        assert_eq!(world.get::<ChunkBlockCounts>(entity).unwrap().rendered, 6);
    }

    #[test]
    fn water_boundary_scene_stops_marking_meshes_dirty_after_settling() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(FluidStepBudget(16))
            .insert_resource(FluidTickCounter(0))
            .add_systems(Update, step_chunk_fluids);

        spawn_flat_chunk(&mut app, IVec3::new(0, 0, -1), None);
        spawn_flat_chunk(
            &mut app,
            IVec3::ZERO,
            Some((uvec3(8, 1, 0), ChunkCell::water_source())),
        );
        set_cell(
            &mut app,
            IVec3::new(0, 0, -1),
            uvec3(8, 1, 14),
            ChunkCell::water_source(),
        );
        set_cell(
            &mut app,
            IVec3::ZERO,
            uvec3(8, 1, 1),
            ChunkCell::water_source(),
        );

        assert_scene_settles(&mut app, 100);
    }

    #[test]
    fn water_boundary_scene_stops_marking_meshes_dirty_after_source_removal() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(FluidStepBudget(16))
            .insert_resource(FluidTickCounter(0))
            .add_systems(Update, step_chunk_fluids);

        spawn_flat_chunk(&mut app, IVec3::new(0, 0, -1), None);
        spawn_flat_chunk(
            &mut app,
            IVec3::ZERO,
            Some((uvec3(8, 1, 0), ChunkCell::water_source())),
        );
        set_cell(
            &mut app,
            IVec3::new(0, 0, -1),
            uvec3(8, 1, 14),
            ChunkCell::water_source(),
        );
        set_cell(
            &mut app,
            IVec3::ZERO,
            uvec3(8, 1, 1),
            ChunkCell::water_source(),
        );
        assert_scene_settles(&mut app, 100);

        set_cell(
            &mut app,
            IVec3::new(0, 0, -1),
            uvec3(8, 1, 14),
            ChunkCell::EMPTY,
        );
        set_cell(&mut app, IVec3::ZERO, uvec3(8, 1, 0), ChunkCell::EMPTY);
        set_cell(&mut app, IVec3::ZERO, uvec3(8, 1, 1), ChunkCell::EMPTY);
        mark_chunks_with_fluids_active(&mut app);
        clear_mesh_dirty_markers(&mut app);

        assert_scene_settles(&mut app, 100);
    }

    #[test]
    fn flat_boundary_water_source_stops_marking_meshes_dirty_after_settling() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(FluidStepBudget(16))
            .insert_resource(FluidTickCounter(0))
            .add_systems(Update, step_chunk_fluids);

        spawn_flat_chunk(&mut app, IVec3::new(0, 1, -1), None);
        spawn_flat_chunk(
            &mut app,
            IVec3::new(0, 1, 0),
            Some((uvec3(12, 2, 2), ChunkCell::water_source())),
        );

        assert_scene_settles(&mut app, 100);
    }

    #[test]
    fn inactive_boundary_source_flows_into_later_loaded_chunk() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(FluidStepBudget(16))
            .insert_resource(FluidTickCounter(0))
            .add_systems(Update, step_chunk_fluids);

        spawn_flat_chunk(
            &mut app,
            IVec3::ZERO,
            Some((
                uvec3(CHUNK_SIZE as u32 - 1, 1, 8),
                ChunkCell::water_source(),
            )),
        );
        assert_scene_settles(&mut app, 100);
        assert!(active_fluid_chunk_positions(app.world_mut()).is_empty());

        spawn_flat_chunk(&mut app, IVec3::X, None);
        mark_chunks_with_fluids_active(&mut app);
        app.update();

        assert_eq!(
            get_cell(&mut app, IVec3::X, uvec3(0, 1, 8)),
            ChunkCell::water_flow(7)
        );
    }

    fn assert_scene_settles(app: &mut App, max_steps: usize) {
        let mut last_dirty_positions = Vec::new();
        let mut last_active_positions = Vec::new();
        for _ in 0..max_steps {
            app.update();

            let world = app.world_mut();
            last_dirty_positions = dirty_chunk_positions(world);
            last_active_positions = active_fluid_chunk_positions(world);

            if last_dirty_positions.is_empty() && last_active_positions.is_empty() {
                return;
            }

            clear_mesh_dirty_markers_for_world(world);
        }

        panic!(
            "water should settle and stop marking meshes dirty within {max_steps} steps; \
             last dirty chunks: {last_dirty_positions:?}; \
             last active chunks: {last_active_positions:?}"
        );
    }

    fn spawn_flat_chunk(app: &mut App, pos: IVec3, water: Option<(UVec3, ChunkCell)>) -> Entity {
        let mut chunk = Chunk::default();
        for x in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                chunk.set_cell_xyz(x, 0, z, BlockType::Stone.into());
            }
        }
        if let Some((pos, cell)) = water {
            chunk.set_cell(pos, cell);
        }
        let counts = chunk.compute_block_counts();
        let has_active_fluids = water.is_some();
        let entity = app
            .world_mut()
            .spawn((ChunkPosition(pos), chunk, counts))
            .id();
        if has_active_fluids {
            app.world_mut()
                .entity_mut(entity)
                .insert(ChunkHasActiveFluids);
        }
        entity
    }

    fn set_cell(app: &mut App, chunk_pos: IVec3, cell_pos: UVec3, cell: ChunkCell) {
        let world = app.world_mut();
        let mut query =
            world.query::<(Entity, &ChunkPosition, &mut Chunk, &mut ChunkBlockCounts)>();
        let (entity, _, mut chunk, mut counts) = query
            .iter_mut(world)
            .find(|(_, pos, _, _)| pos.0 == chunk_pos)
            .expect("chunk should exist");
        chunk.set_cell(cell_pos, cell);
        *counts = chunk.compute_block_counts();
        drop(chunk);
        world.entity_mut(entity).insert(ChunkHasActiveFluids);
    }

    fn get_cell(app: &mut App, chunk_pos: IVec3, cell_pos: UVec3) -> ChunkCell {
        let world = app.world_mut();
        let mut query = world.query::<(&ChunkPosition, &Chunk)>();
        query
            .iter(world)
            .find_map(|(pos, chunk)| (pos.0 == chunk_pos).then(|| chunk.get_cell(cell_pos)))
            .expect("chunk should exist")
    }

    fn mark_chunks_with_fluids_active(app: &mut App) {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &Chunk)>();
        let entities: Vec<Entity> = query
            .iter(world)
            .filter_map(|(entity, chunk)| chunk.has_fluids().then_some(entity))
            .collect();
        for entity in entities {
            world.entity_mut(entity).insert(ChunkHasActiveFluids);
        }
    }

    fn clear_mesh_dirty_markers(app: &mut App) {
        clear_mesh_dirty_markers_for_world(app.world_mut());
    }

    fn clear_mesh_dirty_markers_for_world(world: &mut World) {
        let mut query = world.query_filtered::<Entity, With<ChunkNeedsMeshRebuild>>();
        let dirty_entities: Vec<Entity> = query.iter(world).collect();
        for entity in dirty_entities {
            world.entity_mut(entity).remove::<ChunkNeedsMeshRebuild>();
        }
    }

    fn dirty_chunk_positions(world: &mut World) -> Vec<IVec3> {
        let mut query = world.query_filtered::<&ChunkPosition, With<ChunkNeedsMeshRebuild>>();
        query.iter(world).map(|pos| pos.0).collect()
    }

    fn active_fluid_chunk_positions(world: &mut World) -> Vec<IVec3> {
        let mut query = world.query_filtered::<&ChunkPosition, With<ChunkHasActiveFluids>>();
        query.iter(world).map(|pos| pos.0).collect()
    }
}
