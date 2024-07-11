use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};
use bevy_rapier3d::control::KinematicCharacterController;

use super::cam::MouseCam;
use super::control::KeyBindings;
use super::Player;

pub struct FlyControllerPlugin;

impl Plugin for FlyControllerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(FixedUpdate, player_move);
    }
}

#[derive(Resource)]
pub struct MovementSettings {
    pub speed: f32,
}

impl Default for MovementSettings {
    fn default() -> Self {
        Self { speed: 12.0 }
    }
}

#[derive(Component)]
pub struct FlyController;

#[derive(Component)]
pub struct Flying;

#[derive(Component)]
pub struct Velocity;

pub fn player_move(
    keys: Res<ButtonInput<KeyCode>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    settings: Res<MovementSettings>,
    key_bindings: Res<KeyBindings>,
    time: Res<Time>,
    mut player_q: Query<&mut KinematicCharacterController, (With<FlyController>, With<Player>)>,
    mouse_cam_q: Query<(&Transform, &Parent), With<MouseCam>>,
) {
    if mouse_cam_q.is_empty() {
        return;
    }

    let Ok(window) = primary_window.get_single() else {
        warn!("Primary window not found for `player_move`!");
        return;
    };

    for (look_transform, parent) in mouse_cam_q.iter() {
        let mut kinematic_controller = player_q.get_mut(parent.get()).unwrap();

        let mut velocity = Vec3::ZERO;
        let local_z = look_transform.local_z();
        let forward = -Vec3::new(local_z.x, 0., local_z.z);
        let right = Vec3::new(local_z.z, 0., -local_z.x);
        let mut sprinting = false;

        for key in keys.get_pressed() {
            match window.cursor.grab_mode {
                CursorGrabMode::None => (),
                _ => {
                    let key = *key;
                    if key == key_bindings.move_forward {
                        velocity += forward;
                    } else if key == key_bindings.move_backward {
                        velocity -= forward;
                    } else if key == key_bindings.move_left {
                        velocity -= right;
                    } else if key == key_bindings.move_right {
                        velocity += right;
                    } else if key == key_bindings.move_ascend {
                        velocity += Vec3::Y;
                    } else if key == key_bindings.move_descend {
                        velocity -= Vec3::Y;
                    }

                    if key == key_bindings.sprint {
                        sprinting = true;
                    }
                }
            }
        }

        velocity = velocity.normalize_or_zero() * settings.speed * time.delta_seconds();

        if sprinting {
            velocity *= 2.0;
        }

        kinematic_controller.translation =
            Some(kinematic_controller.translation.unwrap_or(Vec3::ZERO) + velocity);
    }
}
