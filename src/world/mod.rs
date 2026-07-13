pub mod chunk;
pub mod definition;
pub mod dimension;
pub mod generation;
pub mod loading;
pub mod storage;

use std::path::PathBuf;

use avian3d::prelude::CollisionLayers;
use bevy::prelude::*;

use chunk::ChunkPlugin;
use dimension::DimensionPlugin;
use storage::{
    ChunkRepository, ChunkStoreResult, InMemoryChunkStore, NoopChunkStore, SqliteChunkStore,
    development_world_path,
};
#[cfg(feature = "turso-store")]
use storage::{TursoChunkStore, development_turso_path};

pub use definition::{
    ChunkAddress, ColumnAddress, DimensionCatalog, DimensionDefinition, DimensionId,
    GeneratorProfile,
};
pub use generation::{InvalidWorldHeight, WorldHeight, WorldMetadata};

pub struct WorldPlugin;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ChunkSimulationSet {
    ExternalMutation,
    FluidStep,
}

#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct WorldConfig {
    pub metadata: WorldMetadata,
    pub storage: WorldStorageConfig,
}

impl WorldConfig {
    pub fn in_memory(metadata: WorldMetadata) -> Self {
        Self {
            metadata,
            storage: WorldStorageConfig::InMemory,
        }
    }

    pub fn noop(metadata: WorldMetadata) -> Self {
        Self {
            metadata,
            storage: WorldStorageConfig::Noop,
        }
    }

    pub fn sqlite(metadata: WorldMetadata, path: impl Into<PathBuf>) -> Self {
        Self {
            metadata,
            storage: WorldStorageConfig::Sqlite { path: path.into() },
        }
    }

    #[cfg(feature = "turso-store")]
    pub fn turso(metadata: WorldMetadata, path: impl Into<PathBuf>) -> Self {
        Self {
            metadata,
            storage: WorldStorageConfig::Turso { path: path.into() },
        }
    }

    pub fn development_sqlite(metadata: WorldMetadata) -> Self {
        let path = development_world_path(&metadata);
        Self::sqlite(metadata, path)
    }

    #[cfg(feature = "turso-store")]
    pub fn development_turso(metadata: WorldMetadata) -> Self {
        let path = development_turso_path(&metadata);
        Self::turso(metadata, path)
    }
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self::in_memory(WorldMetadata::default())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldStorageConfig {
    InMemory,
    Noop,
    Sqlite {
        path: PathBuf,
    },
    #[cfg(feature = "turso-store")]
    Turso {
        path: PathBuf,
    },
}

pub const WORLD_LAYER: u32 = 1 << 0;
pub const ACTOR_LAYER: u32 = 1 << 1;

pub const WORLD_COLLISION_LAYERS: CollisionLayers =
    CollisionLayers::from_bits(WORLD_LAYER, ACTOR_LAYER);
pub const ACTOR_COLLISION_LAYERS: CollisionLayers =
    CollisionLayers::from_bits(ACTOR_LAYER, WORLD_LAYER);

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        ensure_world_resources(app);
        configure_chunk_simulation(app);
        app.add_plugins((DimensionPlugin, ChunkPlugin));
    }
}

fn configure_chunk_simulation(app: &mut App) {
    app.configure_sets(
        FixedUpdate,
        (
            ChunkSimulationSet::ExternalMutation,
            ChunkSimulationSet::FluidStep,
        )
            .chain(),
    );
}

fn ensure_world_resources(app: &mut App) {
    let config = app
        .world()
        .get_resource::<WorldConfig>()
        .cloned()
        .unwrap_or_else(|| {
            let metadata = app
                .world()
                .get_resource::<WorldMetadata>()
                .cloned()
                .unwrap_or_default();
            WorldConfig::in_memory(metadata)
        });

    if !app.world().contains_resource::<WorldConfig>() {
        app.insert_resource(config.clone());
    }

    if let Some(metadata) = app.world().get_resource::<WorldMetadata>() {
        assert_eq!(
            metadata, &config.metadata,
            "WorldMetadata resource must match WorldConfig metadata"
        );
    } else {
        app.insert_resource(config.metadata.clone());
    }

    if let Some(repository) = app.world().get_resource::<ChunkRepository>() {
        assert_eq!(
            repository.metadata(),
            &config.metadata,
            "ChunkRepository metadata must match WorldConfig metadata"
        );
        return;
    }

    let repository = build_chunk_repository(&config)
        .unwrap_or_else(|error| panic!("failed to open configured chunk store: {error}"));
    app.insert_resource(repository);
}

