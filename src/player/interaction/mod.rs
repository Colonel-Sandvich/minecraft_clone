use avian3d::{
    prelude::Collider,
    spatial_query::{SpatialQuery, SpatialQueryFilter},
};
use bevy::{color::palettes::basic, input::InputSystems, prelude::*};

use crate::{
    block::{BlockUpdateKind, BlockUpdateMessage},
    ui::Hotbar,
    world::{
        ACTOR_LAYER, WORLD_LAYER,
        chunk::{
            Chunk, ChunkBlockPos, ChunkCell, ChunkContentCounts, ChunkNeedsColliderRebuild,
            ChunkNeedsFluidStep, ChunkNeedsLightRebuild, ChunkNeedsMeshRebuild, ChunkNeedsSave,
            WorldBlockPos, chunk_neighbor_offsets_for_block,
        },
        dimension::Dimension,
    },
};

use super::cam::MouseCam;

pub struct BlockInteractionPlugin;

impl Plugin for BlockInteractionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CurrentBlockTarget>()
            .add_message::<BlockInteractionRequest>()
            .add_systems(PreUpdate, update_block_target.after(InputSystems))
            .add_systems(
                FixedUpdate,
                (
                    emit_block_interaction_requests.in_set(BlockInteractionSystems::EmitRequests),
                    apply_block_interaction_requests.after(BlockInteractionSystems::EmitRequests),
                ),
            );
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockInteractionSystems {
    EmitRequests,
}

#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct CurrentBlockTarget(pub Option<BlockTarget>);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockTarget {
    pub hit_block: ChunkBlockPos,
    pub adjacent_block: ChunkBlockPos,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockInteractionKind {
    Pick,
    Break,
    Place,
}

#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub struct BlockInteractionRequest {
    pub kind: BlockInteractionKind,
    pub target: BlockTarget,
}

impl BlockInteractionRequest {
    fn block_pos(self) -> ChunkBlockPos {
        match self.kind {
            BlockInteractionKind::Pick | BlockInteractionKind::Break => self.target.hit_block,
            BlockInteractionKind::Place => self.target.adjacent_block,
        }
    }
}

pub const PLAYER_REACH: f32 = 5.0;

fn update_block_target(
    camera: Single<(&ChildOf, &GlobalTransform), With<MouseCam>>,
    spatial_query: SpatialQuery,
    mut current_target: ResMut<CurrentBlockTarget>,
    mut gizmos: Gizmos,
) {
    current_target.0 = None;

    let (camera_parent, camera) = *camera;
    let Some(target) = raycast_block_target(camera_parent.parent(), camera, &spatial_query) else {
        return;
    };

    draw_block_target_gizmos(&mut gizmos, target);
    current_target.0 = Some(target);
}

fn raycast_block_target(
    entity_to_ignore: Entity,
    camera: &GlobalTransform,
    spatial_query: &SpatialQuery,
) -> Option<BlockTarget> {
    let ray = spatial_query.cast_ray(
        camera.translation(),
        camera.forward(),
        PLAYER_REACH,
        true,
        &SpatialQueryFilter::from_mask(WORLD_LAYER).with_excluded_entities([entity_to_ignore]),
    )?;

    let hit_block = (camera.translation() + camera.forward().as_vec3() * (ray.distance + 0.001))
        .floor()
        .as_ivec3();
    let adjacent_block = hit_block + block_face_normal(ray.normal);

    Some(BlockTarget {
        hit_block: WorldBlockPos::from_ivec3(hit_block).split(),
        adjacent_block: WorldBlockPos::from_ivec3(adjacent_block).split(),
    })
}

fn block_face_normal(normal: Vec3) -> IVec3 {
    let abs = normal.abs();

    if abs.x >= abs.y && abs.x >= abs.z {
        ivec3(normal.x.signum() as i32, 0, 0)
    } else if abs.y >= abs.z {
        ivec3(0, normal.y.signum() as i32, 0)
    } else {
        ivec3(0, 0, normal.z.signum() as i32)
    }
}

fn draw_block_target_gizmos(gizmos: &mut Gizmos, target: BlockTarget) {
    gizmos.cube(
        Transform::from_translation(target.hit_block.world().as_ivec3().as_vec3() + 0.5)
            .with_scale(Vec3::splat(1.01)),
        basic::BLACK,
    );
}

fn emit_block_interaction_requests(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    current_target: Res<CurrentBlockTarget>,
    mut requests: MessageWriter<BlockInteractionRequest>,
) {
    let Some(target) = current_target.0 else {
        return;
    };

    for (button, kind) in [
        (MouseButton::Middle, BlockInteractionKind::Pick),
        (MouseButton::Left, BlockInteractionKind::Break),
        (MouseButton::Right, BlockInteractionKind::Place),
    ] {
        if mouse_buttons.pressed(button) {
            requests.write(BlockInteractionRequest { kind, target });
        }
    }
}

