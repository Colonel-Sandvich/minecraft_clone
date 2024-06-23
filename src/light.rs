use bevy::prelude::*;

pub struct LightPlugin;

impl Plugin for LightPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_point_light);
    }
}

fn spawn_point_light(mut commands: Commands) {
    commands.spawn(DirectionalLightBundle {
        directional_light: DirectionalLight {
            illuminance: light_consts::lux::AMBIENT_DAYLIGHT,
            // shadows_enabled: true,
            ..default()
        },
        transform: Transform {
            rotation: Quat::from_rotation_x(-2.0),
            ..default()
        },
        ..default()
    });
}
