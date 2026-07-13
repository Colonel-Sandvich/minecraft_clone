mod crosshair;
mod debug;
mod hotbar;
mod pause_menu;

use bevy::prelude::*;
#[cfg(debug_assertions)]
use bevy_dev_tools::diagnostics_overlay::{DiagnosticsOverlay, DiagnosticsOverlayPlugin};
use crosshair::CrosshairPlugin;
use debug::DebugPlugin;
use hotbar::HotbarPlugin;
use pause_menu::PauseMenuPlugin;

pub use hotbar::Hotbar;

pub struct UIPlugin;

impl Plugin for UIPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(CrosshairPlugin);
        app.add_plugins(DebugPlugin);
        app.add_plugins(HotbarPlugin);
        app.add_plugins(PauseMenuPlugin);
        #[cfg(debug_assertions)]
        app.add_plugins(DiagnosticsOverlayPlugin)
            .add_systems(Startup, spawn_diagnostics_overlay);
    }
}

#[cfg(debug_assertions)]
fn spawn_diagnostics_overlay(mut commands: Commands) {
    commands.spawn(DiagnosticsOverlay::fps());
}
