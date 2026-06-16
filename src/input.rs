use bevy::prelude::*;

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
