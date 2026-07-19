use std::collections::VecDeque;

use bevy::{input::InputSystems, prelude::*};

use crate::{
    game_state::GameState,
    item::{DropItemRequest, ItemStack},
    player::{
        cam::{MouseCam, MouseState, gameplay_input_active},
        control::KeyBindings,
    },
    ui::Hotbar,
};

/// Bridges render-frame device input into fixed-tick gameplay requests.
pub struct GameInputPlugin;

impl Plugin for GameInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameInputs>()
            .init_resource::<KeyBindings>()
            .add_message::<DropItemRequest>()
            .configure_sets(
                PreUpdate,
                InputCollectionSystems::Collect.after(InputSystems),
            )
            .configure_sets(FixedUpdate, GameActionSystems)
            .add_systems(
                PreUpdate,
                collect_game_inputs
                    .in_set(InputCollectionSystems::Collect)
                    .run_if(gameplay_input_active),
            )
            .add_systems(
                FixedPreUpdate,
                dispatch_game_inputs
                    .in_set(FixedInputSystems::Dispatch)
                    .run_if(gameplay_input_active),
            )
            .add_systems(OnEnter(GameState::Playing), clear_pending_game_inputs)
            .add_systems(OnExit(GameState::Playing), clear_pending_game_inputs)
            .add_systems(OnEnter(MouseState::Free), clear_pending_game_inputs);
    }
}

/// Variable-timestep input collection, after Bevy has processed device events.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InputCollectionSystems {
    Collect,
}

/// Fixed-timestep promotion of buffered input into typed gameplay messages.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FixedInputSystems {
    Dispatch,
}

/// Gameplay requests that must be applied before the physics step.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GameActionSystems;

/// Semantic input that must survive render frames with no fixed tick.
#[derive(Resource, Default)]
pub struct GameInputs {
    pending_item_drops: VecDeque<DropItemInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DropItemInput;

fn collect_game_inputs(
    keyboard: Res<ButtonInput<KeyCode>>,
    bindings: Res<KeyBindings>,
    mut game_inputs: ResMut<GameInputs>,
) {
    if keyboard.just_pressed(bindings.drop_item) {
        game_inputs.pending_item_drops.push_back(DropItemInput);
    }
}

fn dispatch_game_inputs(
    mut game_inputs: ResMut<GameInputs>,
    hotbar: Res<Hotbar>,
    camera: Single<&Transform, With<MouseCam>>,
    mut drop_requests: MessageWriter<DropItemRequest>,
) {
    // Dropping is rate-limited to one request per simulation tick. Keeping a
    // FIFO preserves every distinct press and its ordering across render-only
    // frames.
    if game_inputs.pending_item_drops.pop_front().is_some()
        && let Some(item) = hotbar.selected_item()
    {
        drop_requests.write(DropItemRequest {
            stack: ItemStack::one(item),
            look_direction: *camera.forward(),
        });
    }
}

fn clear_pending_game_inputs(mut game_inputs: ResMut<GameInputs>) {
    game_inputs.pending_item_drops.clear();
}

/// Detects when a key is pressed twice within a threshold.
///
/// Intended for use as `Local<DoubleTap>` in a single system.
/// If multiple systems need to coordinate double-tap on the same key,
/// register a `ModifierCombo` resource instead.
pub struct DoubleTap {
    threshold: f32,
    last_tap: f32,
}

impl DoubleTap {
    pub fn new(threshold_secs: f32) -> Self {
        Self {
            threshold: threshold_secs,
            last_tap: 0.0,
        }
    }

