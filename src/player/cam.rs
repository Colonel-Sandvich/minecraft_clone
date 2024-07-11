use std::f32::consts::PI;

use avian3d::collision::ColliderTransform;
use bevy::ecs::event::{Events, ManualEventReader};
use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::render::view::{GpuCulling, NoCpuCulling};
use bevy::window::{CursorGrabMode, PrimaryWindow};

pub struct PlayerCamPlugin;

impl Plugin for PlayerCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputState>()
            .init_state::<MouseState>()
            .add_systems(Update, player_look.run_if(in_state(MouseState::Grabbed)))
            .add_systems(OnEnter(MouseState::Grabbed), grab_cursor)
            .add_systems(OnEnter(MouseState::Free), free_cursor);
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
    pub no_cpu_culling: NoCpuCulling,
    pub gpu_culling: GpuCulling,
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
            no_cpu_culling: NoCpuCulling,
            gpu_culling: GpuCulling,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, States)]
pub enum MouseState {
    #[default]
    Free,
    Grabbed,
}

fn grab_cursor(mut primary_window: Query<&mut Window, With<PrimaryWindow>>) {
    let window = &mut primary_window.single_mut();

    window.cursor.grab_mode = CursorGrabMode::Locked;
    window.cursor.visible = false;
}

fn free_cursor(mut primary_window: Query<&mut Window, With<PrimaryWindow>>) {
    let window = &mut primary_window.single_mut();

    window.cursor.grab_mode = CursorGrabMode::None;
    window.cursor.visible = true;
}

const EPSILON: f32 = 0.01;

fn player_look(
    settings: Res<MouseSettings>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    mut state: ResMut<InputState>,
    motion: Res<Events<MouseMotion>>,
    mut mouse_cam_q: Query<(&mut Transform, &Parent), With<MouseCam>>,
    mut parent_transform_q: Query<&mut ColliderTransform, Without<MouseCam>>,
) {
    if mouse_cam_q.is_empty() {
        return;
    }

    let Ok(window) = primary_window.get_single() else {
        warn!("Primary window not found for `player_move`!");
        return;
    };

    for (mut transform, parent) in mouse_cam_q.iter_mut() {
        for ev in state.reader_motion.read(&motion) {
            let (mut yaw, mut pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
            // Using smallest of height or width ensures equal vertical and horizontal sensitivity
            let window_scale = window.height().min(window.width());
            pitch -= (settings.sensitivity * ev.delta.y * window_scale).to_radians();
            yaw -= (settings.sensitivity * ev.delta.x * window_scale).to_radians();

            pitch = pitch.clamp(-PI / 2.0 + EPSILON, PI / 2.0 - EPSILON);

            // Order is important to prevent unintended roll
            transform.rotation =
                Quat::from_axis_angle(Vec3::Y, yaw) * Quat::from_axis_angle(Vec3::X, pitch);
        }

        let mut parent_transform = parent_transform_q.get_mut(parent.get()).unwrap();

        parent_transform.rotation = Quat::from_axis_angle(Vec3::Y, transform.rotation.y).into();
    }
}
