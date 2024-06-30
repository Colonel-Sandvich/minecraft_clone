use bevy::{math::vec3, prelude::*};
use bevy_rapier3d::prelude::*;

use crate::{
    chunk::CHUNK_SIZE,
    player::{fly_controller::Flying, make_collider, PLAYER_HEIGHT, PLAYER_LENGTH, PLAYER_WIDTH},
};

pub struct MobPlugin;

impl Plugin for MobPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, spawn_creeper);
        app.add_systems(FixedUpdate, apply_velocity);
        app.add_systems(FixedUpdate, apply_gravity);
    }
}

#[derive(Component)]
struct Creeper;

fn spawn_creeper(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands
        .spawn((
            SpatialBundle {
                transform: Transform::from_translation(Vec3::new(
                    5.0,
                    CHUNK_SIZE as f32 + 3.0,
                    5.0,
                )),
                ..default()
            },
            RigidBody::KinematicVelocityBased,
            KinematicCharacterController {
                autostep: Some(CharacterAutostep::default()),
                apply_impulse_to_dynamic_bodies: true,
                ..default()
            },
            make_collider(),
            LockedAxes::ROTATION_LOCKED,
            GravityScale(1.0),
            Creeper,
        ))
        .with_children(|p| {
            p.spawn(PbrBundle {
                mesh: meshes.add(Cuboid::new(PLAYER_WIDTH, PLAYER_HEIGHT, PLAYER_LENGTH)),
                material: materials.add(Color::GREEN),
                ..default()
            });
        });
}

fn apply_velocity(
    time: Res<Time>,
    mut controllers: Query<(&mut KinematicCharacterController, &mut Velocity)>,
) {
    for (mut controller, velocity) in controllers.iter_mut() {
        controller.translation = Some(
            controller.translation.unwrap_or(Vec3::ZERO) + velocity.linvel * time.delta_seconds(),
        );
    }
}

pub const GRAVITY: f32 = 9.81;

fn apply_gravity(
    time: Res<Time>,
    mut controllers: Query<
        (&mut Velocity, &GravityScale),
        (With<KinematicCharacterController>, Without<Flying>),
    >,
) {
    for (mut velocity, gravity) in controllers.iter_mut() {
        velocity.linvel += vec3(0.0, -gravity.0 * GRAVITY, 0.0) * time.delta_seconds();
    }
}

fn read_character_controller_collisions(
    mut character_controller_outputs: Query<&mut KinematicCharacterControllerOutput>,
) {
    for mut output in character_controller_outputs.iter_mut() {
        for collision in &output.collisions {
            // Do something with that collision information.
        }
    }
}
