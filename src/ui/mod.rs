mod crosshair;
mod fps;

use bevy::prelude::*;
use crosshair::CrosshairPlugin;
use fps::FpsPlugin;

pub struct UIPlugin;

impl Plugin for UIPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(CrosshairPlugin);
        app.add_plugins(FpsPlugin);
    }
}
