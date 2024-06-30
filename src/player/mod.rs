pub mod cam;
mod click_handler;
mod control;
pub mod fly_controller;
pub mod laser;
pub mod spawn;

use bevy::prelude::*;
use bevy_rapier3d::geometry::Collider;
use cam::{MouseSettings, PlayerCamPlugin};
use click_handler::ClickHandlerPlugin;
use control::ControlPlayerPlugin;
use fly_controller::{FlyControllerPlugin, MovementSettings};
use laser::LaserPlugin;
use spawn::SpawnPlayerPlugin;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ControlPlayerPlugin);
        app.add_plugins(PlayerCamPlugin);
        app.add_plugins(SpawnPlayerPlugin);
        app.add_plugins(FlyControllerPlugin);
        app.add_plugins(LaserPlugin);
        app.add_plugins(ClickHandlerPlugin);

        app.insert_resource(MovementSettings { speed: 8.0 })
            .insert_resource(MouseSettings {
                sensitivity: 0.00007,
                fov: 100.0,
            });
    }
}

pub const PLAYER_HEIGHT: f32 = 1.8;
pub const PLAYER_WIDTH: f32 = 0.6;
pub const PLAYER_LENGTH: f32 = PLAYER_WIDTH;

#[derive(Component, Default)]
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

pub fn make_collider() -> Collider {
    Collider::cuboid(PLAYER_LENGTH / 2.0, PLAYER_HEIGHT / 2.0, PLAYER_WIDTH / 2.0)
}
