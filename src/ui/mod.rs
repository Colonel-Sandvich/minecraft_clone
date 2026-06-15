mod crosshair;
mod debug;
mod hotbar;

use bevy::prelude::*;
use crosshair::CrosshairPlugin;
use debug::DebugPlugin;
use hotbar::HotbarPlugin;

pub use hotbar::Hotbar;

pub struct UIPlugin;

impl Plugin for UIPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(CrosshairPlugin);
        app.add_plugins(DebugPlugin);
        app.add_plugins(HotbarPlugin);
    }
}
