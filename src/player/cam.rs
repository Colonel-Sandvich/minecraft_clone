use std::f32::consts::PI;

use bevy::ecs::event::{Events, ManualEventReader};
use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};

use super::control::KeyBindings;

pub struct PlayerCamPlugin;

impl Plugin for PlayerCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputState>()
            .insert_resource(MouseGrabbed(false))
            .add_systems(Startup, initial_grab_cursor)
            .add_systems(Startup, initial_grab_on_mousecam_spawn)
            .add_systems(
                Update,
                player_look.run_if(resource_equals(MouseGrabbed(true))),
            )
            .add_systems(Update, cursor_grab);
    }
}

/// Keeps track of mouse motion events, pitch, and yaw
#[derive(Resource, Default)]
pub struct InputState {
    reader_motion: ManualEventReader<MouseMotion>,
}

#[derive(Resource)]
pub struct MouseSettings {
    pub sensitivity: f32,
    pub fov: f32,
}

impl Default for MouseSettings {
    fn default() -> Self {
        Self {
            sensitivity: 0.00012,
            fov: 90.0,
        }
    }
}

#[derive(Component)]
pub struct MouseCam;

#[derive(Bundle)]
pub struct MouseCamBundle {
    pub camera: Camera3dBundle,
    pub mouse_cam: MouseCam,
    pub is_default_ui_camera: IsDefaultUiCamera,
}

impl Default for MouseCamBundle {
    fn default() -> Self {
        Self {
            camera: Camera3dBundle {
                transform: Transform::default().looking_to(Vec3::X, Vec3::Y),
                projection: Projection::Perspective(PerspectiveProjection {
                    fov: MouseSettings::default().fov / 180.0 * PI,
                    ..default()
                }),
                ..default()
            },
            mouse_cam: MouseCam,
            is_default_ui_camera: IsDefaultUiCamera,
        }
    }
}

#[derive(Debug, Resource, PartialEq, Default, Deref, DerefMut)]
pub struct MouseGrabbed(pub bool);

pub fn toggle_grab_cursor(window: &mut Window, mut grabbed: ResMut<MouseGrabbed>) {
    match window.cursor.grab_mode {
        CursorGrabMode::None => {
            window.cursor.grab_mode = CursorGrabMode::Confined;
            window.cursor.visible = false;
            grabbed.0 = true;
        }
        _ => {
            window.cursor.grab_mode = CursorGrabMode::None;
            window.cursor.visible = true;
            grabbed.0 = false;
        }
    }
}

pub fn initial_grab_cursor(
    mut primary_window: Query<&mut Window, With<PrimaryWindow>>,
    grabbed: ResMut<MouseGrabbed>,
) {
    if let Ok(mut window) = primary_window.get_single_mut() {
        toggle_grab_cursor(&mut window, grabbed);
    } else {
        warn!("Primary window not found for `initial_grab_cursor`!");
    }
}

pub fn initial_grab_on_mousecam_spawn(
    mut primary_window: Query<&mut Window, With<PrimaryWindow>>,
    query_added: Query<Entity, Added<MouseCam>>,
    grabbed: ResMut<MouseGrabbed>,
) {
    if query_added.is_empty() {
        return;
    }

    if let Ok(window) = &mut primary_window.get_single_mut() {
        toggle_grab_cursor(window, grabbed);
    } else {
        warn!("Primary window not found for `initial_grab_cursor`!");
    }
}

pub fn cursor_grab(
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    mut primary_window: Query<&mut Window, With<PrimaryWindow>>,
    grabbed: ResMut<MouseGrabbed>,
) {
    if let Ok(mut window) = primary_window.get_single_mut() {
        if keys.just_pressed(key_bindings.toggle_grab_cursor) {
            toggle_grab_cursor(&mut window, grabbed);
        }
    } else {
        warn!("Primary window not found for `cursor_grab`!");
    }
}

pub fn player_look(
    settings: Res<MouseSettings>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    mut state: ResMut<InputState>,
    motion: Res<Events<MouseMotion>>,
    mut mouse_cam_q: Query<&mut Transform, With<MouseCam>>,
) {
    if mouse_cam_q.is_empty() {
        return;
    }

    let Ok(window) = primary_window.get_single() else {
        warn!("Primary window not found for `player_move`!");
        return;
    };

    for mut transform in mouse_cam_q.iter_mut() {
        for ev in state.reader_motion.read(&motion) {
            let (mut yaw, mut pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
            // Using smallest of height or width ensures equal vertical and horizontal sensitivity
            let window_scale = window.height().min(window.width());
            pitch -= (settings.sensitivity * ev.delta.y * window_scale).to_radians();
            yaw -= (settings.sensitivity * ev.delta.x * window_scale).to_radians();

            pitch = pitch.clamp(-PI / 2.0, PI / 2.0);

            // Order is important to prevent unintended roll
            transform.rotation =
                Quat::from_axis_angle(Vec3::Y, yaw) * Quat::from_axis_angle(Vec3::X, pitch);
        }
    }
}
