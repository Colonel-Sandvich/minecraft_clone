pub mod block_interaction;
pub mod cam;
pub mod control;
pub mod inspector;
pub mod spawn;

use bevy::prelude::*;
use block_interaction::BlockInteractionPlugin;
use cam::PlayerCamPlugin;
use control::ControlPlayerPlugin;
use inspector::InspectorPlugin;
use spawn::SpawnPlayerPlugin;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ControlPlayerPlugin,
            PlayerCamPlugin,
            SpawnPlayerPlugin,
            BlockInteractionPlugin,
            InspectorPlugin,
        ));
    }
}

pub const PLAYER_HEIGHT: f32 = 1.8;
pub const PLAYER_WIDTH: f32 = 0.6;
pub const PLAYER_LENGTH: f32 = PLAYER_WIDTH;

#[derive(Component, Default)]
#[require(Name::new("Player"))]
pub struct Player {
    pub gamemode: GameMode,
}

#[derive(Default)]
pub enum GameMode {
    Survival,
    #[default]
    Creative,
    Adventure,
    Spectator,
}
