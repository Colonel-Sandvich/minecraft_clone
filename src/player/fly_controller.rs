use super::cam::MouseCam;
use super::control::KeyBindings;
use super::Player;
use crate::mob::MovementAcceleration;
use avian3d::prelude::LinearVelocity;
use bevy::math::vec3;
use bevy::prelude::*;

#[derive(Component)]
pub struct FlyController;

#[derive(Component)]
pub struct Flying;

pub fn fly_move(
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    mut player_q: Query<
        (&MovementAcceleration, &mut LinearVelocity, &Children),
        (With<Flying>, With<Player>),
    >,
    camera_q: Query<&Transform, With<MouseCam>>,
    time: Res<Time>,
) {
    for (movement_acceleration, mut linear_velocity, children) in player_q.iter_mut() {
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
            } else if key == key_bindings.move_ascend {
                Vec3::Y
            } else if key == key_bindings.move_descend {
                -Vec3::Y
            } else {
                Vec3::ZERO
            }
        }

        let mut velocity =
            direction.normalize_or_zero() * movement_acceleration.0 * time.delta_seconds();

        if keys.just_pressed(key_bindings.sprint) {
            velocity *= 2.0;
        }

        linear_velocity.0 += velocity;
    }
}
