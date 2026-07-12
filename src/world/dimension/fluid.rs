use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

use crate::world::{
    ChunkSimulationSet,
    chunk::{
        CHUNK_VOLUME, Chunk, ChunkCell, ChunkContentCounts, ChunkEditor, ChunkInvalidationPlan,
        ChunkNeedsFluidStep, FluidProfile, FluidSnapshot, WorldBlockPos, chunk_neighbor_offsets,
        simulate_fluid_step,
    },
};

use super::{Active, Dimension, apply_chunk_invalidations};

pub(crate) struct DimensionFluidPlugin;

#[derive(Resource, Debug, Clone, Copy)]
pub(crate) struct FluidStepBudget(pub usize);

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

type FluidChunkRead = (
    &'static Chunk,
    &'static ChunkContentCounts,
    Option<&'static ChunkNeedsFluidStep>,
);
type FluidChunkWrite = (&'static mut Chunk, &'static mut ChunkContentCounts);

impl Plugin for DimensionFluidPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FluidStepBudget>()
            .init_resource::<FluidTickCounter>()
            .add_systems(
                FixedUpdate,
                (tick_counter, step_chunk_fluids)
                    .chain()
                    .in_set(ChunkSimulationSet::FluidStep),
            );
    }
}

fn tick_counter(mut counter: ResMut<FluidTickCounter>) {
    counter.0 = counter.0.wrapping_add(1);
}

fn step_chunk_fluids(
    mut commands: Commands,
    budget: Res<FluidStepBudget>,
    counter: Res<FluidTickCounter>,
    dimension: Single<&Dimension, With<Active>>,
    mut param_set: ParamSet<(Query<FluidChunkRead>, Query<FluidChunkWrite>)>,
) {
    // Water in Minecraft spreads at 1 block per 5 ticks (4/sec).
    if !counter.0.is_multiple_of(5) {
        return;
    }

    if budget.0 == 0 {
        return;
    }

    let dimension = dimension.into_inner();
    let chunks_by_pos = dimension
        .iter_published_chunks()
        .map(|(position, entity)| (position.as_ivec3(), entity))
        .collect::<HashMap<_, _>>();
    let mut active_source_chunks = Vec::new();

    {
        let chunks = param_set.p0();
        for (&position, &entity) in &chunks_by_pos {
            let Ok((_, counts, active)) = chunks.get(entity) else {
                continue;
            };
            if active.is_some() {
                if counts.fluids > 0 {
                    active_source_chunks.push(position);
                } else {
                    commands.entity(entity).remove::<ChunkNeedsFluidStep>();
                }
            }
        }
    }

    if active_source_chunks.is_empty() {
        return;
    }

    active_source_chunks.sort_by_key(|pos| (pos.x, pos.y, pos.z));
    let source_chunks = active_source_chunks
        .into_iter()
        .take(budget.0)
        .collect::<Vec<_>>();
    let source_chunks = expand_with_fluid_neighbors(source_chunks, &chunks_by_pos, &param_set.p0());
    let processed_entities = source_chunks
        .iter()
        .filter_map(|pos| chunks_by_pos.get(pos).copied())
        .collect::<HashSet<_>>();

    let snapshot_chunks =
        snapshot_chunks_for_sources(&source_chunks, &chunks_by_pos, &param_set.p0());

    let snapshot = FluidSnapshot::new(snapshot_chunks);
    let step = simulate_fluid_step(&snapshot, &source_chunks, FluidProfile::WATER);
    if step.is_empty() {
        for entity in processed_entities {
            commands.entity(entity).remove::<ChunkNeedsFluidStep>();
        }
        return;
    }

    let mut invalidations = ChunkInvalidationPlan::new();
    for update in step.updates {
        let address = WorldBlockPos::from_ivec3(update.pos).split();
        let Some(entity) = chunks_by_pos.get(&address.chunk().as_ivec3()).copied() else {
            continue;
        };

        let mut chunks_q = param_set.p1();
        let Ok((mut chunk, mut counts)) = chunks_q.get_mut(entity) else {
            continue;
        };
        let mut editor =
            ChunkEditor::new(address.chunk(), &mut chunk, &mut counts, &mut invalidations);
        editor.set_cell(address.local(), update.cell);
    }

    for entity in processed_entities {
        commands.entity(entity).remove::<ChunkNeedsFluidStep>();
    }

    apply_chunk_invalidations(&mut commands, dimension, &invalidations);
}

fn expand_with_fluid_neighbors(
    active_source_chunks: Vec<IVec3>,
    chunks_by_pos: &HashMap<IVec3, Entity>,
    chunks_q: &Query<FluidChunkRead>,
) -> Vec<IVec3> {
    let mut selected = active_source_chunks.clone();
    let mut seen = selected.iter().copied().collect::<HashSet<_>>();
    for chunk in active_source_chunks {
        for offset in chunk_neighbor_offsets() {
            let neighbor = chunk + offset;
            let Some(entity) = chunks_by_pos.get(&neighbor).copied() else {
                continue;
            };
            let Ok((_, counts, _)) = chunks_q.get(entity) else {
                continue;
            };
            if counts.fluids > 0 && seen.insert(neighbor) {
                selected.push(neighbor);
            }
        }
    }
    selected.sort_by_key(|pos| (pos.x, pos.y, pos.z));
    selected
}

