//! Keyboard chords for Bevy's render debug overlay.
//!
//! The plugin's own F1/F2 shortcuts stay disabled. Holding [`CHORD_MODIFIER`]
//! turns `1` into the mode cycle and `2` into the opacity cycle.

use bevy::prelude::*;
use bevy_dev_tools::render_debug::{
    RenderDebugOverlayEvent, RenderDebugOverlayKeybindings, RenderDebugOverlayPlugin,
};

const CHORD_MODIFIER: KeyCode = KeyCode::KeyT;
const CYCLE_MODE_KEY: KeyCode = KeyCode::Digit1;
const CYCLE_OPACITY_KEY: KeyCode = KeyCode::Digit2;

pub(super) struct RenderDebugOverlayControlsPlugin;

impl Plugin for RenderDebugOverlayControlsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(RenderDebugOverlayPlugin)
            .insert_resource(RenderDebugOverlayKeybindings {
                enable_keybindings: false,
                ..default()
            })
            .add_systems(Update, handle_render_debug_overlay_chords);
    }
}

/// Whether the overlay chord modifier is currently held.
pub(super) fn chord_modifier_held(keyboard: &ButtonInput<KeyCode>) -> bool {
    keyboard.pressed(CHORD_MODIFIER)
}

fn chord_pressed(keyboard: &ButtonInput<KeyCode>, key: KeyCode) -> bool {
    chord_modifier_held(keyboard) && keyboard.just_pressed(key)
}

fn handle_render_debug_overlay_chords(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut events: MessageWriter<RenderDebugOverlayEvent>,
) {
    if chord_pressed(&keyboard, CYCLE_MODE_KEY) {
        events.write(RenderDebugOverlayEvent::CycleMode);
    }
    if chord_pressed(&keyboard, CYCLE_OPACITY_KEY) {
        events.write(RenderDebugOverlayEvent::CycleOpacity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls_app() -> App {
        let mut app = App::new();
        app.insert_resource(ButtonInput::<KeyCode>::default())
            .add_message::<RenderDebugOverlayEvent>()
            .add_systems(Update, handle_render_debug_overlay_chords);
        app
    }

    fn press(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
    }

    fn run_frame(app: &mut App) {
        app.world_mut().run_schedule(Update);
    }

    fn emitted(app: &App) -> Vec<RenderDebugOverlayEvent> {
        app.world()
            .resource::<Messages<RenderDebugOverlayEvent>>()
            .iter_current_update_messages()
            .copied()
            .collect()
    }

    #[test]
    fn t_plus_digit1_cycles_the_mode() {
        let mut app = controls_app();
        press(&mut app, CHORD_MODIFIER);
        press(&mut app, CYCLE_MODE_KEY);
        run_frame(&mut app);

        assert_eq!(emitted(&app), vec![RenderDebugOverlayEvent::CycleMode]);
    }

    #[test]
    fn t_plus_digit2_cycles_the_opacity() {
        let mut app = controls_app();
        press(&mut app, CHORD_MODIFIER);
        press(&mut app, CYCLE_OPACITY_KEY);
        run_frame(&mut app);

        assert_eq!(emitted(&app), vec![RenderDebugOverlayEvent::CycleOpacity]);
    }

    #[test]
    fn digit1_alone_does_nothing() {
        let mut app = controls_app();
        press(&mut app, CYCLE_MODE_KEY);
        run_frame(&mut app);

        assert!(emitted(&app).is_empty());
    }

    #[test]
    fn t_alone_does_nothing() {
        let mut app = controls_app();
        press(&mut app, CHORD_MODIFIER);
        run_frame(&mut app);

        assert!(emitted(&app).is_empty());
    }

    #[test]
    fn released_modifier_does_not_arm_the_chord() {
        let mut app = controls_app();
        {
            let mut keyboard = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keyboard.press(CHORD_MODIFIER);
            keyboard.release(CHORD_MODIFIER);
        }
        press(&mut app, CYCLE_MODE_KEY);
        run_frame(&mut app);

        assert!(emitted(&app).is_empty());
    }

    #[test]
    fn f1_and_f2_have_no_custom_binding() {
        let mut app = controls_app();
        press(&mut app, KeyCode::F1);
        press(&mut app, KeyCode::F2);
        run_frame(&mut app);

        assert!(emitted(&app).is_empty());
    }
}
