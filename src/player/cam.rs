use std::f32::consts::PI;

use bevy::input::{InputSystems, mouse::AccumulatedMouseMotion};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_settings::{ReflectSettingsGroup, SettingsGroup};

use crate::game_state::GameState;

pub struct PlayerCamPlugin;

impl Plugin for PlayerCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<MouseState>()
            .add_systems(
                PreUpdate,
                player_look
                    .in_set(PlayerCameraSystems::Look)
                    .after(InputSystems)
                    .run_if(gameplay_input_active),
            )
            .add_systems(OnEnter(MouseState::Grabbed), apply_grabbed_cursor)
            .add_systems(OnEnter(MouseState::Free), apply_free_cursor)
            .add_systems(OnEnter(GameState::Playing), enter_gameplay_cursor)
            .add_systems(OnExit(GameState::Playing), leave_gameplay_cursor)
            .add_systems(PreUpdate, release_unfocused_cursor.after(InputSystems));

        app.init_resource::<MouseSettings>();
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerCameraSystems {
    Look,
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

pub(crate) fn gameplay_input_active(
    game_state: Option<Res<State<GameState>>>,
    mouse_state: Option<Res<State<MouseState>>>,
    keyboard: Option<Res<ButtonInput<KeyCode>>>,
    primary_windows: Query<(&Window, &CursorOptions), With<PrimaryWindow>>,
) -> bool {
    let (Some(game_state), Some(mouse_state), Some(keyboard)) = (game_state, mouse_state, keyboard)
    else {
        return false;
    };
    let Ok((window, cursor_options)) = primary_windows.single() else {
        return false;
    };

    gameplay_input_is_active(
        *game_state.get(),
        *mouse_state.get(),
        window.focused,
        cursor_options.grab_mode,
        keyboard.just_pressed(KeyCode::Escape),
    )
}

pub(crate) fn gameplay_input_is_active(
    game_state: GameState,
    mouse_state: MouseState,
    window_focused: bool,
    cursor_grab_mode: CursorGrabMode,
    pause_requested: bool,
) -> bool {
    game_state == GameState::Playing
        && mouse_state == MouseState::Grabbed
        && window_focused
        && cursor_grab_mode != CursorGrabMode::None
        && !pause_requested
}

fn set_cursor_grabbed(cursor_options: &mut CursorOptions) {
    cursor_options.grab_mode = CursorGrabMode::Locked;
    cursor_options.visible = false;
}

fn set_cursor_free(cursor_options: &mut CursorOptions) {
    cursor_options.grab_mode = CursorGrabMode::None;
    cursor_options.visible = true;
}

fn apply_grabbed_cursor(
    game_state: Res<State<GameState>>,
    mut primary_windows: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut next_mouse_state: ResMut<NextState<MouseState>>,
) {
    if *game_state.get() == GameState::Playing
        && let Ok((window, mut cursor_options)) = primary_windows.single_mut()
        && window.focused
    {
        set_cursor_grabbed(&mut cursor_options);
        return;
    }

    for (_, mut cursor_options) in &mut primary_windows {
        set_cursor_free(&mut cursor_options);
    }
    next_mouse_state.set(MouseState::Free);
}

fn apply_free_cursor(mut primary_windows: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    for mut cursor_options in &mut primary_windows {
        set_cursor_free(&mut cursor_options);
    }
}

fn enter_gameplay_cursor(
    mut primary_windows: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut next_mouse_state: ResMut<NextState<MouseState>>,
) {
    if let Ok((window, mut cursor_options)) = primary_windows.single_mut()
        && window.focused
    {
        // Apply immediately so the resumed frame cannot briefly expose a free
        // cursor, then keep MouseState in sync for gameplay run conditions.
        set_cursor_grabbed(&mut cursor_options);
        next_mouse_state.set(MouseState::Grabbed);
        return;
    }

    for (_, mut cursor_options) in &mut primary_windows {
        set_cursor_free(&mut cursor_options);
    }
    next_mouse_state.set(MouseState::Free);
}

fn leave_gameplay_cursor(
    mut primary_windows: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut next_mouse_state: ResMut<NextState<MouseState>>,
) {
    // Releasing directly here avoids waiting an extra state-transition frame
    // before the OS cursor becomes usable by the pause menu.
    for mut cursor_options in &mut primary_windows {
        set_cursor_free(&mut cursor_options);
    }
    next_mouse_state.set(MouseState::Free);
}

fn release_unfocused_cursor(
    mut primary_windows: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    mouse_state: Res<State<MouseState>>,
    mut next_mouse_state: ResMut<NextState<MouseState>>,
) {
    let should_release = match primary_windows.single() {
        Ok((window, cursor_options)) => {
            !window.focused
                && (*mouse_state.get() != MouseState::Free
                    || cursor_options.grab_mode != CursorGrabMode::None)
        }
        // With no unambiguous primary window, shared gameplay mouse state must
        // never claim that input is safely captured.
        Err(_) => !primary_windows.is_empty() || *mouse_state.get() != MouseState::Free,
    };
    if !should_release {
        return;
    }

    for (_, mut cursor_options) in &mut primary_windows {
        set_cursor_free(&mut cursor_options);
    }
    next_mouse_state.set(MouseState::Free);
}

const EPSILON: f32 = 0.01;

fn player_look(
    settings: Res<MouseSettings>,
    primary_windows: Query<&Window, With<PrimaryWindow>>,
    mouse_motion: Res<AccumulatedMouseMotion>,
    mut mouse_cam: Single<&mut Transform, With<MouseCam>>,
) {
    let Ok(window) = primary_windows.single() else {
        return;
    };
    let (mut yaw, mut pitch, _) = mouse_cam.rotation.to_euler(EulerRot::YXZ);
    let window_scale = window.height().min(window.width());
    pitch -= (settings.sensitivity * mouse_motion.delta.y * window_scale).to_radians();
    yaw -= (settings.sensitivity * mouse_motion.delta.x * window_scale).to_radians();

    pitch = pitch.clamp(-PI / 2.0 + EPSILON, PI / 2.0 - EPSILON);

    mouse_cam.rotation =
        Quat::from_axis_angle(Vec3::Y, yaw) * Quat::from_axis_angle(Vec3::X, pitch);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::state::app::StatesPlugin;

    #[test]
    fn gameplay_input_requires_playing_focused_and_actually_grabbed() {
        assert!(gameplay_input_is_active(
            GameState::Playing,
            MouseState::Grabbed,
            true,
            CursorGrabMode::Locked,
            false,
        ));

        for (game_state, mouse_state, focused, grab_mode, pause_requested) in [
            (
                GameState::Paused,
                MouseState::Grabbed,
                true,
                CursorGrabMode::Locked,
                false,
            ),
            (
                GameState::Playing,
                MouseState::Free,
                true,
                CursorGrabMode::Locked,
                false,
            ),
            (
                GameState::Playing,
                MouseState::Grabbed,
                false,
                CursorGrabMode::Locked,
                false,
            ),
            (
                GameState::Playing,
                MouseState::Grabbed,
                true,
                CursorGrabMode::None,
                false,
            ),
            (
                GameState::Playing,
                MouseState::Grabbed,
                true,
                CursorGrabMode::Locked,
                true,
            ),
        ] {
            assert!(!gameplay_input_is_active(
                game_state,
                mouse_state,
                focused,
                grab_mode,
                pause_requested,
            ));
        }
    }

    #[test]
    fn multiple_primary_windows_are_forcibly_released() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin))
            .init_state::<MouseState>()
            .add_systems(Update, release_unfocused_cursor);

        for _ in 0..2 {
            app.world_mut().spawn((
                Window::default(),
                CursorOptions {
                    grab_mode: CursorGrabMode::Locked,
                    visible: false,
                    ..default()
                },
                PrimaryWindow,
            ));
        }

        app.update();

        let mut cursors = app.world_mut().query::<&CursorOptions>();
        assert!(
            cursors
                .iter(app.world())
                .all(|cursor| cursor.grab_mode == CursorGrabMode::None && cursor.visible)
        );
    }
}
