pub mod crosshair;

use bevy::prelude::*;
use crosshair::CrosshairPlugin;

pub struct UIPlugin;

impl Plugin for UIPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(CrosshairPlugin);
    }
}
