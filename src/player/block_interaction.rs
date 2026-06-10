use avian3d::{
    prelude::Collider,
    spatial_query::{SpatialQuery, SpatialQueryFilter},
};
use bevy::{color::palettes::basic, input::InputSystems, prelude::*};

use crate::{
    block::{BlockPos, BlockType, BlockUpdateKind, BlockUpdateMessage},
    chunk::Chunk,
    dimension::Dimension,
};

use super::cam::MouseCam;

pub struct BlockInteractionPlugin;

impl Plugin for BlockInteractionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CurrentBlockTarget>()
            .init_resource::<SelectedBlock>()
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
    pub hit_block: BlockPos,
    pub adjacent_block: BlockPos,
}

#[derive(Resource, Deref, DerefMut)]
pub struct SelectedBlock(pub BlockType);

impl Default for SelectedBlock {
    fn default() -> Self {
        Self(BlockType::default())
    }
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
    fn block_pos(self) -> BlockPos {
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
        Dir3::from(camera.forward()),
        PLAYER_REACH,
        true,
        &SpatialQueryFilter::default().with_excluded_entities([entity_to_ignore]),
    )?;

    let hit_block =
        (camera.translation() + camera.forward().as_vec3() * (ray.distance + 0.001)).floor();
    let adjacent_block = hit_block + ray.normal;

    Some(BlockTarget {
        hit_block: BlockPos::from_global(hit_block.as_ivec3()),
        adjacent_block: BlockPos::from_global(adjacent_block.as_ivec3()),
    })
}

fn draw_block_target_gizmos(gizmos: &mut Gizmos, target: BlockTarget) {
    gizmos.cube(
        Transform::from_translation(target.hit_block.to_global().as_vec3() + 0.5)
            .with_scale(Vec3::splat(1.01)),
        basic::BLUE,
    );
    gizmos.cube(
        Transform::from_translation(target.adjacent_block.to_global().as_vec3() + 0.5)
            .with_scale(Vec3::splat(1.01)),
        basic::RED,
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
    mut requests: MessageReader<BlockInteractionRequest>,
    dimension: Single<&Dimension>,
    mut chunks: Query<&mut Chunk>,
    mut block_updates: MessageWriter<BlockUpdateMessage>,
    mut selected_block: ResMut<SelectedBlock>,
    spatial_query: SpatialQuery,
) {
    for request in requests.read().copied() {
        let pos = request.block_pos();

        let Some(chunk_entity) = dimension.chunks.get(&pos.chunk) else {
            warn!("Interacted with missing chunk");
            continue;
        };

        let Ok(mut chunk) = chunks.get_mut(*chunk_entity) else {
            continue;
        };

        match request.kind {
            BlockInteractionKind::Pick => {
                selected_block.0 = chunk.get(pos.block);
            }
            BlockInteractionKind::Break => {
                if !chunk.break_block(pos.block) {
                    continue;
                }

                block_updates.write(BlockUpdateMessage {
                    chunk: *chunk_entity,
                    pos,
                    kind: BlockUpdateKind::Break,
                });
            }
            BlockInteractionKind::Place => {
                if block_place_would_intersect(pos, &spatial_query) {
                    continue;
                }

                if !chunk.place_block(pos.block, selected_block.0) {
                    continue;
                }

                block_updates.write(BlockUpdateMessage {
                    chunk: *chunk_entity,
                    pos,
                    kind: BlockUpdateKind::Place(selected_block.0),
                });
            }
        }
    }
}

fn block_place_would_intersect(pos: BlockPos, spatial_query: &SpatialQuery) -> bool {
    !spatial_query
        .shape_intersections(
            &Collider::cuboid(0.90, 0.90, 0.90),
            pos.to_global().as_vec3() + 0.5,
            Quat::IDENTITY,
            &SpatialQueryFilter::default(),
        )
        .is_empty()
}

#[cfg(test)]
#[path = "block_interaction_tests.rs"]
mod tests;
