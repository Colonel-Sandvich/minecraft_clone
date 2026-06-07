use avian3d::spatial_query::{SpatialQuery, SpatialQueryFilter};
use bevy::{color::palettes::basic, input::InputSystems};

use crate::block::BlockPos;

use super::cam::MouseCam;
use bevy::prelude::*;

pub struct LaserPlugin;

impl Plugin for LaserPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BlockMessages>();
        app.add_systems(PreUpdate, laser.after(InputSystems));
        app.add_systems(FixedUpdate, act_on_clicks);
        app.add_message::<BlockClickMessage>();
    }
}

pub enum MouseButtonForBlock {
    Left,
    Right,
    Middle,
}

#[derive(Message)]
pub struct BlockClickMessage {
    pub button: MouseButtonForBlock,
    pub pos: BlockPos,
}

pub const PLAYER_REACH: f32 = 5.0;

#[derive(Resource, Default)]
pub struct BlockMessages {
    pub left: Option<BlockClickMessage>,
    pub right: Option<BlockClickMessage>,
    pub middle: Option<BlockClickMessage>,
}

fn laser(
    click: Res<ButtonInput<MouseButton>>,
    camera: Single<(&ChildOf, &GlobalTransform), With<MouseCam>>,
    spatial_query: SpatialQuery,
    mut queued_events: ResMut<BlockMessages>,
    mut gizmos: Gizmos,
) {
    let (camera_parent, camera) = *camera;

    if let Some(ray) = spatial_query.cast_ray(
        camera.translation(),
        Dir3::from(camera.forward()),
        PLAYER_REACH,
        true,
        &SpatialQueryFilter::default().with_excluded_entities([camera_parent.parent()]),
    ) {
        let block =
            (camera.translation() + camera.forward().as_vec3() * (ray.distance + 0.001)).floor();

        let block_normal = block + ray.normal;

        let block_pos = BlockPos::from_global(block.as_ivec3());
        let block_normal_pos = BlockPos::from_global(block_normal.as_ivec3());

        gizmos.cube(
            Transform::from_translation(block + 0.5).with_scale(Vec3::splat(1.01)),
            basic::BLUE,
        );
        gizmos.cube(
            Transform::from_translation(block_normal + 0.5).with_scale(Vec3::splat(1.01)),
            basic::RED,
        );

        if click.pressed(MouseButton::Left) {
            queued_events.left = Some(BlockClickMessage {
                button: MouseButtonForBlock::Left,
                pos: block_pos,
            });
        } else if click.pressed(MouseButton::Right) {
            queued_events.right = Some(BlockClickMessage {
                button: MouseButtonForBlock::Right,
                pos: block_normal_pos,
            });
        } else if click.pressed(MouseButton::Middle) {
            queued_events.middle = Some(BlockClickMessage {
                button: MouseButtonForBlock::Middle,
                pos: block_pos,
            });
        }
    }
}

fn act_on_clicks(
    mut queued_events: ResMut<BlockMessages>,
    mut block_click_events: MessageWriter<BlockClickMessage>,
) {
    if let Some(left) = queued_events.left.take() {
        block_click_events.write(left);
    }
    if let Some(right) = queued_events.right.take() {
        block_click_events.write(right);
    }
    if let Some(middle) = queued_events.middle.take() {
        block_click_events.write(middle);
    }
}
