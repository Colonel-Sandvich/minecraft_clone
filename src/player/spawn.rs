use std::f32::consts::PI;

use super::{
    PLAYER_HEIGHT, PLAYER_LENGTH, PLAYER_WIDTH, Player, PlayerDimension,
    cam::{MouseCam, MouseSettings},
    persistence::PlayerPersistenceDisabled,
};
use avian3d::prelude::{Collider, Position, RigidBody, TransformInterpolation};
use bevy::prelude::*;

use crate::{
    game_state::GameState,
    mob::controller::{CharacterController, FlyController},
    world::{
        ACTOR_COLLISION_LAYERS,
        dimension::{Active, DesiredColumnView, Dimension},
        storage::ChunkRepository,
    },
};

pub struct SpawnPlayerPlugin;

impl Plugin for SpawnPlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnExit(GameState::GenWorld), spawn_player);
    }
}

pub const EYELINE: f32 = 0.1;

fn spawn_player(
    mut commands: Commands,
    mut dimensions: Query<(Entity, &mut Dimension, &mut DesiredColumnView, Has<Active>)>,
    repository: Option<Res<ChunkRepository>>,
) {
    let roots = dimensions
        .iter()
        .map(|(entity, dimension, _, active)| (entity, dimension.id(), dimension.arrival(), active))
        .collect::<Vec<_>>();
    let active_dimensions = roots
        .iter()
        .filter(|(_, _, _, active)| *active)
        .copied()
        .collect::<Vec<_>>();
    let [(active_entity, active_dimension, active_arrival, _)] = active_dimensions.as_slice()
    else {
        panic!("player spawn requires exactly one active dimension");
    };

    let mut spawn_dimension = *active_dimension;
    let mut spawn_point = *active_arrival;
    let mut persistence_enabled = true;
    if let Some(repository) = repository {
        match repository.load_player(super::PlayerId::LOCAL) {
            Ok(Some(stored)) => {
                let stored_position = stored.position();
                if roots
                    .iter()
                    .any(|(_, dimension, _, _)| *dimension == stored_position.dimension())
                {
                    spawn_dimension = stored_position.dimension();
                    spawn_point = stored_position.translation();
                } else {
                    persistence_enabled = false;
                    warn!(
                        player_id = stored.id().get(),
                        dimension = %stored_position.dimension(),
                        "Saved player dimension has no runtime root; using the active arrival and disabling position saves"
                    );
                }
            }
            Ok(None) => {}
            Err(error) => {
                persistence_enabled = false;
                error!(%error, "Failed to load player position; using the active arrival and disabling position saves");
            }
        }
    }

    if spawn_dimension != *active_dimension {
        let evicted = {
            let (_, mut outgoing, mut outgoing_view, _) = dimensions
                .get_mut(*active_entity)
                .expect("active dimension root must remain queryable");
            let evicted = outgoing.drain_streamed_columns();
            *outgoing_view = DesiredColumnView::default();
            evicted
        };
        for column in evicted {
            commands.entity(column.incarnation).despawn();
        }
        commands
            .entity(*active_entity)
            .remove::<Active>()
            .insert(Visibility::Hidden);
        let (target, _, _, _) = roots
            .iter()
            .find(|(_, dimension, _, _)| *dimension == spawn_dimension)
            .expect("validated player dimension must retain its runtime root");
        commands
            .entity(*target)
            .insert((Active, Visibility::Inherited));
    }

    let mut player = commands.spawn((
        Player::default(),
        PlayerDimension::new(spawn_dimension),
        RigidBody::Kinematic,
        Position::new(spawn_point),
        Transform::from_translation(spawn_point),
        TransformInterpolation,
        ACTOR_COLLISION_LAYERS,
        make_player_collider(),
        CharacterController,
        FlyController,
        Visibility::default(),
        children![(
            MouseCam,
            Camera3d::default(),
            Transform::default()
                .looking_to(Vec3::X, Vec3::Y)
                .with_translation(Vec3::Y * (PLAYER_HEIGHT / 2.0 - EYELINE)),
            Projection::Perspective(PerspectiveProjection {
                fov: MouseSettings::default().fov / 180.0 * PI,
                ..default()
            }),
            IsDefaultUiCamera,
        )],
    ));
    if !persistence_enabled {
        player.insert(PlayerPersistenceDisabled);
    }
}

pub fn make_player_collider() -> Collider {
    Collider::cuboid(PLAYER_LENGTH, PLAYER_HEIGHT, PLAYER_WIDTH)
}

#[cfg(test)]
mod tests {
    use std::io::ErrorKind;

