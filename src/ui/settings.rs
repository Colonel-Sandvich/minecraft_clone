//! User-facing UI settings and their root font-size (rem) bridge.
//!
//! Bevy resolves `Val::Rem` and `FontSize::Rem` against the global [`RemSize`]
//! resource. The UI is authored at a 1.0 scale where one rem is
//! [`BASE_REM_SIZE_PX`] logical pixels, so changing the UI scale only requires
//! updating [`RemSize`]; every rem-sized node and font follows in layout.

use bevy::{prelude::*, text::RemSize};
use bevy_settings::{ReflectSettingsGroup, SettingsGroup};

/// The rem size, in logical pixels, used at a UI scale of 1.0.
///
/// This is Bevy's [`RemSize`] default. UI dimensions are written as
/// `px / BASE_REM_SIZE_PX` rem so the existing 1.0-scale look is preserved.
pub(super) const BASE_REM_SIZE_PX: f32 = 20.0;

/// Selectable UI scales, from the smallest to the largest.
pub(super) const UI_SCALE_STEPS: [f32; 4] = [0.75, 1.0, 1.25, 1.5];

pub(super) const MIN_UI_SCALE: f32 = UI_SCALE_STEPS[0];
pub(super) const MAX_UI_SCALE: f32 = UI_SCALE_STEPS[UI_SCALE_STEPS.len() - 1];

pub(super) struct UiSettingsPlugin;

impl Plugin for UiSettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiSettings>()
            .add_systems(Startup, sync_rem_size)
            .add_systems(Update, sync_rem_size);
    }
}

/// Persisted user-facing UI preferences.
#[derive(Resource, SettingsGroup, Reflect, Debug, Clone, Copy, PartialEq)]
#[reflect(Resource, SettingsGroup, Default)]
pub struct UiSettings {
    /// Multiplier applied to every rem-sized UI dimension and font.
    pub scale: f32,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self { scale: 1.0 }
    }
}

impl UiSettings {
    /// The clamped scale used by the UI and by [`RemSize`].
    pub(super) fn effective_scale(self) -> f32 {
        clamp_ui_scale(self.scale)
    }

    /// The global rem size, in logical pixels, for this scale.
    pub(super) fn rem_size(self) -> f32 {
        BASE_REM_SIZE_PX * self.effective_scale()
    }

    /// The human-readable percentage shown in the options screen.
    pub(super) fn scale_percent(self) -> u32 {
        (self.effective_scale() * 100.0).round() as u32
    }

    /// Advances to the next preset scale, wrapping at the maximum.
    pub(super) fn next_scale(mut self) -> Self {
        self.scale = next_ui_scale(self.scale);
        self
    }
}

/// Clamps a stored or hand-edited scale into the supported range.
///
/// Non-finite values (for example from a corrupt settings file) fall back to
/// the default instead of poisoning layout values.
pub(super) fn clamp_ui_scale(scale: f32) -> f32 {
    if !scale.is_finite() {
        return 1.0;
    }
    scale.clamp(MIN_UI_SCALE, MAX_UI_SCALE)
}

/// Returns the first preset above `scale`, wrapping to the minimum.
pub(super) fn next_ui_scale(scale: f32) -> f32 {
    if !scale.is_finite() || scale < MIN_UI_SCALE {
        return MIN_UI_SCALE;
    }
    let scale = scale.min(MAX_UI_SCALE);
    UI_SCALE_STEPS
        .into_iter()
        .find(|step| *step > scale + f32::EPSILON)
        .unwrap_or(MIN_UI_SCALE)
}

/// Converts a logical-pixel dimension authored at 1.0 scale into rem.
pub(super) fn rem(px: f32) -> Val {
    Val::Rem(px / BASE_REM_SIZE_PX)
}

/// Converts a logical-pixel font size authored at 1.0 scale into rem.
pub(super) fn rem_font(px: f32) -> FontSize {
    FontSize::Rem(px / BASE_REM_SIZE_PX)
}

/// Keeps Bevy's global rem size in sync with the persisted setting.
fn sync_rem_size(settings: Res<UiSettings>, mut rem_size: ResMut<RemSize>) {
    let target = settings.rem_size();
    if rem_size.0 != target {
        rem_size.0 = target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_scale_is_one_and_uses_bevys_default_rem_size() {
        let settings = UiSettings::default();
        assert_eq!(settings.effective_scale(), 1.0);
        assert_eq!(settings.rem_size(), BASE_REM_SIZE_PX);
    }

    #[test]
    fn scale_is_clamped_to_the_supported_bounds() {
        assert_eq!(clamp_ui_scale(0.1), MIN_UI_SCALE);
        assert_eq!(clamp_ui_scale(9.0), MAX_UI_SCALE);
        assert_eq!(clamp_ui_scale(f32::NAN), 1.0);
        assert_eq!(clamp_ui_scale(f32::INFINITY), 1.0);
    }

    #[test]
    fn rem_size_tracks_the_clamped_scale() {
        let small = UiSettings { scale: 0.75 };
        let large = UiSettings { scale: 1.5 };
        let clamped = UiSettings { scale: 4.0 };

        assert_eq!(small.rem_size(), 15.0);
        assert_eq!(large.rem_size(), 30.0);
        assert_eq!(clamped.rem_size(), BASE_REM_SIZE_PX * MAX_UI_SCALE);
    }

    #[test]
    fn scale_steps_advance_and_wrap() {
        assert_eq!(next_ui_scale(0.75), 1.0);
        assert_eq!(next_ui_scale(1.0), 1.25);
        assert_eq!(next_ui_scale(1.25), 1.5);
        assert_eq!(next_ui_scale(1.5), MIN_UI_SCALE);
        assert_eq!(next_ui_scale(1.1), 1.25);
        assert_eq!(next_ui_scale(0.1), 0.75);
    }

    #[test]
    fn cycling_updates_the_persisted_scale() {
        let mut settings = UiSettings::default();
        settings = settings.next_scale();
        assert_eq!(settings.scale, 1.25);
        settings = UiSettings {
            scale: MAX_UI_SCALE,
        }
        .next_scale();
        assert_eq!(settings.scale, MIN_UI_SCALE);
    }

    #[test]
    fn percent_label_is_rounded_from_the_effective_scale() {
        assert_eq!(UiSettings { scale: 1.0 }.scale_percent(), 100);
        assert_eq!(UiSettings { scale: 1.25 }.scale_percent(), 125);
        assert_eq!(UiSettings { scale: 9.0 }.scale_percent(), 150);
    }

    #[test]
    fn px_helpers_convert_to_rem_at_the_base_size() {
        assert_eq!(rem(BASE_REM_SIZE_PX), Val::Rem(1.0));
        assert_eq!(rem(60.0), Val::Rem(3.0));
        assert_eq!(rem_font(20.0), FontSize::Rem(1.0));
        assert_eq!(rem_font(28.0), FontSize::Rem(1.4));
    }

    #[test]
    fn plugin_syncs_bevys_rem_size_when_the_scale_changes() {
        let mut app = App::new();
        app.init_resource::<RemSize>().add_plugins(UiSettingsPlugin);

        app.update();
        assert_eq!(app.world().resource::<RemSize>().0, BASE_REM_SIZE_PX);

        app.world_mut().resource_mut::<UiSettings>().scale = 1.5;
        app.update();
        assert_eq!(app.world().resource::<RemSize>().0, BASE_REM_SIZE_PX * 1.5);
    }
}
