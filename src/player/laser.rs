use avian3d::spatial_query::{SpatialQuery, SpatialQueryFilter};
use bevy::{color::palettes::basic, input::InputSystem};

use crate::block::BlockPos;

use super::cam::MouseCam;
use bevy::prelude::*;

pub struct LaserPlugin;

impl Plugin for LaserPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BlockEvents>();
        app.add_systems(PreUpdate, laser.after(InputSystem));
        app.add_systems(FixedUpdate, act_on_clicks);
        app.add_event::<BlockClickEvent>();
    }
}

pub enum MouseButtonForBlock {
    Left,
    Right,
    Middle,
}

#[derive(Event)]
pub struct BlockClickEvent {
    pub button: MouseButtonForBlock,
    pub pos: BlockPos,
}

pub const PLAYER_REACH: f32 = 5.0;

#[derive(Resource, Default)]
pub struct BlockEvents {
    pub left: Option<BlockClickEvent>,
    pub right: Option<BlockClickEvent>,
    pub middle: Option<BlockClickEvent>,
}

fn laser(
    click: Res<ButtonInput<MouseButton>>,
    cameras: Query<(&Parent, &GlobalTransform), With<MouseCam>>,
    spatial_query: SpatialQuery,
    mut queued_events: ResMut<BlockEvents>,
    mut gizmos: Gizmos,
) {
    let (camera_parent, camera) = cameras.single();

    if let Some(ray) = spatial_query.cast_ray(
        camera.translation(),
        Dir3::from(camera.forward()),
        PLAYER_REACH,
        true,
        SpatialQueryFilter::default().with_excluded_entities([camera_parent.get()]),
    ) {
        let block =
            (camera.translation() + camera.forward() * (ray.time_of_impact + 0.001)).floor();
        // let block_normal =
        //     (camera.translation() + camera.forward() * (ray.time_of_impact - 0.001)).floor();

        let block_normal = block + ray.normal;

        let block_pos = BlockPos::from_global(block.as_ivec3());
        let block_normal_pos = BlockPos::from_global(block_normal.as_ivec3());

        gizmos.cuboid(
            Transform::from_translation(block + 0.5).with_scale(Vec3::splat(1.01)),
            basic::BLUE,
        );
        gizmos.cuboid(
            Transform::from_translation(block_normal + 0.5).with_scale(Vec3::splat(1.01)),
            basic::RED,
        );

        if click.pressed(MouseButton::Left) {
            queued_events.left = Some(BlockClickEvent {
                button: MouseButtonForBlock::Left,
                pos: block_pos,
            });
        } else if click.pressed(MouseButton::Right) {
            queued_events.right = Some(BlockClickEvent {
                button: MouseButtonForBlock::Right,
                pos: block_normal_pos,
            });
        } else if click.pressed(MouseButton::Middle) {
            queued_events.middle = Some(BlockClickEvent {
                button: MouseButtonForBlock::Middle,
                pos: block_pos,
            });
        }
    }
}

fn act_on_clicks(
    mut queued_events: ResMut<BlockEvents>,
    mut block_click_events: EventWriter<BlockClickEvent>,
) {
    if let Some(left) = queued_events.left.take() {
        block_click_events.send(left);
    }
    if let Some(right) = queued_events.right.take() {
        block_click_events.send(right);
    }
    if let Some(middle) = queued_events.middle.take() {
        block_click_events.send(middle);
    }
}
