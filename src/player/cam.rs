use std::f32::consts::PI;

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_settings::{ReflectSettingsGroup, SettingsGroup};

pub struct PlayerCamPlugin;

impl Plugin for PlayerCamPlugin {
    fn build(&self, app: &mut App) {
        // app.init_resource::<InputState>();
        app.init_state::<MouseState>()
            .add_systems(Update, player_look.run_if(in_state(MouseState::Grabbed)))
            .add_systems(OnEnter(MouseState::Grabbed), grab_cursor)
            .add_systems(OnEnter(MouseState::Free), free_cursor);

        app.init_resource::<MouseSettings>();
    }
}

#[derive(Resource, SettingsGroup, Reflect, Debug, Clone, Copy)]
#[reflect(Resource, SettingsGroup, Default)]
pub struct MouseSettings {
    pub sensitivity: f32,
    pub fov: f32,
}

impl Default for MouseSettings {
    fn default() -> Self {
        Self {
            sensitivity: 0.00007,
            fov: 100.0,
        }
    }
}

#[derive(Component)]
#[require(Transform = Transform::default().looking_to(Vec3::X, Vec3::Y))]
#[require(Projection = Projection::Perspective(PerspectiveProjection::default()))]
pub struct MouseCam;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, States)]
pub enum MouseState {
    #[default]
    Free,
    Grabbed,
}

fn grab_cursor(mut cursor_options: Single<&mut CursorOptions, With<PrimaryWindow>>) {
    cursor_options.grab_mode = CursorGrabMode::Locked;
    cursor_options.visible = false;
}

fn free_cursor(mut cursor_options: Single<&mut CursorOptions, With<PrimaryWindow>>) {
    cursor_options.grab_mode = CursorGrabMode::None;
    cursor_options.visible = true;
}

const EPSILON: f32 = 0.01;

fn player_look(
    settings: Res<MouseSettings>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse_motion: Res<AccumulatedMouseMotion>,
    mut mouse_cam: Single<&mut Transform, With<MouseCam>>,
) {
    let (mut yaw, mut pitch, _) = mouse_cam.rotation.to_euler(EulerRot::YXZ);
    let window_scale = window.height().min(window.width());
    pitch -= (settings.sensitivity * mouse_motion.delta.y * window_scale).to_radians();
    yaw -= (settings.sensitivity * mouse_motion.delta.x * window_scale).to_radians();

    pitch = pitch.clamp(-PI / 2.0 + EPSILON, PI / 2.0 - EPSILON);

    mouse_cam.rotation =
        Quat::from_axis_angle(Vec3::Y, yaw) * Quat::from_axis_angle(Vec3::X, pitch);
}
