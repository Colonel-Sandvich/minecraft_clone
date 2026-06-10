mod block;
mod chunk;
mod dimension;
mod game_state;
mod light;
mod mob;
mod player;
mod quad;
mod textures;
mod ui;
mod util;

use std::time::Duration;

use avian3d::PhysicsPlugins;
use bevy::{
    diagnostic::FrameTimeDiagnosticsPlugin, input::common_conditions::input_toggle_active,
    prelude::*,
};
use bevy_framepace::FramepacePlugin;
use bevy_inspector_egui::{bevy_egui::EguiPlugin, quick::WorldInspectorPlugin};
use block::BlockPlugin;
use chunk::ChunkPlugin;
use dimension::DimensionPlugin;
use game_state::GameStatePlugin;
use light::LightPlugin;
use mob::MobControllerPlugin;
use player::PlayerPlugin;
use textures::BlockTextureAtlasPlugin;
use ui::UIPlugin;

const FIXED_TICK_RATE_HZ: f64 = 20.0;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
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
        .add_plugins(DimensionPlugin)
        .add_plugins(ChunkPlugin)
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
        .add_plugins(FramepacePlugin)
        // .insert_resource(FramepaceSettings {
        //     limiter: Limiter::from_framerate(60.0),
        // })
        .run();
}