    /// Returns true once when `key` is double-tapped (pressed twice within the threshold).
    pub fn just_double_tapped(
        &mut self,
        key: KeyCode,
        keys: &ButtonInput<KeyCode>,
        time: &Time,
    ) -> bool {
        if keys.just_pressed(key) {
            let now = time.elapsed_secs();
            if self.last_tap > 0.0 && now - self.last_tap < self.threshold {
                self.last_tap = 0.0;
                return true;
            }
            self.last_tap = now;
        }
        false
    }
}

impl Default for DoubleTap {
    fn default() -> Self {
        Self::new(0.3)
    }
}

/// Tracks whether a modifier+key combo was used while a modifier key was held,
/// so the modifier's solo action can be deferred until release and skipped if
/// any combo fired in between.
///
/// Use as a `Resource`. Multiple systems share the same instance:
/// - Combo systems call [`mark_combo`](ModifierCombo::mark_combo) when their combo fires.
/// - The solo system calls [`check_solo`](ModifierCombo::check_solo) when the modifier is released.
#[derive(Resource, Default)]
pub struct ModifierCombo {
    combo_used: bool,
}

impl ModifierCombo {
    /// Call when the modifier key is released.
    /// Returns true if the solo action should fire (no combo was used).
    /// Resets internal state after the call.
    pub fn check_solo(&mut self) -> bool {
        let fired = !self.combo_used;
        self.combo_used = false;
        fired
    }

    /// Call when a combo (modifier + action key) fires.
    pub fn mark_combo(&mut self) {
        self.combo_used = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct ObservedDrops(Vec<DropItemRequest>);

    fn count_drop_requests(
        mut requests: MessageReader<DropItemRequest>,
        mut observed: ResMut<ObservedDrops>,
    ) {
        observed.0.extend(requests.read().copied());
    }

    fn input_app() -> App {
        let mut app = App::new();
        app.init_resource::<GameInputs>()
            .init_resource::<KeyBindings>()
            .init_resource::<Hotbar>()
            .init_resource::<ObservedDrops>()
            .insert_resource(ButtonInput::<KeyCode>::default())
            .add_message::<DropItemRequest>()
            .add_systems(PreUpdate, collect_game_inputs)
            .add_systems(FixedPreUpdate, dispatch_game_inputs)
            .add_systems(FixedUpdate, count_drop_requests);
        app.world_mut().spawn((MouseCam, Transform::default()));
        app
    }

    fn set_camera_direction(app: &mut App, direction: Vec3) {
        let mut cameras = app
            .world_mut()
            .query_filtered::<&mut Transform, With<MouseCam>>();
        *cameras.single_mut(app.world_mut()).unwrap() =
            Transform::default().looking_to(direction, Vec3::Y);
    }

    fn collect_drop_press(app: &mut App) {
        let drop_key = app.world().resource::<KeyBindings>().drop_item;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(drop_key);
        app.world_mut().run_schedule(PreUpdate);

        let mut keyboard = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keyboard.release(drop_key);
        keyboard.clear();
    }

    #[test]
    fn queued_drop_presses_dispatch_fifo_at_one_per_fixed_tick() {
        let mut app = input_app();

        // Two complete render-frame presses occur before any fixed tick.
        collect_drop_press(&mut app);
        collect_drop_press(&mut app);

        app.world_mut().resource_mut::<Hotbar>().selected = 2;
        set_camera_direction(&mut app, Vec3::NEG_Z);
        app.world_mut().run_schedule(FixedPreUpdate);
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(app.world().resource::<ObservedDrops>().0.len(), 1);
        let first = app.world().resource::<ObservedDrops>().0[0];
        assert_eq!(first.stack.item, crate::item::Item::Sand);
        assert!(first.look_direction.abs_diff_eq(Vec3::NEG_Z, 1e-6));

        app.world_mut().resource_mut::<Hotbar>().selected = 1;
        set_camera_direction(&mut app, Vec3::X);
        app.world_mut().run_schedule(FixedPreUpdate);
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(app.world().resource::<ObservedDrops>().0.len(), 2);
        let second = app.world().resource::<ObservedDrops>().0[1];
        assert_eq!(second.stack.item, crate::item::Item::Stone);
        assert!(second.look_direction.abs_diff_eq(Vec3::X, 1e-6));

        app.world_mut().run_schedule(FixedPreUpdate);
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(app.world().resource::<ObservedDrops>().0.len(), 2);
    }

    #[test]
    fn clearing_game_inputs_discards_buffered_actions() {
        let mut app = input_app();
        app.add_systems(Update, clear_pending_game_inputs);

        collect_drop_press(&mut app);
        app.world_mut().run_schedule(Update);
        app.world_mut().run_schedule(FixedPreUpdate);
        app.world_mut().run_schedule(FixedUpdate);

        assert!(app.world().resource::<ObservedDrops>().0.is_empty());
    }
}
