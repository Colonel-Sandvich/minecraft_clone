use std::f32::consts::PI;

use avian3d::prelude::ColliderTransform;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};

pub struct PlayerCamPlugin;

impl Plugin for PlayerCamPlugin {
    fn build(&self, app: &mut App) {
        // app.init_resource::<InputState>();
        app.init_state::<MouseState>()
            .add_systems(Update, player_look.run_if(in_state(MouseState::Grabbed)))
            .add_systems(OnEnter(MouseState::Grabbed), grab_cursor)
            .add_systems(OnEnter(MouseState::Free), free_cursor);

        app.insert_resource(MouseSettings {
            sensitivity: 0.00007,
            fov: 100.0,
        });
    }
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
#[require(Transform = Transform::default().looking_to(Vec3::X, Vec3::Y))]
#[require(Projection = Projection::Perspective(PerspectiveProjection::default()))]
pub struct MouseCam;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, States)]
pub enum MouseState {
    #[default]
    Free,
    Grabbed,
}

fn grab_cursor(mut window: Single<&mut Window, With<PrimaryWindow>>) {
    window.cursor_options.grab_mode = CursorGrabMode::Locked;
    window.cursor_options.visible = false;
}

fn free_cursor(mut window: Single<&mut Window, With<PrimaryWindow>>) {
    window.cursor_options.grab_mode = CursorGrabMode::None;
    window.cursor_options.visible = true;
}

const EPSILON: f32 = 0.01;

fn player_look(
    settings: Res<MouseSettings>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse_motion: Res<AccumulatedMouseMotion>,
    mut mouse_cam_q: Query<(&mut Transform, &ChildOf), With<MouseCam>>,
    mut parent_transform_q: Query<&mut ColliderTransform, Without<MouseCam>>,
) {
    for (mut transform, parent) in mouse_cam_q.iter_mut() {
        let (mut yaw, mut pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
        // Using smallest of height or width ensures equal vertical and horizontal sensitivity
        let window_scale = window.height().min(window.width());
        pitch -= (settings.sensitivity * mouse_motion.delta.y * window_scale).to_radians();
        yaw -= (settings.sensitivity * mouse_motion.delta.x * window_scale).to_radians();

        pitch = pitch.clamp(-PI / 2.0 + EPSILON, PI / 2.0 - EPSILON);

        // Order is important to prevent unintended roll
        transform.rotation =
            Quat::from_axis_angle(Vec3::Y, yaw) * Quat::from_axis_angle(Vec3::X, pitch);

        let mut parent_transform = parent_transform_q.get_mut(parent.parent()).unwrap();

        parent_transform.rotation = Quat::from_axis_angle(Vec3::Y, transform.rotation.y).into();
    }
}
