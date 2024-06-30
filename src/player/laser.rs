use std::ops::Add;

use crate::block;

use super::cam::{MouseCam, MouseGrabbed};
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;

pub struct LaserPlugin;

impl Plugin for LaserPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, laser.run_if(resource_equals(MouseGrabbed(true))));
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
    pub pos: IVec3,
}

fn laser(
    click: Res<ButtonInput<MouseButton>>,
    cameras: Query<&GlobalTransform, With<MouseCam>>,
    context: Res<RapierContext>,
    mut gizmos: Gizmos,
    mut block_click_events: EventWriter<BlockClickEvent>,
) {
    let camera = cameras.single();

    if let Some((_, ray)) = context.cast_ray_and_get_normal(
        camera.translation(),
        camera.forward(),
        5.0,
        true,
        QueryFilter::only_fixed(),
    ) {
        let block =
            (camera.translation() + camera.forward() * (ray.time_of_impact + 0.001)).floor();
        // let block_normal =
        //     (camera.translation() + camera.forward() * (ray.time_of_impact - 0.001)).floor();

        let block_normal = block + ray.normal;

        gizmos.cuboid(
            Transform::from_translation(block + 0.5).with_scale(Vec3::splat(1.01)),
            Color::BLUE,
        );
        gizmos.cuboid(
            Transform::from_translation(block_normal + 0.5).with_scale(Vec3::splat(1.01)),
            Color::RED,
        );
        if click.pressed(MouseButton::Left) {
            block_click_events.send(BlockClickEvent {
                button: MouseButtonForBlock::Left,
                pos: block.as_ivec3(),
            });
        } else if click.pressed(MouseButton::Right) {
            block_click_events.send(BlockClickEvent {
                button: MouseButtonForBlock::Right,
                pos: block_normal.as_ivec3(),
            });
        }
    }
}
