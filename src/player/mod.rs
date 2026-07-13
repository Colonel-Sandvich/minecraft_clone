pub mod cam;
pub mod control;
pub mod inspector;
pub mod interaction;
mod persistence;
pub mod spawn;

use bevy::prelude::*;
use cam::PlayerCamPlugin;
use control::ControlPlayerPlugin;
use inspector::InspectorPlugin;
use interaction::BlockInteractionPlugin;
use persistence::PlayerPersistencePlugin;
use spawn::SpawnPlayerPlugin;

use crate::world::DimensionId;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ControlPlayerPlugin,
            PlayerCamPlugin,
            SpawnPlayerPlugin,
            PlayerPersistencePlugin,
            BlockInteractionPlugin,
            InspectorPlugin,
        ));
    }
}

pub const PLAYER_HEIGHT: f32 = 1.8;
pub const PLAYER_WIDTH: f32 = 0.6;
pub const PLAYER_LENGTH: f32 = PLAYER_WIDTH;

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerId(i64);

impl PlayerId {
    /// Stable identity for the local player in the initial single-player world.
    pub const LOCAL: Self = Self(1);

    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

impl Default for PlayerId {
    fn default() -> Self {
        Self::LOCAL
    }
}

#[derive(Component, Default)]
#[require(Name::new("Player"))]
pub struct Player {
    pub id: PlayerId,
    pub gamemode: GameMode,
}

/// Logical dimension membership for actors that must outlive runtime roots.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerDimension(DimensionId);

impl PlayerDimension {
    pub const fn new(id: DimensionId) -> Self {
        Self(id)
    }

    pub const fn id(self) -> DimensionId {
        self.0
    }
}

#[derive(Default)]
pub enum GameMode {
    Survival,
    #[default]
    Creative,
    Adventure,
    Spectator,
}