fn snapshot_chunks_for_sources(
    source_chunks: &[IVec3],
    chunks_by_pos: &HashMap<IVec3, Entity>,
    chunks_q: &Query<FluidChunkRead>,
) -> HashMap<IVec3, Box<[ChunkCell; CHUNK_VOLUME]>> {
    let mut snapshot_positions = HashSet::new();
    for chunk in source_chunks {
        snapshot_positions.insert(*chunk);
        for offset in chunk_neighbor_offsets() {
            snapshot_positions.insert(*chunk + offset);
        }
    }

    snapshot_positions
        .into_iter()
        .filter_map(|pos| {
            let entity = chunks_by_pos.get(&pos).copied()?;
            let (chunk, _, _) = chunks_q.get(entity).ok()?;
            Some((pos, Box::new(chunk.to_cell_buffer())))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockType;
    use crate::world::chunk::{
        CHUNK_SIZE, ChunkCell, ChunkNeedsMeshRebuild, ChunkNeedsSave, ChunkPosition,
    };

    #[derive(Resource)]
    struct TestDimension(Entity);

    fn add_test_dimension(app: &mut App) {
        let entity = app.world_mut().spawn((Dimension::default(), Active)).id();
        app.insert_resource(TestDimension(entity));
    }

    fn register_chunk(app: &mut App, position: IVec3, entity: Entity) {
        let dimension = app.world().resource::<TestDimension>().0;
        app.world_mut()
            .entity_mut(dimension)
            .get_mut::<Dimension>()
            .unwrap()
            .register_published_chunk(position.into(), entity);
    }

    #[test]
    fn fluid_step_marks_changed_chunks_dirty_and_updates_counts() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(FluidStepBudget(1))
            .insert_resource(FluidTickCounter(4))
            .add_systems(Update, tick_counter)
            .add_systems(Update, step_chunk_fluids.after(tick_counter));
        add_test_dimension(&mut app);

        let mut chunk = Chunk::default();
        chunk.set_cell(uvec3(8, 1, 8), ChunkCell::water_source());
        chunk.set_block(uvec3(8, 0, 8), BlockType::Stone);
        let counts = chunk.compute_content_counts();
        let entity = app
            .world_mut()
            .spawn((
                ChunkPosition::from(IVec3::ZERO),
                chunk,
                counts,
                ChunkNeedsFluidStep,
            ))
            .id();
        register_chunk(&mut app, IVec3::ZERO, entity);

        app.update();

        let world = app.world();
        assert!(world.get::<ChunkNeedsSave>(entity).is_some());
        assert!(world.get::<ChunkNeedsMeshRebuild>(entity).is_some());
        let chunk = world.get::<Chunk>(entity).unwrap();
        let counts = *world.get::<ChunkContentCounts>(entity).unwrap();
        assert_eq!(counts, chunk.compute_content_counts());
        assert_eq!(counts.rendered, 6);
        assert_eq!(counts.solid, 1);
        assert_eq!(counts.fluids, 5);
    }

    #[test]
    fn water_boundary_scene_stops_marking_meshes_dirty_after_settling() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(FluidStepBudget(16))
            .insert_resource(FluidTickCounter(0))
            .add_systems(Update, step_chunk_fluids);
        add_test_dimension(&mut app);

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
        add_test_dimension(&mut app);

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
        add_test_dimension(&mut app);

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
        add_test_dimension(&mut app);

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
        let counts = chunk.compute_content_counts();
        let has_active_fluids = water.is_some();
        let entity = app
            .world_mut()
            .spawn((ChunkPosition::from(pos), chunk, counts))
            .id();
        if has_active_fluids {
            app.world_mut()
                .entity_mut(entity)
                .insert(ChunkNeedsFluidStep);
        }
        register_chunk(app, pos, entity);
        entity
    }

    fn set_cell(app: &mut App, chunk_pos: IVec3, cell_pos: UVec3, cell: ChunkCell) {
        let world = app.world_mut();
        let entity = {
            let mut query =
                world.query::<(Entity, &ChunkPosition, &mut Chunk, &mut ChunkContentCounts)>();
            let (entity, _, mut chunk, mut counts) = query
                .iter_mut(world)
                .find(|(_, pos, _, _)| pos.as_ivec3() == chunk_pos)
                .expect("chunk should exist");
            chunk.set_cell(cell_pos, cell);
            *counts = chunk.compute_content_counts();
            entity
        };
        world.entity_mut(entity).insert(ChunkNeedsFluidStep);
    }

    fn get_cell(app: &mut App, chunk_pos: IVec3, cell_pos: UVec3) -> ChunkCell {
        let world = app.world_mut();
        let mut query = world.query::<(&ChunkPosition, &Chunk)>();
        query
            .iter(world)
            .find_map(|(pos, chunk)| {
                (pos.as_ivec3() == chunk_pos).then(|| chunk.get_cell(cell_pos))
            })
            .expect("chunk should exist")
    }

    fn mark_chunks_with_fluids_active(app: &mut App) {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &ChunkContentCounts)>();
        let entities: Vec<Entity> = query
            .iter(world)
            .filter_map(|(entity, counts)| (counts.fluids > 0).then_some(entity))
            .collect();
        for entity in entities {
            world.entity_mut(entity).insert(ChunkNeedsFluidStep);
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
        query.iter(world).map(|pos| pos.as_ivec3()).collect()
    }

    fn active_fluid_chunk_positions(world: &mut World) -> Vec<IVec3> {
        let mut query = world.query_filtered::<&ChunkPosition, With<ChunkNeedsFluidStep>>();
        query.iter(world).map(|pos| pos.as_ivec3()).collect()
    }
}
