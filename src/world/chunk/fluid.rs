use bevy::prelude::*;

use super::{
    CHUNK_SIZE, Chunk, ChunkBlockCounts, ChunkCell, ChunkHasActiveFluids, ChunkNeedsMeshRebuild,
    ChunkNeedsSave, ChunkPosition, FluidState, FluidType,
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
        Query<(Entity, &ChunkPosition, &mut Chunk)>,
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

    let mut stepped = 0;
    for (entity, pos, mut chunk, mut counts) in &mut param_set.p0() {
        if stepped >= budget.0 {
            break;
        }
        stepped += 1;

        let result = chunk.step_fluids();
        if !result.changed {
            commands.entity(entity).remove::<ChunkHasActiveFluids>();
            continue;
        }

        *counts = chunk.compute_block_counts();
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert((ChunkNeedsSave, ChunkNeedsMeshRebuild));
        if !chunk.has_fluids() {
            entity_commands.remove::<ChunkHasActiveFluids>();
        }

        if result.boundary_changed {
            // Collect cross-chunk boundary flows for this chunk
            let cp = pos.0;
            for y in 0..CHUNK_SIZE {
                for z in 0..CHUNK_SIZE {
                    for x in 0..CHUNK_SIZE {
                        let Some(fluid) = chunk.cell_xyz(x, y, z).as_fluid() else {
                            continue;
                        };
                        if fluid.is_empty() || fluid.ty != FluidType::Water {
                            continue;
                        }

                        // Horizontal boundary: water at x=0 → flow to -X neighbor
                        if x == 0 {
                            let next_level = fluid.level.saturating_sub(1);
                            if next_level > 0 {
                                boundary_flows.push(BoundaryFlow {
                                    target_pos: cp + IVec3::NEG_X,
                                    x: CHUNK_SIZE - 1,
                                    y,
                                    z,
                                    fluid: FluidState::water_flow(next_level),
                                });
                            }
                        }
                        if x == CHUNK_SIZE - 1 {
                            let next_level = fluid.level.saturating_sub(1);
                            if next_level > 0 {
                                boundary_flows.push(BoundaryFlow {
                                    target_pos: cp + IVec3::X,
                                    x: 0,
                                    y,
                                    z,
                                    fluid: FluidState::water_flow(next_level),
                                });
                            }
                        }
                        // Horizontal boundary: water at z=0 → flow to -Z neighbor
                        if z == 0 {
                            let next_level = fluid.level.saturating_sub(1);
                            if next_level > 0 {
                                boundary_flows.push(BoundaryFlow {
                                    target_pos: cp + IVec3::NEG_Z,
                                    x,
                                    y,
                                    z: CHUNK_SIZE - 1,
                                    fluid: FluidState::water_flow(next_level),
                                });
                            }
                        }
                        if z == CHUNK_SIZE - 1 {
                            let next_level = fluid.level.saturating_sub(1);
                            if next_level > 0 {
                                boundary_flows.push(BoundaryFlow {
                                    target_pos: cp + IVec3::Z,
                                    x,
                                    y,
                                    z: 0,
                                    fluid: FluidState::water_flow(next_level),
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
                                fluid: FluidState::water_source(),
                            });
                        }
                    }
                }
            }
        }
    }

    // Second pass: write boundary flows into neighbor chunks
    for flow in boundary_flows {
        for (entity, cpos, mut chunk) in &mut param_set.p1() {
            if cpos.0 == flow.target_pos {
                let cell = chunk.cell_xyz(flow.x, flow.y, flow.z);
                if !cell.is_block()
                    && cell
                        .as_fluid()
                        .is_none_or(|f| !f.is_source() && flow.fluid.level > f.level)
                {
                    chunk.set_cell_xyz(flow.x, flow.y, flow.z, ChunkCell::fluid(flow.fluid));
                    commands.entity(entity).insert((
                        ChunkHasActiveFluids,
                        ChunkNeedsSave,
                        ChunkNeedsMeshRebuild,
                    ));
                }
                break;
            }
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
}
