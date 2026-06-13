mod crosshair;
mod debug;

use bevy::prelude::*;
use crosshair::CrosshairPlugin;
use debug::DebugPlugin;

pub struct UIPlugin;

impl Plugin for UIPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(CrosshairPlugin);
        app.add_plugins(DebugPlugin);
    }
}
