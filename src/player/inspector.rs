use bevy::{input::common_conditions::input_toggle_active, prelude::*};
use bevy_inspector_egui::quick::FilterQueryInspectorPlugin;

use super::Player;

pub struct InspectorPlugin;

impl Plugin for InspectorPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(
            FilterQueryInspectorPlugin::<With<Player>>::default()
                .run_if(input_toggle_active(false, KeyCode::F3)),
        );
    }
}
