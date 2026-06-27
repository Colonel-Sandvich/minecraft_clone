use bevy::{platform::collections::HashMap, prelude::*};

use super::{
    CHUNK_SIZE, CHUNK_VOLUME, Chunk, ChunkBlockCounts, ChunkCell, ChunkHasActiveFluids,
    ChunkNeedsMeshRebuild, ChunkNeedsSave, ChunkPosition, FluidProfile, FluidState,
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
    mut param_set: ParamSet<(
        Query<
            (Entity, &ChunkPosition, &mut Chunk, &mut ChunkBlockCounts),
            With<ChunkHasActiveFluids>,
        >,
        Query<(Entity, &ChunkPosition, &mut Chunk, &mut ChunkBlockCounts)>,
    )>,
) {
    // Water in Minecraft spreads at 1 block per 5 ticks (4/sec).
    if counter.0 % 5 != 0 {
        return;
    }

    // Collect boundary flows so we can write them in a second pass
    // without borrow conflicts.
    struct BoundaryFlow {
        target_pos: IVec3,
        x: usize,
        y: usize,
        z: usize,
        fluid: FluidState,
    }
    let mut boundary_flows: Vec<BoundaryFlow> = Vec::new();
    let mut old_cells_by_entity: HashMap<Entity, Box<[ChunkCell; CHUNK_VOLUME]>> = HashMap::new();

    let mut stepped = 0;
    for (entity, pos, mut chunk, _) in &mut param_set.p0() {
        if stepped >= budget.0 {
            break;
        }
        stepped += 1;

        old_cells_by_entity.insert(entity, Box::new(chunk.to_cell_buffer()));
        let profile = FluidProfile::WATER;
        let result = chunk.step_fluids(&profile);

        if result.boundary_changed {
            // Collect cross-chunk boundary flows for this chunk
            let cp = pos.0;
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    for x in 0..CHUNK_SIZE {
                        let Some(fluid) = chunk.cell_xyz(x, y, z).as_fluid() else {
                            continue;
                        };
                        if fluid.ty() != profile.ty {
                            continue;
                        }

                        // Horizontal boundary: water at x=0 → flow to -X neighbor
                        if x == 0 {
                            if let Some(next_fluid) = profile.decayed_flow(fluid) {
                                boundary_flows.push(BoundaryFlow {
                                    target_pos: cp + IVec3::NEG_X,
                                    x: CHUNK_SIZE - 1,
                                    y,
                                    z,
                                    fluid: next_fluid,
                                });
                            }
                        }
                        if x == CHUNK_SIZE - 1 {
                            if let Some(next_fluid) = profile.decayed_flow(fluid) {
                                boundary_flows.push(BoundaryFlow {
                                    target_pos: cp + IVec3::X,
                                    x: 0,
                                    y,
                                    z,
                                    fluid: next_fluid,
                                });
                            }
                        }
                        // Horizontal boundary: water at z=0 → flow to -Z neighbor
                        if z == 0 {
                            if let Some(next_fluid) = profile.decayed_flow(fluid) {
                                boundary_flows.push(BoundaryFlow {
                                    target_pos: cp + IVec3::NEG_Z,
                                    x,
                                    y,
                                    z: CHUNK_SIZE - 1,
                                    fluid: next_fluid,
                                });
                            }
                        }
                        if z == CHUNK_SIZE - 1 {
                            if let Some(next_fluid) = profile.decayed_flow(fluid) {
                                boundary_flows.push(BoundaryFlow {
                                    target_pos: cp + IVec3::Z,
                                    x,
                                    y,
                                    z: 0,
                                    fluid: next_fluid,
                                });
                            }
                        }
                        // Vertical boundary: water at y=0 → flow down to -Y neighbor
                        if y == 0 {
                            boundary_flows.push(BoundaryFlow {
                                target_pos: cp + IVec3::NEG_Y,
                                x,
                                y: CHUNK_SIZE - 1,
                                z,
                                fluid: profile.falling(),
                            });
                        }
                    }
                }
            }
        }
    }

    let chunks_by_pos = {
        let mut chunks_by_pos = HashMap::with_capacity(param_set.p1().iter().len());
        chunks_by_pos.extend(
            param_set
                .p1()
                .iter()
                .map(|(entity, pos, _, _)| (pos.0, entity)),
        );
        chunks_by_pos
    };

    // Second pass: write boundary flows into neighbor chunks
    for flow in boundary_flows {
        let Some(entity) = chunks_by_pos.get(&flow.target_pos).copied() else {
            continue;
        };

        let mut chunks_q = param_set.p1();
        let Ok((entity, _, mut chunk, _)) = chunks_q.get_mut(entity) else {
            continue;
        };

        let cell = chunk.cell_xyz(flow.x, flow.y, flow.z);
        if !cell.is_block()
            && cell
                .as_fluid()
                .is_none_or(|f| !f.is_source() && flow.fluid.level() > f.level())
        {
            old_cells_by_entity
                .entry(entity)
                .or_insert_with(|| Box::new(chunk.to_cell_buffer()));
            chunk.set_cell_xyz(flow.x, flow.y, flow.z, ChunkCell::fluid(flow.fluid));
        }
    }

    for (entity, old_cells) in old_cells_by_entity {
        let mut chunks_q = param_set.p1();
        let Ok((entity, _, chunk, mut counts)) = chunks_q.get_mut(entity) else {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockType;
    use crate::world::chunk::ChunkCell;

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
