use std::time::Duration;

use bevy::{input::InputSystems, prelude::*, window::PrimaryWindow};

pub struct GameStatePlugin;

impl Plugin for GameStatePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>()
            .add_systems(PreUpdate, request_pause_state.after(InputSystems))
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

fn request_pause_state(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    game_state: Res<State<GameState>>,
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
