use bevy::prelude::*;

pub struct LightPlugin;

const SKY_FILL_BRIGHTNESS: f32 = 500.0;

impl Plugin for LightPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GlobalAmbientLight {
            color: Color::srgb(0.78, 0.86, 1.0),
            brightness: SKY_FILL_BRIGHTNESS,
            affects_lightmapped_meshes: true,
        })
        .add_systems(Startup, spawn_sun_light);
    }
}

fn spawn_sun_light(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: light_consts::lux::AMBIENT_DAYLIGHT,
            color: Color::srgb(1.0, 0.96, 0.9),
            // Shadows are disabled because shadow-map resolution at the
            // draw distances used by this game would produce unacceptable
            // aliasing artefacts with the current Bevy renderer.
            // shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_rotation_x(-2.0)),
    ));
}
