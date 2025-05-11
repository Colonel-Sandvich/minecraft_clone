use crate::{
    mob::collide_and_slide::CollideAndSlideConfig,
    player::{Player, cam::MouseCam, control::KeyBindings},
};

use avian3d::prelude::RigidBody;
use bevy::prelude::*;

#[derive(Component)]
#[require(RigidBody = RigidBody::Kinematic)]
#[require(Velocity)]
#[require(JumpImpulse = JumpImpulse(5.0))]
#[require(MovementAcceleration = MovementAcceleration(40.0))]
#[require(MovementDampingFactor = MovementDampingFactor(0.9))]
#[require(CollideAndSlideConfig)]
pub struct CharacterController;

#[derive(Component, Default, Deref, DerefMut)]
pub struct Velocity(pub Vec3);

#[derive(Component)]
pub struct JumpImpulse(pub f32);

#[derive(Component, Default)]
pub struct MovementAcceleration(pub f32);

#[derive(Component)]
pub struct MovementDampingFactor(pub f32);

#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct Grounded;

#[derive(Component)]
pub struct FlyController;

#[derive(Component)]
pub struct Flying;

/// Put this on all mobs.
/// If an entity has MouseCam marker component, then update this rotation not its transform

pub fn classic_move(
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>, // Need to change to actions
    mut player_q: Query<
        (
            &MovementAcceleration,
            &JumpImpulse,
            &mut Velocity,
            &Children,
            Has<Grounded>,
            Has<Flying>,
        ),
        With<Player>,
    >,
    camera_q: Query<&Transform, With<MouseCam>>,
    time: Res<Time>,
) {
    for (movement_acceleration, jump_impulse, mut linear_velocity, children, is_grounded, flying) in
        player_q.iter_mut()
    {
        let mut direction = Vec3::ZERO;
        let look_transform = camera_q.get(*children.first().unwrap()).unwrap();
        let forward = look_transform
            .forward()
            .reject_from_normalized(Vec3::Y)
            .normalize();
        let right = look_transform
            .right()
            .reject_from_normalized(Vec3::Y)
            .normalize();

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
            direction.normalize_or_zero() * movement_acceleration.0 * time.delta_secs();

        if keys.pressed(key_bindings.jump) && is_grounded && !flying {
            velocity.y = jump_impulse.0;
        }

        if keys.pressed(key_bindings.sprint) {
            velocity.x *= 2.0;
            velocity.z *= 2.0;
        }

        linear_velocity.0 += velocity;
    }
}

pub fn fly_move(
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    mut player_q: Query<(&MovementAcceleration, &mut Velocity), (With<Flying>, With<Player>)>,
    time: Res<Time>,
) {
    for (movement_acceleration, mut linear_velocity) in player_q.iter_mut() {
        let mut direction = Vec3::ZERO;

        for key in keys.get_pressed() {
            let key = *key;
            direction += if key == key_bindings.move_ascend {
                Vec3::Y
            } else if key == key_bindings.move_descend {
                -Vec3::Y
            } else {
                Vec3::ZERO
            }
        }

        let velocity = direction.normalize_or_zero() * movement_acceleration.0 * time.delta_secs();

        linear_velocity.0 += velocity;
    }
}
