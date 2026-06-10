use crate::{
    mob::collide_and_slide::CollideAndSlideConfig,
    player::{Player, cam::MouseCam, control::KeyBindings},
};

use avian3d::prelude::RigidBody;
use bevy::prelude::*;

#[derive(Component)]
#[require(RigidBody = RigidBody::Kinematic)]
#[require(Velocity)]
#[require(JumpImpulse = JumpImpulse(8.4))]
#[require(MovementAcceleration = MovementAcceleration(39.2))]
#[require(AirMovementAcceleration = AirMovementAcceleration(8.0))]
#[require(CollideAndSlideConfig)]
pub struct CharacterController;

#[derive(Component, Default, Deref, DerefMut)]
pub struct Velocity(pub Vec3);

#[derive(Component)]
pub struct JumpImpulse(pub f32);

#[derive(Component, Default)]
pub struct MovementAcceleration(pub f32);

/// Vanilla air control is much weaker than ground acceleration:
/// 0.02 blocks/tick = 0.4 blocks/s added per tick = 8.0 blocks/s² at 20 TPS.
#[derive(Component)]
pub struct AirMovementAcceleration(pub f32);

#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct Grounded;

#[derive(Component)]
pub struct FlyController;

#[derive(Component)]
pub struct Flying;

pub fn apply_player_movement_input(
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    player_q: Single<
        (
            &MovementAcceleration,
            &AirMovementAcceleration,
            &JumpImpulse,
            &mut Velocity,
            &Children,
            Has<Grounded>,
            Has<Flying>,
        ),
        With<Player>,
    >,
    camera_q: Query<&Transform, With<MouseCam>>,
    time: Res<Time<Fixed>>,
) {
    let (
        movement_acceleration,
        air_movement_acceleration,
        jump_impulse,
        mut linear_velocity,
        children,
        is_grounded,
        flying,
    ) = player_q.into_inner();
    let movement_intent = key_bindings.movement_intent(&keys);

    let look_transform = camera_q.get(*children.first().unwrap()).unwrap();
    let look_forward = horizontal_direction(*look_transform.forward());
    let move_direction = world_move_direction(
        *look_transform.forward(),
        *look_transform.right(),
        movement_intent.local_move_axis,
    );
    let sprint_active = movement_intent.wants_forward_sprint();

    let velocity_delta = horizontal_velocity_delta(
        move_direction,
        movement_acceleration.0,
        air_movement_acceleration.0,
        time.delta_secs(),
        is_grounded,
        flying,
        sprint_active,
    );

    linear_velocity.0 += velocity_delta;

    if movement_intent.jump && is_grounded && !flying {
        // Vanilla sets vertical jump velocity; it does not add the jump impulse
        // to any residual downward velocity left by the previous grounded tick.
        apply_jump_impulse(
            &mut linear_velocity.0,
            look_forward,
            jump_impulse.0,
            sprint_active,
        );
    }
}

pub(crate) fn world_move_direction(
    look_forward: Vec3,
    look_right: Vec3,
    local_move_axis: Vec3,
) -> Vec3 {
    let forward = horizontal_direction(look_forward);
    let right = horizontal_direction(look_right);

    (forward * local_move_axis.z + right * local_move_axis.x).normalize_or_zero()
}

fn horizontal_direction(mut direction: Vec3) -> Vec3 {
    direction.y = 0.0;
    direction.normalize_or_zero()
}

pub fn horizontal_velocity_delta(
    move_direction: Vec3,
    ground_acceleration: f32,
    air_acceleration: f32,
    delta_secs: f32,
    is_grounded: bool,
    flying: bool,
    wants_sprint: bool,
) -> Vec3 {
    let horizontal_acceleration = if is_grounded || flying {
        ground_acceleration
    } else {
        air_acceleration
    };
    let sprint_multiplier = if wants_sprint { 1.3 } else { 1.0 };
    move_direction * horizontal_acceleration * delta_secs * sprint_multiplier
}

pub fn apply_jump_impulse(
    velocity: &mut Vec3,
    look_forward: Vec3,
    jump_impulse: f32,
    wants_sprint: bool,
) {
    velocity.y = jump_impulse;
    if wants_sprint {
        *velocity += look_forward.normalize_or_zero() * 4.0;
    }
}

pub fn apply_flight_vertical_input(
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    mut player_q: Query<(&MovementAcceleration, &mut Velocity), (With<Flying>, With<Player>)>,
    time: Res<Time<Fixed>>,
) {
    let movement_intent = key_bindings.movement_intent(&keys);
    let vertical_direction = Vec3::Y * movement_intent.local_move_axis.y;

    for (movement_acceleration, mut linear_velocity) in player_q.iter_mut() {
        let velocity_delta = vertical_direction * movement_acceleration.0 * time.delta_secs();
        linear_velocity.0 += velocity_delta;
    }
}
