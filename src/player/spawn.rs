use std::f32::consts::PI;

use super::{
    cam::MouseCamBundle,
    fly_controller::{FlyController, Flying},
    make_collider, Player, PLAYER_HEIGHT,
};
use bevy::{math::vec3, prelude::*};

use crate::{chunk::CHUNK_SIZE, mob::CharacterControllerBundle};

pub struct SpawnPlayerPlugin;

impl Plugin for SpawnPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup);
    }
}

pub const SPAWN_POINT: Vec3 = Vec3::new(0.5, CHUNK_SIZE as f32 + 2.0, 0.5);

fn setup(mut commands: Commands) {
    commands
        .spawn((
            Player::default(),
            SpatialBundle {
                transform: Transform::from_translation(SPAWN_POINT),
                ..default()
            },
            FlyController,
            // Flying,
            CharacterControllerBundle::new(make_collider(), 9.81 * Vec3::NEG_Y).with_movement(
                50.0,
                0.95,
                1.5,
                PI / 4.0,
            ),
        ))
        .with_children(|p| {
            let mut mouse_cam_bundle = MouseCamBundle::default();
            mouse_cam_bundle.camera.transform.translation = vec3(0.0, PLAYER_HEIGHT / 2.0, 0.0);
            p.spawn(mouse_cam_bundle);
        });
}
