use std::time::Duration;

use bevy::{
    input::InputSystems,
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};

use crate::ui::InventoryUi;

pub struct GameStatePlugin;

impl Plugin for GameStatePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .init_resource::<InputMode>()
            .configure_sets(
                PreUpdate,
                (
                    InputRouting::Route.after(InputSystems),
                    InputRouting::Derive.after(InputRouting::Route),
                ),
            )
            .add_systems(PreUpdate, route_back_input.in_set(InputRouting::Route))
            .add_systems(PreUpdate, update_input_mode.in_set(InputRouting::Derive))
            .add_systems(
                OnEnter(GameState::Paused),
                (pause_virtual_time, reset_gameplay_inputs),
            )
            .add_systems(OnExit(GameState::Paused), resume_virtual_time)
            // Also discard the click/key press used to leave the menu. Without
            // this, holding the resume click can immediately break a block.
            .add_systems(
                OnEnter(GameState::Playing),
                (reset_gameplay_inputs, pause_unfocused_game),
            );

        app.configure_sets(
            Update,
            Playing.run_if(in_state(GameState::Playing).or_else(in_state(GameState::GenWorld))),
        );
    }
}

#[derive(Default, Debug, PartialEq, Eq, Clone, Copy, Hash, States)]
pub enum GameState {
    MainMenu,
    #[default]
    GenWorld,
    Playing,
    Paused,
}

#[derive(Debug, SystemSet, Hash, PartialEq, Eq, Clone, Copy)]
pub struct Playing;

/// PreUpdate ordering for frame input ownership.
///
/// [`InputRouting::Route`] owns the back button and settles which screen
/// consumes it. Screens that process routing intents run between the two sets.
/// [`InputRouting::Derive`] then freezes this frame's [`InputMode`], and every
/// gameplay input system runs after it.
#[derive(Debug, SystemSet, Hash, PartialEq, Eq, Clone, Copy)]
pub enum InputRouting {
    /// Runs the back button through its priority chain exactly once per frame.
    Route,
    /// Derives [`InputMode`] after screens have settled their intent.
    Derive,
}

/// Who owns device input for the current frame.
///
/// [`update_input_mode`] derives this once per frame, after back-input routing
/// has settled, so gameplay systems make one shared decision instead of each
/// re-checking state, focus, and cursor capture. Gameplay systems must run
/// after [`InputRouting::Derive`] to see the current frame's mode.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    /// Gameplay owns the cursor and may read device input.
    Gameplay,
    /// A screen (inventory, menu) owns the cursor, or the game is not running.
    #[default]
    Ui,
    /// The pause transition is pending or the game is already paused.
    Paused,
}

impl InputMode {
    pub(crate) fn is_gameplay(self) -> bool {
        self == Self::Gameplay
    }
}

/// Gate for every gameplay input system.
///
/// Systems using this condition must be ordered after [`InputRouting::Derive`].
pub(crate) fn gameplay_input_active(input_mode: Option<Res<InputMode>>) -> bool {
    input_mode.is_some_and(|mode| mode.is_gameplay())
}

fn update_input_mode(
    game_state: Res<State<GameState>>,
    next_game_state: Res<NextState<GameState>>,
    inventory_ui: Option<Res<InventoryUi>>,
    primary_windows: Query<(&Window, &CursorOptions), With<PrimaryWindow>>,
    mut input_mode: ResMut<InputMode>,
) {
    let (window_focused, cursor_captured) =
        primary_windows
            .single()
            .map_or((false, false), |(window, cursor_options)| {
                (
                    window.focused,
                    cursor_options.grab_mode != CursorGrabMode::None,
                )
            });

    *input_mode = classify_input_mode(
        *game_state.get(),
        pause_is_pending(&next_game_state),
        inventory_ui.is_some_and(|ui| ui.is_open()),
        window_focused,
        cursor_captured,
    );
}

fn pause_is_pending(next_game_state: &NextState<GameState>) -> bool {
    matches!(
        next_game_state,
        NextState::Pending(GameState::Paused) | NextState::PendingIfDifferent(GameState::Paused)
    )
}

