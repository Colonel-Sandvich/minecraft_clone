use super::fly_controller::Flying;
use super::Player;
use super::{cam::MouseCam, control::KeyBindings};
use crate::mob::{Grounded, JumpImpulse, MovementAcceleration};
use avian3d::prelude::LinearVelocity;
use bevy::math::vec3;
use bevy::prelude::*;

pub fn classic_move(
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    mut player_q: Query<
        (
            &MovementAcceleration,
            &JumpImpulse,
            &mut LinearVelocity,
            Has<Grounded>,
            &Children,
        ),
        (Without<Flying>, With<Player>),
    >,
    camera_q: Query<&Transform, With<MouseCam>>,
    time: Res<Time>,
) {
    for (movement_acceleration, jump_impulse, mut linear_velocity, is_grounded, children) in
        player_q.iter_mut()
    {
        let mut direction = Vec3::ZERO;
        let look_transform = camera_q.get(*children.first().unwrap()).unwrap();
        let local_z = look_transform.local_z();

        let forward = -vec3(local_z.x, 0.0, local_z.z);
        let right = vec3(local_z.z, 0.0, -local_z.x);

        for key in keys.get_pressed() {
            let key = *key;
            direction += if key == key_bindings.move_forward {
                forward
            } else if key == key_bindings.move_backward {
                -forward
            } else if key == key_bindings.move_left {
                -right
            } else if key == key_bindings.move_right {
                right
            } else {
                Vec3::ZERO
            }
        }

        let mut velocity =
            direction.normalize_or_zero() * movement_acceleration.0 * time.delta_seconds();

        if keys.pressed(key_bindings.jump) && is_grounded {
            velocity.y = jump_impulse.0;
        }

        if keys.pressed(key_bindings.sprint) {
            velocity.x *= 2.0;
            velocity.z *= 2.0;
        }

        linear_velocity.0 += velocity;
    }
}
