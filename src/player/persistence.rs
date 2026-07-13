use std::collections::{HashMap, HashSet};

use avian3d::prelude::Position;
use bevy::{app::AppExit, prelude::*, time::Real, window::ExitSystems};

use crate::world::storage::{ChunkRepository, StoredPlayer, StoredPlayerPosition};

use super::{Player, PlayerDimension, PlayerId};

const PLAYER_AUTOSAVE_INTERVAL_SECONDS: f32 = 5.0;

pub(super) struct PlayerPersistencePlugin;

/// Prevents a fallback spawn from overwriting a player row that failed to
/// load. A future retry flow can remove this marker after it resolves the
/// stored state.
#[derive(Component)]
pub(super) struct PlayerPersistenceDisabled;

impl Plugin for PlayerPersistencePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerSaveState>()
            .add_systems(Last, save_player_positions.after(ExitSystems));
    }
}

#[derive(Resource)]
struct PlayerSaveState {
    timer: Timer,
    attempted: HashSet<PlayerId>,
    last_saved: HashMap<PlayerId, StoredPlayerPosition>,
}

impl Default for PlayerSaveState {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(PLAYER_AUTOSAVE_INTERVAL_SECONDS, TimerMode::Repeating),
            attempted: HashSet::new(),
            last_saved: HashMap::new(),
        }
    }
}

fn save_player_positions(
    repository: Option<Res<ChunkRepository>>,
    players: Query<(&Player, &PlayerDimension, &Position), Without<PlayerPersistenceDisabled>>,
    real_time: Res<Time<Real>>,
    mut exits: MessageReader<AppExit>,
    mut state: ResMut<PlayerSaveState>,
) {
    let exiting = exits.read().next().is_some();
    let autosave_due = state.timer.tick(real_time.delta()).just_finished();
    let Some(repository) = repository else {
        return;
    };

    for (player, dimension, position) in &players {
        let first_attempt = state.attempted.insert(player.id);
        if !first_attempt && !autosave_due && !exiting {
            continue;
        }

        let stored_position =
            match StoredPlayerPosition::from_translation(dimension.id(), position.0) {
                Ok(position) => position,
                Err(error) => {
                    error!(
                        player_id = player.id.get(),
                        translation = ?position.0,
                        %error,
                        "Refusing to save an invalid player position"
                    );
                    continue;
                }
            };
        if state.last_saved.get(&player.id) == Some(&stored_position) {
            continue;
        }

        let stored_player = StoredPlayer::new(player.id, stored_position);
        match repository.save_player(&stored_player) {
            Ok(()) => {
                state.last_saved.insert(player.id, stored_position);
            }
            Err(error) => {
                error!(
                    player_id = player.id.get(),
                    %error,
                    "Failed to save player position; the autosave will retry"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::world::{DimensionId, WorldMetadata, storage::InMemoryChunkStore};
    use bevy::time::TimeUpdateStrategy;

    #[derive(Resource, Default)]
    struct ExitOnNextLast(bool);

    fn emit_requested_exit(mut request: ResMut<ExitOnNextLast>, mut exits: MessageWriter<AppExit>) {
        if request.0 {
            request.0 = false;
            exits.write(AppExit::Success);
        }
    }

    fn persistence_app() -> App {
        let metadata = WorldMetadata::with_seed(9);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(ChunkRepository::new(InMemoryChunkStore::new(metadata)))
            .init_resource::<ExitOnNextLast>()
            .add_systems(Last, emit_requested_exit.in_set(ExitSystems))
            .add_plugins(PlayerPersistencePlugin);
        app
    }

    #[test]
    fn first_player_frame_is_saved_and_exit_flushes_latest_position() {
        let mut app = persistence_app();
        let player = app
            .world_mut()
            .spawn((
                Player::default(),
                PlayerDimension::new(DimensionId::OVERWORLD),
                Position::new(Vec3::new(-0.25, 31.5, 16.75)),
            ))
            .id();

        app.update();

        let repository = app.world().resource::<ChunkRepository>();
        let initial = repository.load_player(PlayerId::LOCAL).unwrap().unwrap();
        assert_eq!(
            initial.position().translation(),
            Vec3::new(-0.25, 31.5, 16.75)
        );

        app.world_mut().get_mut::<Position>(player).unwrap().0 = Vec3::new(40.5, 12.25, -3.0);
        app.world_mut()
            .resource_mut::<Messages<AppExit>>()
            .write(AppExit::Success);
        app.update();

        let repository = app.world().resource::<ChunkRepository>();
        let flushed = repository.load_player(PlayerId::LOCAL).unwrap().unwrap();
        assert_eq!(
            flushed.position().translation(),
            Vec3::new(40.5, 12.25, -3.0)
        );
    }

    #[test]
    fn window_exit_systems_run_before_the_final_player_flush() {
        let mut app = persistence_app();
        let player = app
            .world_mut()
            .spawn((
                Player::default(),
                PlayerDimension::new(DimensionId::OVERWORLD),
                Position::new(Vec3::new(1.0, 2.0, 3.0)),
            ))
            .id();
        app.update();

        app.world_mut().get_mut::<Position>(player).unwrap().0 = Vec3::new(-40.25, 27.5, 63.75);
        app.world_mut().resource_mut::<ExitOnNextLast>().0 = true;
        app.update();

        let repository = app.world().resource::<ChunkRepository>();
        let flushed = repository.load_player(PlayerId::LOCAL).unwrap().unwrap();
        assert_eq!(
            flushed.position().translation(),
            Vec3::new(-40.25, 27.5, 63.75)
        );
    }

    #[test]
    fn changed_positions_are_saved_on_the_periodic_interval() {
        let mut app = persistence_app();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs(1)));
        let initial = Vec3::new(1.0, 2.0, 3.0);
        let moved = Vec3::new(33.25, 17.5, -0.125);
        let player = app
            .world_mut()
            .spawn((
                Player::default(),
                PlayerDimension::new(DimensionId::OVERWORLD),
                Position::new(initial),
            ))
            .id();
        app.update();

        app.world_mut().get_mut::<Position>(player).unwrap().0 = moved;
        for _ in 0..4 {
            app.update();
        }
        let repository = app.world().resource::<ChunkRepository>();
        assert_eq!(
            repository
                .load_player(PlayerId::LOCAL)
                .unwrap()
                .unwrap()
                .position()
                .translation(),
            initial
        );

        app.update();
        let repository = app.world().resource::<ChunkRepository>();
        assert_eq!(
            repository
                .load_player(PlayerId::LOCAL)
                .unwrap()
                .unwrap()
                .position()
                .translation(),
            moved
        );
    }
}
