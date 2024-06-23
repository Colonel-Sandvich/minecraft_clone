use bevy::prelude::*;
use bevy_flycam::{FlyCam, KeyBindings, MovementSettings, NoCameraPlayerPlugin};

use crate::chunk::CHUNK_SIZE;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(NoCameraPlayerPlugin)
            .add_systems(Startup, setup)
            .insert_resource(MovementSettings {
                sensitivity: 0.00008,
                speed: 8.0,
            })
            .insert_resource(KeyBindings {
                move_descend: KeyCode::ControlLeft,
                ..default()
            });
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Camera3dBundle {
            transform: Transform::from_translation(Vec3::new(0.0, CHUNK_SIZE as f32 + 2.0, 0.0))
                .looking_to(Vec3::X, Vec3::Y),
            ..default()
        },
        FlyCam,
        IsDefaultUiCamera,
    ));
}
