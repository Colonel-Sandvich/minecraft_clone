mod crosshair;
mod debug;
mod game_test;
mod hotbar;
mod inventory;
mod item_stack;
mod pause_menu;
mod render_debug;
mod settings;
mod tick_control;

use bevy::prelude::*;

pub(super) mod z {
    pub const HUD: i32 = 0;
    pub const PAUSE_MENU: i32 = 1_000;
    pub const INVENTORY: i32 = 2_000;
    pub const INVENTORY_CURSOR: i32 = 2_100;
}
#[cfg(debug_assertions)]
use bevy_dev_tools::diagnostics_overlay::{DiagnosticsOverlay, DiagnosticsOverlayPlugin};
use crosshair::CrosshairPlugin;
use debug::DebugPlugin;
use game_test::GameTestOverlayPlugin;
use hotbar::HotbarPlugin;
use inventory::InventoryUiPlugin;
use pause_menu::PauseMenuPlugin;
use render_debug::RenderDebugOverlayControlsPlugin;
use settings::UiSettingsPlugin;
use tick_control::TickControlOverlayPlugin;

pub use hotbar::Hotbar;
pub(crate) use inventory::InventoryUi;
pub use settings::UiSettings;

pub struct UIPlugin;

impl Plugin for UIPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(CrosshairPlugin);
        app.add_plugins(DebugPlugin);
        app.add_plugins(GameTestOverlayPlugin);
        app.add_plugins(HotbarPlugin);
        app.add_plugins(InventoryUiPlugin);
        app.add_plugins(PauseMenuPlugin);
        app.add_plugins(RenderDebugOverlayControlsPlugin);
        app.add_plugins(TickControlOverlayPlugin);
        app.add_plugins(UiSettingsPlugin);
        #[cfg(debug_assertions)]
        app.add_plugins(DiagnosticsOverlayPlugin)
            .add_systems(Startup, spawn_diagnostics_overlay);
    }
}

#[cfg(debug_assertions)]
fn spawn_diagnostics_overlay(mut commands: Commands) {
    commands.spawn((
        DiagnosticsOverlay::fps(),
        Pickable::IGNORE,
        GlobalZIndex(z::HUD),
    ));
}
