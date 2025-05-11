pub mod collide_and_slide;
pub mod controller;

use crate::player::cam::MouseState;
use bevy::prelude::*;
use collide_and_slide::mov_system;
use controller::{Flying, MovementDampingFactor, Velocity, classic_move, fly_move};

pub struct MobControllerPlugin;

impl Plugin for MobControllerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (
                apply_gravity,
                (fly_move, classic_move).distributive_run_if(in_state(MouseState::Grabbed)),
                apply_movement_damping,
                mov_system,
            )
                .chain(),
        );
    }
}

const GRAVITY: f32 = 9.81;

fn apply_gravity(time: Res<Time>, mut controllers: Query<&mut Velocity, Without<Flying>>) {
    for mut velocity in &mut controllers {
        velocity.0 += GRAVITY * Vec3::NEG_Y * time.delta_secs();
    }
}

const SPEED_THRESHOLD: f32 = 0.01;

fn apply_movement_damping(mut query: Query<(&MovementDampingFactor, &mut Velocity, Has<Flying>)>) {
    for (damping_factor, mut velocity, is_flying) in &mut query {
        velocity.0.x *= damping_factor.0;
        velocity.0.z *= damping_factor.0;
        if is_flying {
            velocity.0.y *= damping_factor.0;
        }

        if velocity.x.abs() < SPEED_THRESHOLD {
            velocity.x = 0.0;
        }

        if velocity.y.abs() < SPEED_THRESHOLD {
            velocity.y = 0.0;
        }

        if velocity.z.abs() < SPEED_THRESHOLD {
            velocity.z = 0.0;
        }
    }
}
