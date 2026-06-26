use bevy::prelude::*;

pub struct GameStatePlugin;

impl Plugin for GameStatePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<GameState>();

        app.configure_sets(
            Update,
            Playing.run_if(in_state(GameState::Playing).or_else(in_state(GameState::GenWorld))),
        );
    }
}

#[derive(Default, Debug, PartialEq, Eq, Clone, Copy, Hash, States)]
pub enum GameState {
    MainMenu,
    #[default]
    GenWorld,
    Playing,
    Paused,
}

#[derive(Debug, SystemSet, Hash, PartialEq, Eq, Clone, Copy)]
pub struct Playing;