fn apply_block_interaction_requests(
    mut commands: Commands,
    mut requests: MessageReader<BlockInteractionRequest>,
    dimension: Single<&Dimension>,
    mut chunks: Query<&mut Chunk>,
    mut meta_q: Query<&mut ChunkContentCounts>,
    mut block_updates: MessageWriter<BlockUpdateMessage>,
    mut hotbar: ResMut<Hotbar>,
    spatial_query: SpatialQuery,
) {
    for request in requests.read().copied() {
        let pos = request.block_pos();

        let Some(chunk_entity) = dimension.chunk_entity(pos.chunk().as_ivec3()) else {
            warn!("Interacted with missing chunk");
            continue;
        };

        let Ok(mut chunk) = chunks.get_mut(chunk_entity) else {
            continue;
        };

        match request.kind {
            BlockInteractionKind::Pick => {
                hotbar.set_selected_cell(chunk.get_cell(pos.local().as_uvec3()));
            }
            BlockInteractionKind::Break => {
                let Some(delta) = chunk.break_block(pos.local().as_uvec3()) else {
                    continue;
                };
                if let Ok(mut meta) = meta_q.get_mut(chunk_entity) {
                    meta.apply_delta(delta);
                }
                mark_chunk_fluid_activity(&mut commands, chunk_entity, &chunk);

                block_updates.write(BlockUpdateMessage {
                    chunk: chunk_entity,
                    pos,
                    kind: BlockUpdateKind::Break,
                });
                commands.entity(chunk_entity).insert((
                    ChunkNeedsSave,
                    ChunkNeedsMeshRebuild,
                    ChunkNeedsColliderRebuild,
                    ChunkNeedsLightRebuild,
                ));
                mark_boundary_neighbor_meshes_dirty(&mut commands, &dimension, pos);
                mark_block_edit_light_columns_dirty(
                    &mut commands,
                    &dimension,
                    pos.chunk().as_ivec3(),
                );
            }
            BlockInteractionKind::Place => {
                let Some(cell) = hotbar.selected_cell() else {
                    continue;
                };
                if placement_requires_actor_clearance(cell)
                    && block_place_would_intersect(pos, &spatial_query)
                {
                    continue;
                }

                let Some(delta) = chunk.place_cell(pos.local().as_uvec3(), cell) else {
                    continue;
                };
                if let Ok(mut meta) = meta_q.get_mut(chunk_entity) {
                    meta.apply_delta(delta);
                }
                mark_chunk_fluid_activity(&mut commands, chunk_entity, &chunk);

                if let Some(block) = cell.as_block() {
                    block_updates.write(BlockUpdateMessage {
                        chunk: chunk_entity,
                        pos,
                        kind: BlockUpdateKind::Place(block),
                    });
                }
                commands.entity(chunk_entity).insert((
                    ChunkNeedsSave,
                    ChunkNeedsMeshRebuild,
                    ChunkNeedsColliderRebuild,
                    ChunkNeedsLightRebuild,
                ));
                mark_boundary_neighbor_meshes_dirty(&mut commands, &dimension, pos);
                mark_block_edit_light_columns_dirty(
                    &mut commands,
                    &dimension,
                    pos.chunk().as_ivec3(),
                );
            }
        }
    }
}

fn mark_chunk_fluid_activity(commands: &mut Commands, chunk_entity: Entity, chunk: &Chunk) {
    if chunk.has_fluids() {
        commands.entity(chunk_entity).insert(ChunkNeedsFluidStep);
    } else {
        commands
            .entity(chunk_entity)
            .remove::<ChunkNeedsFluidStep>();
    }
}

fn mark_boundary_neighbor_meshes_dirty(
    commands: &mut Commands,
    dimension: &Dimension,
    pos: ChunkBlockPos,
) {
    for offset in chunk_neighbor_offsets_for_block(pos.local().as_uvec3()) {
        let Some(entity) = dimension.chunk_entity(pos.chunk().as_ivec3() + offset) else {
            continue;
        };

        commands
            .entity(entity)
            .insert((ChunkNeedsMeshRebuild, ChunkNeedsFluidStep));
    }
}

fn mark_block_edit_light_columns_dirty(commands: &mut Commands, dimension: &Dimension, pos: IVec3) {
    for (&chunk_pos, &entity) in &dimension.chunks {
        if !block_edit_light_reaches_column(pos, chunk_pos) {
            continue;
        }

        commands.entity(entity).insert(ChunkNeedsLightRebuild);
    }
}

fn block_edit_light_reaches_column(edit_chunk: IVec3, chunk_pos: IVec3) -> bool {
    (chunk_pos.x - edit_chunk.x).abs() <= 1 && (chunk_pos.z - edit_chunk.z).abs() <= 1
}

fn placement_requires_actor_clearance(cell: ChunkCell) -> bool {
    cell.is_solid()
}

fn block_place_would_intersect(pos: ChunkBlockPos, spatial_query: &SpatialQuery) -> bool {
    !spatial_query
        .shape_intersections(
            &Collider::cuboid(0.90, 0.90, 0.90),
            pos.world().as_ivec3().as_vec3() + 0.5,
            Quat::IDENTITY,
            &SpatialQueryFilter::from_mask(ACTOR_LAYER),
        )
        .is_empty()
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
