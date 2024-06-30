use super::{
    cam::MouseCamBundle,
    fly_controller::{FlyController, Flying},
    make_collider, Player, PLAYER_HEIGHT,
};
use bevy::{math::vec3, prelude::*};
use bevy_rapier3d::prelude::*;

use crate::chunk::CHUNK_SIZE;

pub struct SpawnPlayerPlugin;

impl Plugin for SpawnPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup);
    }
}

fn setup(mut commands: Commands) {
    commands
        .spawn((
            Player::default(),
            SpatialBundle {
                transform: Transform::from_translation(Vec3::new(
                    0.0,
                    CHUNK_SIZE as f32 + 2.0,
                    0.0,
                )),
                ..default()
            },
            FlyController,
            Flying,
            RigidBody::KinematicVelocityBased,
            KinematicCharacterController {
                autostep: Some(CharacterAutostep::default()),
                apply_impulse_to_dynamic_bodies: true,
                ..default()
            },
            Velocity::default(),
            make_collider(),
            LockedAxes::ROTATION_LOCKED,
            Sleeping::disabled(),
            Ccd::enabled(),
            GravityScale(1.0),
        ))
        .with_children(|p| {
            let mut mouse_cam_bundle = MouseCamBundle::default();
            mouse_cam_bundle.camera.transform.translation = vec3(0.0, PLAYER_HEIGHT / 2.0, 0.0);
            p.spawn(mouse_cam_bundle);
        });
}
