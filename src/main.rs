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

use bevy::{diagnostic::FrameTimeDiagnosticsPlugin, prelude::*};
use bevy_editor_pls::prelude::*;
use bevy_framepace::{FramepacePlugin, FramepaceSettings, Limiter};
use bevy_rapier3d::prelude::*;
use block::BlockPlugin;
use chunk::ChunkPlugin;
use dimension::DimensionPlugin;
use light::LightPlugin;
use mob::MobPlugin;
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
        .insert_resource(ClearColor(Srgba::hex("74b3ff").unwrap().into()))
        .add_plugins(LightPlugin)
        .add_plugins(MobPlugin)
        .add_plugins(PlayerPlugin)
        .add_plugins(BlockPlugin)
        .add_plugins(BlockTextureAtlasPlugin)
        .add_plugins(DimensionPlugin)
        .add_plugins(ChunkPlugin)
        .add_plugins(UIPlugin)
        .add_plugins(RapierPhysicsPlugin::<NoUserData>::default())
        .add_plugins(RapierDebugRenderPlugin::default().disabled())
        .add_plugins((
            FrameTimeDiagnosticsPlugin,
            // Adds a system that prints diagnostics to the console
            // LogDiagnosticsPlugin::default(),
            // Any plugin can register diagnostics. Uncomment this to add an entity count diagnostics:
            bevy::diagnostic::EntityCountDiagnosticsPlugin,
            // Uncomment this to add system info diagnostics:
            bevy::diagnostic::SystemInformationDiagnosticsPlugin,
        ))
        .insert_resource(Msaa::Off)
        .add_plugins(FramepacePlugin)
        // .insert_resource(FramepaceSettings {
        //     limiter: Limiter::from_framerate(60.0),
        // })
        // .insert_resource(Time::<Fixed>::from_hz(20.0))
        // ????
        .run();
}