fn classify_input_mode(
    game_state: GameState,
    pause_pending: bool,
    screen_open: bool,
    window_focused: bool,
    cursor_captured: bool,
) -> InputMode {
    if pause_pending || game_state == GameState::Paused {
        InputMode::Paused
    } else if game_state != GameState::Playing || screen_open || !window_focused || !cursor_captured
    {
        InputMode::Ui
    } else {
        InputMode::Gameplay
    }
}

/// The single owner of the back button (Escape).
///
/// Screens take priority: a press closes the open screen instead of toggling
/// pause, so one press can never both close a screen and pause the game.
pub(crate) fn route_back_input(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    game_state: Res<State<GameState>>,
    mut inventory_ui: Option<ResMut<InventoryUi>>,
    primary_windows: Query<&Window, With<PrimaryWindow>>,
    mut next_game_state: ResMut<NextState<GameState>>,
    mut previous_focus: Local<Option<bool>>,
) {
    let window_focused = unambiguous_primary_window_is_focused(primary_windows.iter());
    let focus_just_regained = *previous_focus == Some(false) && window_focused;
    *previous_focus = Some(window_focused);

    // Losing focus always releases control to a visible pause screen. Regaining
    // focus deliberately leaves the game paused until the player resumes it.
    if *game_state.get() == GameState::Playing && !window_focused {
        next_game_state.set(GameState::Paused);
        return;
    }

    if !keys.is_some_and(|keys| keys.just_pressed(KeyCode::Escape)) || focus_just_regained {
        return;
    }

    if let Some(ui) = inventory_ui.as_mut()
        && ui.is_open()
    {
        ui.request_close();
        return;
    }

    match game_state.get() {
        GameState::Playing => next_game_state.set(GameState::Paused),
        GameState::Paused if window_focused => next_game_state.set(GameState::Playing),
        GameState::MainMenu | GameState::GenWorld | GameState::Paused => {}
    }
}

fn pause_virtual_time(mut time: ResMut<Time<Virtual>>) {
    pause_time_immediately(&mut time);
}

fn resume_virtual_time(mut time: ResMut<Time<Virtual>>) {
    time.unpause();
}

fn pause_unfocused_game(
    primary_windows: Query<&Window, With<PrimaryWindow>>,
    mut next_game_state: ResMut<NextState<GameState>>,
    mut time: ResMut<Time<Virtual>>,
) {
    if !unambiguous_primary_window_is_focused(primary_windows.iter()) {
        // Entering gameplay can race a focus change during world loading. Stop
        // virtual/fixed time immediately while the queued pause state catches up.
        pause_time_immediately(&mut time);
        next_game_state.set(GameState::Paused);
    }
}

fn unambiguous_primary_window_is_focused<'a>(
    mut primary_windows: impl Iterator<Item = &'a Window>,
) -> bool {
    match (primary_windows.next(), primary_windows.next()) {
        // Window-less test/headless apps are not treated as focus loss. More
        // than one primary window is malformed, so fail closed for gameplay.
        (None, _) => true,
        (Some(window), None) => window.focused,
        (Some(_), Some(_)) => false,
    }
}

fn pause_time_immediately(time: &mut Time<Virtual>) {
    time.pause();
    // Virtual time is advanced in First, before Escape is read in PreUpdate.
    // Clear this frame's already-computed delta so RunFixedMainLoop cannot run
    // one last simulation tick after the pause transition.
    time.advance_by(Duration::ZERO);
}

