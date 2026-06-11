mod lifecycle;
mod persistence;
mod tasks;
mod view;

use bevy::{platform::collections::HashMap, prelude::*};

use crate::game_state::{GameState, Playing};

use self::{
    lifecycle::{finish_chunk_load_tasks, maintain_chunk_view, start_chunk_load_tasks},
    persistence::{
        ChunkSaveBudget, ChunkSaveTasks, finish_chunk_save_tasks, start_chunk_save_tasks,
    },
    tasks::ChunkLoadTasks,
};
use super::{generation::WorldMetadata, storage::ChunkRepository};

pub use self::{
    tasks::{ChunkLoadBudget, ChunkSpawnBudget},
    view::{VIEW_DISTANCE, chunk_positions_in_view},
};

#[derive(Default, Component)]
pub struct Dimension {
    pub chunks: HashMap<IVec3, Entity>,
}

impl Dimension {
    pub fn chunk_entity(&self, pos: IVec3) -> Option<Entity> {
        self.chunks.get(&pos).copied()
    }

    pub fn loaded_chunk_count(&self) -> usize {
        self.chunks.len()
    }
}

pub struct DimensionPlugin;

impl Plugin for DimensionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WorldMetadata>()
            .init_resource::<ChunkRepository>()
            .init_resource::<ChunkLoadBudget>()
            .init_resource::<ChunkSpawnBudget>()
            .init_resource::<ChunkSaveBudget>()
            .init_resource::<ChunkSaveTasks>()
            .init_resource::<ChunkLoadTasks>();

        app.add_systems(
            OnEnter(GameState::GenWorld),
            (
                setup,
                maintain_chunk_view,
                start_chunk_load_tasks,
                finish_chunk_load_tasks,
                |mut game_state: ResMut<NextState<GameState>>| game_state.set(GameState::Playing),
            )
                .chain(),
        );

        app.add_systems(
            Update,
            (
                maintain_chunk_view,
                start_chunk_load_tasks,
                finish_chunk_load_tasks,
            )
                .chain()
                .in_set(Playing),
        );
        app.add_systems(
            PostUpdate,
            (finish_chunk_save_tasks, start_chunk_save_tasks)
                .chain()
                .run_if(in_state(GameState::Playing)),
        );
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Dimension::default(),
        Transform::default(),
        Visibility::default(),
        Active,
    ));
}

#[derive(Component)]
pub struct Active;
