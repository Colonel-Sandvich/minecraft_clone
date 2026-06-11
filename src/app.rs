use std::time::Duration;

use avian3d::PhysicsPlugins;
use bevy::{
    diagnostic::FrameTimeDiagnosticsPlugin, input::common_conditions::input_toggle_active,
    prelude::*,
};
use bevy_framepace::FramepacePlugin;
use bevy_inspector_egui::{bevy_egui::EguiPlugin, quick::WorldInspectorPlugin};

use crate::{
    block::BlockPlugin,
    game_state::GameStatePlugin,
    light::LightPlugin,
    mob::MobControllerPlugin,
    player::PlayerPlugin,
    textures::BlockTextureAtlasPlugin,
    ui::UIPlugin,
    world::{WorldConfig, WorldMetadata, WorldPlugin},
};

pub const FIXED_TICK_RATE_HZ: f64 = 20.0;

pub struct AppPlugin;

impl Plugin for AppPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Minecraft".to_string(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(EguiPlugin::default())
        .insert_resource(ClearColor(Srgba::hex("74b3ff").unwrap().into()))
        .add_plugins(GameStatePlugin)
        .add_plugins(LightPlugin)
        .add_plugins(MobControllerPlugin)
        .add_plugins(PlayerPlugin)
        .add_plugins(BlockPlugin)
        .add_plugins(BlockTextureAtlasPlugin)
        .insert_resource(WorldConfig::development_sqlite(WorldMetadata::default()))
        .add_plugins(WorldPlugin)
        .add_plugins(UIPlugin)
        .insert_resource(Time::<Fixed>::from_hz(FIXED_TICK_RATE_HZ))
        .insert_resource(Time::<Virtual>::from_max_delta(Duration::from_secs_f64(
            1.0 / FIXED_TICK_RATE_HZ,
        )))
        .add_plugins(PhysicsPlugins::default())
        // .add_plugins(PhysicsDebugPlugin::default())
        .add_plugins((
            FrameTimeDiagnosticsPlugin::default(),
            // LogDiagnosticsPlugin::default(),
            // bevy::diagnostic::EntityCountDiagnosticsPlugin,
            // bevy::diagnostic::SystemInformationDiagnosticsPlugin,
        ))
        .add_plugins(WorldInspectorPlugin::new().run_if(input_toggle_active(false, KeyCode::F5)))
        .add_plugins(FramepacePlugin);
        // .insert_resource(FramepaceSettings {
        //     limiter: Limiter::from_framerate(60.0),
        // })
    }
}

pub fn run() {
    App::new().add_plugins(AppPlugin).run();
}
