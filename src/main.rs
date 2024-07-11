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

use avian3d::{debug_render::PhysicsDebugPlugin, PhysicsPlugins};
use bevy::{diagnostic::FrameTimeDiagnosticsPlugin, prelude::*};
use bevy_inspector_egui::quick::WorldInspectorPlugin;
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
        .insert_resource(ClearColor(Srgba::hex("74b3ff").unwrap().into()))
        .add_plugins(LightPlugin)
        .add_plugins(MobControllerPlugin)
        .add_plugins(PlayerPlugin)
        .add_plugins(BlockPlugin)
        .add_plugins(BlockTextureAtlasPlugin)
        .add_plugins(DimensionPlugin)
        .add_plugins(ChunkPlugin)
        .add_plugins(UIPlugin)
        .add_plugins(PhysicsPlugins::default())
        .add_plugins(PhysicsDebugPlugin::default())
        .add_plugins((
            FrameTimeDiagnosticsPlugin,
            // Adds a system that prints diagnostics to the console
            // LogDiagnosticsPlugin::default(),
            // Any plugin can register diagnostics. Uncomment this to add an entity count diagnostics:
            bevy::diagnostic::EntityCountDiagnosticsPlugin,
            // Uncomment this to add system info diagnostics:
            // bevy::diagnostic::SystemInformationDiagnosticsPlugin,
        ))
        // .add_plugins(WorldInspectorPlugin::new())
        .insert_resource(Msaa::Off)
        // .add_plugins(FramepacePlugin)
        // .insert_resource(FramepaceSettings {
        //     limiter: Limiter::from_framerate(60.0),
        // })
        // .insert_resource(Time::<Fixed>::from_hz(20.0))
        // ????
        .run();
}

// #[derive(Component)]
// struct Creeper;

// fn spawn_creeper(
//     mut commands: Commands,
//     mut meshes: ResMut<Assets<Mesh>>,
//     mut materials: ResMut<Assets<StandardMaterial>>,
// ) {
//     commands
//         .spawn((
//             SpatialBundle {
//                 transform: Transform::from_translation(Vec3::new(
//                     5.0,
//                     CHUNK_SIZE as f32 + 3.0,
//                     5.0,
//                 )),
//                 ..default()
//             },
//             RigidBody::KinematicVelocityBased,
//             KinematicCharacterController {
//                 autostep: Some(CharacterAutostep::default()),
//                 apply_impulse_to_dynamic_bodies: true,
//                 ..default()
//             },
//             make_collider(),
//             LockedAxes::ROTATION_LOCKED,
//             GravityScale(1.0),
//             Creeper,
//         ))
//         .with_children(|p| {
//             let color: Color = bevy::color::palettes::css::LIME.into();
//             let material: StandardMaterial = color.into();
//             p.spawn(PbrBundle {
//                 mesh: meshes.add(Cuboid::new(PLAYER_WIDTH, PLAYER_HEIGHT, PLAYER_LENGTH)),
//                 material: materials.add(material),
//                 ..default()
//             });
//         });
// }
