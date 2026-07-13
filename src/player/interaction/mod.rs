use avian3d::{
    prelude::Collider,
    spatial_query::{SpatialQuery, SpatialQueryFilter},
};
use bevy::{color::palettes::basic, input::InputSystems, prelude::*};

use crate::{
    game_state::GameState,
    ui::Hotbar,
    world::{
        ACTOR_LAYER, WORLD_LAYER,
        chunk::{
            CellDelta, Chunk, ChunkBlockPos, ChunkCell, ChunkContentCounts, ChunkEditor,
            ChunkInvalidationPlan, WorldBlockPos,
        },
        dimension::{Active, Dimension, apply_chunk_invalidations},
    },
};

use super::cam::{MouseCam, MouseState, gameplay_input_active};

pub struct BlockInteractionPlugin;

impl Plugin for BlockInteractionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CurrentBlockTarget>()
            .add_message::<BlockInteractionRequest>()
            .add_message::<BlockEditCommitted>()
            .add_systems(
                PreUpdate,
                update_block_target
                    .run_if(gameplay_input_active)
                    .after(InputSystems),
            )
            .add_systems(OnEnter(GameState::Paused), clear_block_interaction_state)
            .add_systems(OnEnter(GameState::Playing), clear_block_interaction_state)
            .add_systems(OnEnter(MouseState::Free), clear_block_interaction_state)
            .add_systems(
                FixedUpdate,
                (
                    emit_block_interaction_requests.in_set(BlockInteractionSystems::EmitRequests),
                    apply_block_interaction_requests
                        .after(BlockInteractionSystems::EmitRequests)
                        .in_set(crate::world::ChunkSimulationSet::ExternalMutation),
                )
                    .run_if(gameplay_input_active),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockEditKind {
    Break,
    Place,
}

/// A player edit that crossed the world's tracked mutation boundary.
///
/// Consumers such as audio and particles should react to this message instead
/// of raw input, because failed placement and no-op edits never produce it.
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockEditCommitted {
    pub kind: BlockEditKind,
    pub position: ChunkBlockPos,
    pub delta: CellDelta,
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

fn clear_block_interaction_state(
    mut current_target: ResMut<CurrentBlockTarget>,
    requests: Option<ResMut<Messages<BlockInteractionRequest>>>,
) {
    current_target.0 = None;
    if let Some(mut requests) = requests {
        requests.clear();
    }
}

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
    mut committed_edits: MessageWriter<BlockEditCommitted>,
    dimension: Single<&mut Dimension, With<Active>>,
    mut chunks: Query<(&mut Chunk, &mut ChunkContentCounts)>,
    mut hotbar: ResMut<Hotbar>,
    spatial_query: SpatialQuery,
) {
    let mut dimension = dimension.into_inner();
    let mut invalidations = ChunkInvalidationPlan::new();

    for request in requests.read().copied() {
        let pos = request.block_pos();

        let Some(chunk_entity) = dimension.published_chunk_entity(pos.chunk()) else {
            warn!("Interacted with missing chunk");
            continue;
        };

        let Ok((mut chunk, mut counts)) = chunks.get_mut(chunk_entity) else {
            continue;
        };

        let delta = match request.kind {
            BlockInteractionKind::Pick => {
                hotbar.set_selected_cell(chunk.cell(pos.local()));
                None
            }
            BlockInteractionKind::Break => {
                let mut editor =
                    ChunkEditor::new(pos.chunk(), &mut chunk, &mut counts, &mut invalidations);
                editor.break_block(pos.local())
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

                let mut editor =
                    ChunkEditor::new(pos.chunk(), &mut chunk, &mut counts, &mut invalidations);
                editor.place_cell(pos.local(), cell)
            }
        };

        if let Some(edit) = committed_block_edit(request.kind, pos, delta) {
            committed_edits.write(edit);
        }
    }

    apply_chunk_invalidations(&mut commands, &mut dimension, &invalidations);
}

fn committed_block_edit(
    interaction: BlockInteractionKind,
    position: ChunkBlockPos,
    delta: Option<CellDelta>,
) -> Option<BlockEditCommitted> {
    let kind = match interaction {
        BlockInteractionKind::Break => BlockEditKind::Break,
        BlockInteractionKind::Place => BlockEditKind::Place,
        BlockInteractionKind::Pick => return None,
    };

    Some(BlockEditCommitted {
        kind,
        position,
        delta: delta?,
    })
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