fn reset_gameplay_inputs(
    keyboard: Option<ResMut<ButtonInput<KeyCode>>>,
    mouse: Option<ResMut<ButtonInput<MouseButton>>>,
) {
    if let Some(mut keyboard) = keyboard {
        keyboard.reset_all();
    }
    if let Some(mut mouse) = mouse {
        mouse.reset_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::state::app::StatesPlugin;
    use bevy::time::TimeUpdateStrategy;

    #[derive(Resource, Default)]
    struct FixedTicks(usize);

    fn count_fixed_ticks(mut ticks: ResMut<FixedTicks>) {
        ticks.0 += 1;
    }

    fn playing_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, GameStatePlugin))
            .insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(ButtonInput::<MouseButton>::default())
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                100,
            )))
            .insert_resource(Time::<Fixed>::from_hz(20.0))
            .init_resource::<FixedTicks>()
            .add_systems(FixedUpdate, count_fixed_ticks);
        app.world_mut().spawn((Window::default(), PrimaryWindow));

        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::Playing);
        app.update();
        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::Playing
        );
        app
    }

    #[test]
    fn escape_pauses_and_resumes_virtual_time() {
        let mut app = playing_app();
        let ticks_before_pause = app.world().resource::<FixedTicks>().0;

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();

        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::Paused
        );
        assert!(app.world().resource::<Time<Virtual>>().is_paused());
        assert_eq!(app.world().resource::<FixedTicks>().0, ticks_before_pause);

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();

        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::Playing
        );
        assert!(!app.world().resource::<Time<Virtual>>().is_paused());
    }

    #[test]
    fn focus_loss_pauses_and_does_not_auto_resume() {
        let mut app = playing_app();
        let mut window_query = app
            .world_mut()
            .query_filtered::<&mut Window, With<PrimaryWindow>>();
        window_query.single_mut(app.world_mut()).unwrap().focused = false;

        app.update();
        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::Paused
        );

        window_query.single_mut(app.world_mut()).unwrap().focused = true;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        app.update();

        assert_eq!(
            *app.world().resource::<State<GameState>>().get(),
            GameState::Paused
        );
    }

    #[test]
    fn entering_playing_discards_pressed_inputs() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, StatesPlugin, GameStatePlugin))
            .insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(ButtonInput::<MouseButton>::default());

        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .resource_mut::<NextState<GameState>>()
            .set(GameState::Playing);
        app.update();

        assert!(
            app.world()
                .resource::<ButtonInput<KeyCode>>()
                .get_pressed()
                .next()
                .is_none()
        );
        assert!(
            app.world()
                .resource::<ButtonInput<MouseButton>>()
                .get_pressed()
                .next()
                .is_none()
        );
    }

    #[test]
    fn input_mode_requires_playing_focused_and_actually_captured() {
        assert_eq!(
            classify_input_mode(GameState::Playing, false, false, true, true),
            InputMode::Gameplay
        );

        for (game_state, pause_pending, screen_open, focused, captured) in [
            (GameState::Paused, false, false, true, true),
            (GameState::Playing, true, false, true, true),
            (GameState::Playing, false, true, true, true),
            (GameState::Playing, false, false, false, true),
            (GameState::Playing, false, false, true, false),
            (GameState::GenWorld, false, false, true, true),
            (GameState::MainMenu, false, false, true, true),
        ] {
            let mode =
                classify_input_mode(game_state, pause_pending, screen_open, focused, captured);
            assert!(
                !mode.is_gameplay(),
                "{mode:?} must not let {game_state:?} read gameplay input"
            );
        }

        assert_eq!(
            classify_input_mode(GameState::Playing, true, false, true, true),
            InputMode::Paused
        );
        assert_eq!(
            classify_input_mode(GameState::Playing, false, false, false, true),
            InputMode::Ui
        );
    }

    #[test]
    fn primary_window_focus_fails_closed_for_multiple_windows() {
        let focused = Window {
            focused: true,
            ..default()
        };
        let unfocused = Window {
            focused: false,
            ..default()
        };

        assert!(unambiguous_primary_window_is_focused(std::iter::empty()));
        assert!(unambiguous_primary_window_is_focused(std::iter::once(
            &focused
        )));
        assert!(!unambiguous_primary_window_is_focused(std::iter::once(
            &unfocused
        )));
        assert!(!unambiguous_primary_window_is_focused(
            [&focused, &focused].into_iter()
        ));
    }
}
