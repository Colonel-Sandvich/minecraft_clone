pub mod cam;
pub mod click_handler;
pub mod control;
pub mod inspector;
pub mod laser;
pub mod spawn;

use bevy::prelude::*;
use cam::PlayerCamPlugin;
use click_handler::ClickHandlerPlugin;
use control::ControlPlayerPlugin;
use inspector::InspectorPlugin;
use laser::LaserPlugin;
use spawn::SpawnPlayerPlugin;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ControlPlayerPlugin,
            PlayerCamPlugin,
            SpawnPlayerPlugin,
            LaserPlugin,
            ClickHandlerPlugin,
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