    use super::*;
    use crate::player::PlayerId;
    use crate::world::{
        ChunkAddress, DimensionCatalog, DimensionId, WorldMetadata,
        chunk::{Chunk, ChunkHeightmap},
        storage::{
            ChunkRepository, ChunkStore, ChunkStoreError, ChunkStoreResult, InMemoryChunkStore,
            StoredPlayer, StoredPlayerPosition,
        },
    };

    struct PlayerLoadFailureStore {
        metadata: WorldMetadata,
    }

    impl ChunkStore for PlayerLoadFailureStore {
        fn metadata(&self) -> &WorldMetadata {
            &self.metadata
        }

        fn load_chunk(
            &self,
            _address: ChunkAddress,
        ) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
            Ok(None)
        }

        fn save_chunk(
            &self,
            _address: ChunkAddress,
            _chunk: &Chunk,
            _heightmap: &ChunkHeightmap,
        ) -> ChunkStoreResult<()> {
            Ok(())
        }

        fn load_player(&self, _id: PlayerId) -> ChunkStoreResult<Option<StoredPlayer>> {
            Err(ChunkStoreError::Io {
                kind: ErrorKind::Other,
                message: "scripted player load failure".to_owned(),
            })
        }

        fn save_player(&self, _player: &StoredPlayer) -> ChunkStoreResult<()> {
            panic!("fallback spawn must not overwrite a player that failed to load")
        }
    }

    #[test]
    fn player_uses_active_arrival_without_becoming_a_dimension_child() {
        let definition = *DimensionCatalog::for_world(&WorldMetadata::with_seed(123))
            .get(DimensionId::GRASS_FLOOR)
            .unwrap();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, spawn_player);
        let owner = app.world_mut().spawn_empty().id();
        app.world_mut().entity_mut(owner).insert((
            Dimension::new(owner, definition),
            DesiredColumnView::default(),
            Active,
        ));

        app.update();

        let mut query = app
            .world_mut()
            .query_filtered::<(&PlayerDimension, &Position, Option<&ChildOf>), With<Player>>();
        let (membership, position, parent) = query.single(app.world()).unwrap();
        assert_eq!(membership.id(), definition.id());
        assert_eq!(position.0, definition.arrival());
        assert!(parent.is_none());
    }

    #[test]
    fn player_loads_saved_position_and_activates_its_dimension() {
        let metadata = WorldMetadata::with_seed(123);
        let catalog = DimensionCatalog::for_world(&metadata);
        let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata));
        let position = StoredPlayerPosition::from_translation(
            DimensionId::GRASS_FLOOR,
            Vec3::new(-17.25, 8.5, 34.75),
        )
        .unwrap();
        repository
            .save_player(&StoredPlayer::new(PlayerId::LOCAL, position))
            .unwrap();

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(repository)
            .add_systems(Update, spawn_player);
        let overworld =
            Dimension::spawn_in_world(app.world_mut(), &catalog, DimensionId::OVERWORLD);
        let grass = Dimension::spawn_in_world(app.world_mut(), &catalog, DimensionId::GRASS_FLOOR);
        app.world_mut().entity_mut(overworld).insert(Active);
        app.world_mut().entity_mut(grass).insert(Visibility::Hidden);

        app.update();

        let mut player = app
            .world_mut()
            .query_filtered::<(&PlayerDimension, &Position, &Transform), With<Player>>();
        let (membership, loaded_position, transform) = player.single(app.world()).unwrap();
        assert_eq!(membership.id(), DimensionId::GRASS_FLOOR);
        assert_eq!(loaded_position.0, position.translation());
        assert_eq!(transform.translation, position.translation());
        assert!(app.world().get::<Active>(overworld).is_none());
        assert!(app.world().get::<Active>(grass).is_some());
        assert_eq!(
            app.world().get::<Visibility>(overworld),
            Some(&Visibility::Hidden)
        );
        assert_eq!(
            app.world().get::<Visibility>(grass),
            Some(&Visibility::Inherited)
        );
    }

    #[test]
    fn failed_player_load_disables_saves_instead_of_overwriting_with_fallback() {
        let metadata = WorldMetadata::with_seed(123);
        let catalog = DimensionCatalog::for_world(&metadata);
        let repository = ChunkRepository::new(PlayerLoadFailureStore { metadata });
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(repository)
            .add_plugins(super::super::persistence::PlayerPersistencePlugin)
            .add_systems(Update, spawn_player);
        let overworld =
            Dimension::spawn_in_world(app.world_mut(), &catalog, DimensionId::OVERWORLD);
        app.world_mut().entity_mut(overworld).insert(Active);

        app.update();

        let player = app
            .world_mut()
            .query_filtered::<Entity, With<Player>>()
            .single(app.world())
            .unwrap();
        assert!(
            app.world()
                .get::<PlayerPersistenceDisabled>(player)
                .is_some()
        );
    }
}
