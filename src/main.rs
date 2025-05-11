mod block;
mod chunk;
mod dimension;
mod light;
mod mob;
mod player;
mod quad;
mod textures;
mod ui;
mod util;

use avian3d::{
    PhysicsPlugins, debug_render::PhysicsDebugPlugin, prelude::PhysicsInterpolationPlugin,
};
use bevy::{diagnostic::FrameTimeDiagnosticsPlugin, prelude::*};
use bevy_inspector_egui::{bevy_egui::EguiPlugin, quick::WorldInspectorPlugin};
use block::BlockPlugin;
use chunk::ChunkPlugin;
use dimension::DimensionPlugin;
use light::LightPlugin;
use mob::MobControllerPlugin;
use player::PlayerPlugin;
use textures::BlockTextureAtlasPlugin;
use ui::UIPlugin;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Minecraft".to_string(),
                ..default()
            }),
            ..default()
        }))
        // .add_plugins(EditorPlugin::default())
        .add_plugins(EguiPlugin {
            enable_multipass_for_primary_context: true,
        })
        .insert_resource(ClearColor(Srgba::hex("74b3ff").unwrap().into()))
        .add_plugins(LightPlugin)
        .add_plugins(MobControllerPlugin)
        .add_plugins(PlayerPlugin)
        .add_plugins(BlockPlugin)
        .add_plugins(BlockTextureAtlasPlugin)
        .add_plugins(DimensionPlugin)
        .add_plugins(ChunkPlugin)
        .add_plugins(UIPlugin)
        .add_plugins(PhysicsPlugins::default().set(PhysicsInterpolationPlugin::interpolate_all()))
        .add_plugins(PhysicsDebugPlugin::default())
        .add_plugins((
            FrameTimeDiagnosticsPlugin::default(),
            // LogDiagnosticsPlugin::default(),
            bevy::diagnostic::EntityCountDiagnosticsPlugin,
            // bevy::diagnostic::SystemInformationDiagnosticsPlugin,
        ))
        .add_plugins(WorldInspectorPlugin::new())
        // .add_plugins(FramepacePlugin)
        // .insert_resource(FramepaceSettings {
        //     limiter: Limiter::from_framerate(60.0),
        // })
        // .insert_resource(Time::<Fixed>::from_hz(20.0))
        // ????
        .run();
}
