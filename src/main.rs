mod block;
mod chunk;
mod dimension;
mod light;
mod mesh;
mod player;
mod quad;
mod textures;
mod ui;

use bevy::{
    diagnostic::{FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin},
    prelude::*,
};
use bevy_editor_pls::prelude::*;
use block::BlockPlugin;
use chunk::ChunkPlugin;
use dimension::DimensionPlugin;
use light::LightPlugin;
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
        .add_plugins(EditorPlugin::default())
        .insert_resource(ClearColor(Color::hex("74b3ff").unwrap()))
        .add_plugins(LightPlugin)
        .add_plugins(PlayerPlugin)
        .add_plugins(BlockPlugin)
        .add_plugins(BlockTextureAtlasPlugin)
        .add_plugins(DimensionPlugin)
        .add_plugins(ChunkPlugin)
        .add_plugins(UIPlugin)
        .add_plugins((
            FrameTimeDiagnosticsPlugin,
            // Adds a system that prints diagnostics to the console
            LogDiagnosticsPlugin::default(),
            // Any plugin can register diagnostics. Uncomment this to add an entity count diagnostics:
            bevy::diagnostic::EntityCountDiagnosticsPlugin,
            // Uncomment this to add system info diagnostics:
            bevy::diagnostic::SystemInformationDiagnosticsPlugin,
        ))
        .run();
}