fn build_chunk_repository(config: &WorldConfig) -> ChunkStoreResult<ChunkRepository> {
    match &config.storage {
        WorldStorageConfig::InMemory => {
            info!(seed = config.metadata.seed, "Using in-memory chunk store");
            Ok(ChunkRepository::new(InMemoryChunkStore::new(
                config.metadata.clone(),
            )))
        }
        WorldStorageConfig::Noop => {
            info!(seed = config.metadata.seed, "Using no-op chunk store");
            Ok(ChunkRepository::new(NoopChunkStore::new(
                config.metadata.clone(),
            )))
        }
        WorldStorageConfig::Sqlite { path } => {
            let store = SqliteChunkStore::open(path, &config.metadata)?;
            info!(
                seed = config.metadata.seed,
                path = %path.display(),
                "Using SQLite chunk store"
            );
            Ok(ChunkRepository::new(store))
        }
        #[cfg(feature = "turso-store")]
        WorldStorageConfig::Turso { path } => {
            let store = TursoChunkStore::open(path, &config.metadata)?;
            info!(
                seed = config.metadata.seed,
                path = %path.display(),
                "Using Turso chunk store"
            );
            Ok(ChunkRepository::new(store))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::chunk::ChunkNeedsFluidStep;
    use super::*;

    #[derive(Resource)]
    struct DeferredMarkerTarget(Entity);

    #[derive(Resource, Default)]
    struct FluidSetObservedMarker(bool);

    fn insert_marker_in_external_mutation_set(
        mut commands: Commands,
        target: Res<DeferredMarkerTarget>,
    ) {
        commands.entity(target.0).insert(ChunkNeedsFluidStep);
    }

    fn observe_marker_in_fluid_set(
        target: Res<DeferredMarkerTarget>,
        marked: Query<(), With<ChunkNeedsFluidStep>>,
        mut observed: ResMut<FluidSetObservedMarker>,
    ) {
        observed.0 = marked.get(target.0).is_ok();
    }

    #[test]
    fn external_mutation_commands_are_visible_to_the_fluid_set() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<FluidSetObservedMarker>();
        configure_chunk_simulation(&mut app);
        app.add_systems(
            FixedUpdate,
            insert_marker_in_external_mutation_set.in_set(ChunkSimulationSet::ExternalMutation),
        )
        .add_systems(
            FixedUpdate,
            observe_marker_in_fluid_set.in_set(ChunkSimulationSet::FluidStep),
        );
        let target = app.world_mut().spawn_empty().id();
        app.insert_resource(DeferredMarkerTarget(target));

        app.world_mut().run_schedule(FixedUpdate);

        assert!(app.world().resource::<FluidSetObservedMarker>().0);
    }

    #[test]
    fn default_world_config_is_stable_and_in_memory() {
        let config = WorldConfig::default();

        assert_eq!(config.metadata, WorldMetadata::default());
        assert!(matches!(config.storage, WorldStorageConfig::InMemory));
    }

    #[test]
    fn development_world_config_uses_seeded_sqlite_path() {
        let metadata = WorldMetadata::with_seed(42);
        let config = WorldConfig::development_sqlite(metadata.clone());

        assert_eq!(config.metadata, metadata);
        assert_eq!(
            config.storage,
            WorldStorageConfig::Sqlite {
                path: development_world_path(&metadata)
            }
        );
    }

    #[test]
    fn noop_world_config_is_selectable() {
        let metadata = WorldMetadata::with_seed(42);
        let config = WorldConfig::noop(metadata.clone());

        assert_eq!(config.metadata, metadata);
        assert_eq!(config.storage, WorldStorageConfig::Noop);
    }

    #[test]
    #[should_panic(expected = "ChunkRepository metadata must match WorldConfig metadata")]
    fn preinstalled_repository_must_match_world_config_metadata() {
        let configured = WorldMetadata::with_seed(42);
        let stored = WorldMetadata::with_seed(99);
        let mut app = App::new();
        app.insert_resource(WorldConfig::in_memory(configured))
            .insert_resource(ChunkRepository::new(InMemoryChunkStore::new(stored)));

        ensure_world_resources(&mut app);
    }

    #[cfg(feature = "turso-store")]
    #[test]
    fn development_turso_config_uses_seeded_path() {
        let metadata = WorldMetadata::with_seed(42);

        let config = WorldConfig::development_turso(metadata.clone());

        assert_eq!(config.metadata, metadata);
        assert_eq!(
            config.storage,
            WorldStorageConfig::Turso {
                path: storage::development_turso_path(&metadata)
            }
        );
    }
}
