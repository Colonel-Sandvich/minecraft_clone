use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::*;
use crate::item::Item;
use crate::player::PlayerId;
use crate::world::chunk::{ChunkColumn, ChunkHeightmap, ChunkPos};
use crate::world::definition::{ChunkAddress, ColumnAddress, DimensionId};
use crate::world::generation::WorldHeight;
use bevy::math::{DVec2, DVec3};

static NEXT_TEST_STORE_ID: AtomicU64 = AtomicU64::new(0);

struct TestSqliteStore {
    store: SqliteChunkStore,
    path: PathBuf,
}

#[cfg(feature = "turso-store")]
struct TestTursoStore {
    store: TursoChunkStore,
    path: PathBuf,
}

impl Deref for TestSqliteStore {
    type Target = SqliteChunkStore;

    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

impl Drop for TestSqliteStore {
    fn drop(&mut self) {
        remove_test_store_files(&self.path);
    }
}

#[cfg(feature = "turso-store")]
impl Deref for TestTursoStore {
    type Target = TursoChunkStore;

    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

#[cfg(feature = "turso-store")]
impl Drop for TestTursoStore {
    fn drop(&mut self) {
        remove_test_store_files(&self.path);
    }
}

fn test_store_path(prefix: &str) -> PathBuf {
    let id = NEXT_TEST_STORE_ID.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "minecraft_clone-{prefix}-{}-{id}.sqlite3",
        std::process::id()
    ))
}

fn remove_test_store_files(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
    }
}

fn test_sqlite_store(metadata: &WorldMetadata) -> TestSqliteStore {
    let path = test_store_path("sqlite-test");
    remove_test_store_files(&path);
    let store = SqliteChunkStore::open(&path, metadata).unwrap();

    TestSqliteStore { store, path }
}

#[cfg(feature = "turso-store")]
fn test_turso_store(metadata: &WorldMetadata) -> TestTursoStore {
    let path = test_store_path("turso-test");
    remove_test_store_files(&path);
    let store = TursoChunkStore::open(&path, metadata).unwrap();

    TestTursoStore { store, path }
}

fn chunk_with_block(block: Item) -> Chunk {
    let mut chunk = Chunk::default();
    chunk.set_cell_xyz(0, 0, 0, block.into());
    chunk
}

fn default_heightmap() -> ChunkHeightmap {
    ChunkHeightmap::default()
}

const TEST_DIMENSION: DimensionId = DimensionId::OVERWORLD;

fn chunk_address(position: ChunkPos) -> ChunkAddress {
    ChunkAddress::new(TEST_DIMENSION, position)
}

fn column_address(column: ChunkColumn) -> ColumnAddress {
    ColumnAddress::new(TEST_DIMENSION, column)
}

fn stored_player(id: PlayerId, dimension: DimensionId, translation: Vec3) -> StoredPlayer {
    StoredPlayer::new(
        id,
        StoredPlayerPosition::from_translation(dimension, translation).unwrap(),
    )
}

fn assert_player_store_contract(store: &impl ChunkStore) {
    let first_id = PlayerId::LOCAL;
    let second_id = PlayerId::new(2);
    let first = stored_player(
        first_id,
        DimensionId::OVERWORLD,
        Vec3::new(-0.25, 31.5, 16.75),
    );
    let second = stored_player(
        second_id,
        DimensionId::GRASS_FLOOR,
        Vec3::new(64.125, 4.75, -32.5),
    );

    store.save_player(&first).unwrap();
    store.save_player(&second).unwrap();
    assert_eq!(store.load_player(first_id).unwrap(), Some(first.clone()));
    assert_eq!(store.load_player(second_id).unwrap(), Some(second.clone()));

    let moved = stored_player(
        first_id,
        DimensionId::CENTER_GLASS_PLATFORM,
        Vec3::new(17.5, 8.25, -0.125),
    );
    store.save_player(&moved).unwrap();
    assert_eq!(store.load_player(first_id).unwrap(), Some(moved));
    assert_eq!(store.load_player(second_id).unwrap(), Some(second));
}

fn assert_addressed_store_contract(store: &impl ChunkStore, height: WorldHeight) {
    let position = ChunkPos::new(2, 0, -3);
    let overworld = ChunkAddress::new(DimensionId::OVERWORLD, position);
    let grass_floor = ChunkAddress::new(DimensionId::GRASS_FLOOR, position);
    let overworld_chunk = chunk_with_block(Item::Stone);
    let grass_floor_chunk = chunk_with_block(Item::Grass);
    let overworld_heightmap = ChunkHeightmap {
        heights: [[17; crate::world::chunk::CHUNK_SIZE]; crate::world::chunk::CHUNK_SIZE],
    };
    let grass_floor_heightmap = ChunkHeightmap {
        heights: [[1; crate::world::chunk::CHUNK_SIZE]; crate::world::chunk::CHUNK_SIZE],
    };

    store
        .save_chunk(overworld, &overworld_chunk, &overworld_heightmap)
        .unwrap();
    store
        .save_chunk(grass_floor, &grass_floor_chunk, &grass_floor_heightmap)
        .unwrap();

    assert_eq!(
        store.load_chunk(overworld).unwrap(),
        Some((overworld_chunk.clone(), overworld_heightmap))
    );
    assert_eq!(
        store.load_chunk(grass_floor).unwrap(),
        Some((grass_floor_chunk.clone(), grass_floor_heightmap))
    );

    let overworld_column = store
        .load_stored_column(overworld.column(), height)
        .unwrap();
    let grass_floor_column = store
        .load_stored_column(grass_floor.column(), height)
        .unwrap();
    assert_eq!(overworld_column.heightmap(), &overworld_heightmap);
    assert_eq!(grass_floor_column.heightmap(), &grass_floor_heightmap);
    assert_eq!(overworld_column.chunks()[0].chunk, overworld_chunk);
    assert_eq!(grass_floor_column.chunks()[0].chunk, grass_floor_chunk);

    let bounded_column =
        ColumnAddress::new(DimensionId::CENTER_GLASS_PLATFORM, ChunkColumn::new(9, 9));
    for y in [-1, height.chunks_i32()] {
        store
            .save_chunk(
                bounded_column.chunk(y),
                &chunk_with_block(Item::Glass),
                &default_heightmap(),
            )
            .unwrap();
    }
    assert!(
        store
            .load_stored_column(bounded_column, height)
            .unwrap()
            .chunks()
            .is_empty()
    );
}

struct MisdirectedColumnStore {
    metadata: WorldMetadata,
    returned_address: ColumnAddress,
    returned_height: WorldHeight,
}

impl ChunkStore for MisdirectedColumnStore {
    fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    fn load_chunk(
        &self,
        _address: ChunkAddress,
    ) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        Ok(None)
    }

    fn load_stored_column(
        &self,
        _address: ColumnAddress,
        _height: WorldHeight,
    ) -> ChunkStoreResult<StoredColumn> {
        StoredColumn::empty(self.returned_address, self.returned_height).map_err(Into::into)
    }

    fn save_chunk(
        &self,
        _address: ChunkAddress,
        _chunk: &Chunk,
        _heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()> {
        Ok(())
    }
}

#[test]
fn sqlite_store_roundtrips_full_chunks() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_sqlite_store(&metadata);
    let address = chunk_address(ChunkPos::new(-2, 1, 3));
    let mut chunk = chunk_with_block(Item::Grass);
    chunk.set_cell_xyz(15, 15, 15, Item::OakLeaves.into());

    store
        .save_chunk(address, &chunk, &default_heightmap())
        .unwrap();

    let (loaded, _h) = store.load_chunk(address).unwrap().unwrap();
    assert_eq!(loaded, chunk);
}

#[test]
fn stored_player_positions_preserve_fractional_and_negative_coordinates() {
    for (translation, expected_chunk) in [
        (Vec3::ZERO, ChunkPos::ZERO),
        (Vec3::splat(16.0), ChunkPos::new(1, 1, 1)),
        (Vec3::splat(-16.0), ChunkPos::new(-1, -1, -1)),
        (Vec3::new(-0.25, 31.5, 16.75), ChunkPos::new(-1, 1, 1)),
        (Vec3::new(-16.0, -0.125, -16.001), ChunkPos::new(-1, -1, -2)),
        (Vec3::new(15.999, 0.0, 32.5), ChunkPos::new(0, 0, 2)),
        (Vec3::splat(-f32::EPSILON), ChunkPos::new(-1, -1, -1)),
    ] {
        let position =
            StoredPlayerPosition::from_translation(DimensionId::OVERWORLD, translation).unwrap();
        assert_eq!(position.chunk(), expected_chunk);
        assert_eq!(position.translation(), translation);
        assert!(
            position
                .local()
                .to_array()
                .into_iter()
                .all(|coordinate| (0.0..16.0).contains(&coordinate))
        );
    }

    let negative = StoredPlayerPosition::from_translation(
        DimensionId::OVERWORLD,
        Vec3::new(-0.25, -16.0, -16.001),
    )
    .unwrap();
    assert_eq!(negative.chunk(), ChunkPos::new(-1, -1, -2));
    assert_eq!(negative.local().xy(), DVec2::new(15.75, 0.0));
    assert!((negative.local().z - 15.999).abs() < 0.000_01);
    for invalid in [Vec3::NAN, Vec3::INFINITY, Vec3::NEG_INFINITY] {
        assert!(StoredPlayerPosition::from_translation(DimensionId::OVERWORLD, invalid).is_err());
    }
    for invalid_local in [
        DVec3::new(16.0, 0.0, 0.0),
        DVec3::new(-f64::EPSILON, 0.0, 0.0),
        DVec3::NAN,
        DVec3::INFINITY,
    ] {
        assert!(
            StoredPlayerPosition::try_new(DimensionId::OVERWORLD, ChunkPos::ZERO, invalid_local,)
                .is_err()
        );
    }
}

#[test]
fn sqlite_store_roundtrips_and_upserts_independent_player_rows() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_sqlite_store(&metadata);

    assert_player_store_contract(&*store);
    assert_eq!(
        store.load_chunk(chunk_address(ChunkPos::ZERO)).unwrap(),
        None
    );
}

#[test]
fn sqlite_player_position_index_uses_dimension_x_z_y_chunk_order() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_sqlite_store(&metadata);
    let connection = rusqlite::Connection::open(&store.path).unwrap();
    let mut statement = connection
        .prepare("PRAGMA index_info('players_by_chunk_position')")
        .unwrap();
    let columns = statement
        .query_map([], |row| row.get::<_, String>(2))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(columns, ["dimension", "chunk_x", "chunk_z", "chunk_y"]);
}

#[test]
fn sqlite_open_adds_player_schema_to_an_existing_world() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_sqlite_store(&metadata);
    let address = chunk_address(ChunkPos::new(2, 1, -3));
    let chunk = chunk_with_block(Item::Stone);
    store
        .save_chunk(address, &chunk, &default_heightmap())
        .unwrap();
    let connection = rusqlite::Connection::open(&store.path).unwrap();
    connection.execute("DROP TABLE players", []).unwrap();
    drop(connection);

    let reopened = SqliteChunkStore::open(&store.path, &metadata).unwrap();

    assert_eq!(
        reopened
            .load_chunk(address)
            .unwrap()
            .map(|(chunk, _)| chunk),
        Some(chunk)
    );
    assert_eq!(reopened.load_player(PlayerId::LOCAL).unwrap(), None);
    assert_player_store_contract(&reopened);
}

#[test]
fn sqlite_store_isolates_dimensions_at_equal_coordinates() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_sqlite_store(&metadata);

    assert_addressed_store_contract(&*store, metadata.height());
}

#[test]
fn sqlite_store_loads_columns_by_xz() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_sqlite_store(&metadata);
    let column = ChunkColumn::new(-2, 3);
    let address = column_address(column);
    let lower = chunk_with_block(Item::Grass);
    let upper = chunk_with_block(Item::Stone);
    let other_column = chunk_with_block(Item::Dirt);

    store
        .save_chunk(address.chunk(3), &upper, &default_heightmap())
        .unwrap();
    store
        .save_chunk(address.chunk(0), &lower, &default_heightmap())
        .unwrap();
    store
        .save_chunk(
            chunk_address(ChunkPos::new(column.x() + 1, 0, column.z())),
            &other_column,
            &default_heightmap(),
        )
        .unwrap();

    let column_data = store
        .load_stored_column(address, metadata.height())
        .unwrap();
    assert_eq!(column_data.address(), address);
    assert_eq!(column_data.position(), column);
    assert_eq!(column_data.chunks().len(), 2);
    assert_eq!(column_data.chunks()[0].address, address.chunk(0));
    assert_eq!(column_data.chunks()[0].chunk, lower);
    assert_eq!(column_data.chunks()[1].address, address.chunk(3));
    assert_eq!(column_data.chunks()[1].chunk, upper);
}

#[test]
fn in_memory_store_loads_columns_by_xz() {
    let metadata = WorldMetadata::with_seed(42);
    let store = InMemoryChunkStore::new(metadata.clone());
    let column = ChunkColumn::new(2, -1);
    let address = column_address(column);
    let lower = chunk_with_block(Item::OakLog);
    let upper = chunk_with_block(Item::OakLeaves);

    store
        .save_chunk(address.chunk(2), &upper, &default_heightmap())
        .unwrap();
    store
        .save_chunk(address.chunk(0), &lower, &default_heightmap())
        .unwrap();

    let column_data = store
        .load_stored_column(address, metadata.height())
        .unwrap();
    assert_eq!(column_data.chunks().len(), 2);
    assert_eq!(column_data.chunks()[0].address, address.chunk(0));
    assert_eq!(column_data.chunks()[0].chunk, lower);
    assert_eq!(column_data.chunks()[1].address, address.chunk(2));
    assert_eq!(column_data.chunks()[1].chunk, upper);
}

#[test]
fn in_memory_store_roundtrips_independent_player_rows() {
    let store = InMemoryChunkStore::new(WorldMetadata::with_seed(42));

    assert_player_store_contract(&store);
}

#[test]
fn in_memory_store_isolates_dimensions_at_equal_coordinates() {
    let metadata = WorldMetadata::with_seed(42);
    let store = InMemoryChunkStore::new(metadata.clone());

    assert_addressed_store_contract(&store, metadata.height());
}

#[test]
fn noop_store_discards_chunks() {
    let metadata = WorldMetadata::with_seed(42);
    let store = NoopChunkStore::new(metadata.clone());
    let address = chunk_address(ChunkPos::new(1, 0, 2));

    store
        .save_chunk(
            address,
            &chunk_with_block(Item::Grass),
            &default_heightmap(),
        )
        .unwrap();

    assert_eq!(store.load_chunk(address).unwrap(), None);
    assert!(
        store
            .load_stored_column(address.column(), metadata.height())
            .unwrap()
            .chunks()
            .is_empty()
    );
}

#[test]
fn noop_store_discards_players() {
    let store = NoopChunkStore::new(WorldMetadata::with_seed(42));
    let player = stored_player(
        PlayerId::LOCAL,
        DimensionId::OVERWORLD,
        Vec3::new(8.0, 24.0, 8.0),
    );

    store.save_player(&player).unwrap();

    assert_eq!(store.load_player(PlayerId::LOCAL).unwrap(), None);
}

#[cfg(feature = "turso-store")]
#[test]
fn turso_store_roundtrips_full_chunks() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_turso_store(&metadata);
    let address = chunk_address(ChunkPos::new(-2, 1, 3));
    let mut chunk = chunk_with_block(Item::Grass);
    chunk.set_cell_xyz(15, 15, 15, Item::OakLeaves.into());

    store
        .save_chunk(address, &chunk, &default_heightmap())
        .unwrap();

    let (loaded, _h) = store.load_chunk(address).unwrap().unwrap();
    assert_eq!(loaded, chunk);
}

#[cfg(feature = "turso-store")]
#[test]
fn turso_store_roundtrips_and_upserts_independent_player_rows() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_turso_store(&metadata);

    assert_player_store_contract(&*store);
    assert_eq!(
        store.load_chunk(chunk_address(ChunkPos::ZERO)).unwrap(),
        None
    );
}

#[cfg(feature = "turso-store")]
#[test]
fn turso_player_position_index_uses_dimension_x_z_y_chunk_order() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_turso_store(&metadata);

    assert_eq!(
        store.player_position_index_columns_for_test().unwrap(),
        ["dimension", "chunk_x", "chunk_z", "chunk_y"]
    );
}

#[cfg(feature = "turso-store")]
#[test]
fn turso_open_adds_player_schema_to_an_existing_world() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_turso_store(&metadata);
    let address = chunk_address(ChunkPos::new(2, 1, -3));
    let chunk = chunk_with_block(Item::Stone);
    store
        .save_chunk(address, &chunk, &default_heightmap())
        .unwrap();
    store.drop_player_schema_for_test().unwrap();

    let reopened = TursoChunkStore::open(&store.path, &metadata).unwrap();

    assert_eq!(
        reopened
            .load_chunk(address)
            .unwrap()
            .map(|(chunk, _)| chunk),
        Some(chunk)
    );
    assert_eq!(reopened.load_player(PlayerId::LOCAL).unwrap(), None);
    assert_player_store_contract(&reopened);
}

#[cfg(feature = "turso-store")]
#[test]
fn turso_store_isolates_dimensions_at_equal_coordinates() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_turso_store(&metadata);

    assert_addressed_store_contract(&*store, metadata.height());
}

#[cfg(feature = "turso-store")]
#[test]
fn turso_store_rejects_dimension_generator_metadata_mismatch() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_turso_store(&metadata);
    store.overwrite_metadata_for_test("dimension.2.generator_family", "changed");

    assert!(matches!(
        TursoChunkStore::open(&store.path, &metadata),
        Err(ChunkStoreError::WorldMetadataMismatch {
            key,
            expected,
            found,
        }) if key == "dimension.2.generator_family"
            && expected == "center_glass_platform"
            && found == "changed"
    ));
}

#[cfg(feature = "turso-store")]
#[test]
fn turso_store_loads_columns_by_xz() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_turso_store(&metadata);
    let column = ChunkColumn::new(-2, 3);
    let address = column_address(column);
    let lower = chunk_with_block(Item::Grass);
    let upper = chunk_with_block(Item::Stone);

    store
        .save_chunk(address.chunk(3), &upper, &default_heightmap())
        .unwrap();
    store
        .save_chunk(address.chunk(0), &lower, &default_heightmap())
        .unwrap();

    let column_data = store
        .load_stored_column(address, metadata.height())
        .unwrap();
    assert_eq!(column_data.chunks().len(), 2);
    assert_eq!(column_data.chunks()[0].address, address.chunk(0));
    assert_eq!(column_data.chunks()[0].chunk, lower);
    assert_eq!(column_data.chunks()[1].address, address.chunk(3));
    assert_eq!(column_data.chunks()[1].chunk, upper);
}

#[test]
fn sqlite_store_rejects_world_metadata_mismatch() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_sqlite_store(&metadata);
    let incompatible = metadata
        .clone()
        .with_height_chunks(metadata.height_chunks() + 1)
        .unwrap();

    assert!(
        store
            .save_chunk(
                chunk_address(ChunkPos::ZERO),
                &Chunk::default(),
                &default_heightmap(),
            )
            .is_ok()
    );
    assert!(SqliteChunkStore::open(&store.path, &incompatible).is_err());
}

#[test]
fn sqlite_store_rejects_the_previous_storage_format_cleanly() {
    let metadata = WorldMetadata::with_seed(42);
    let mut legacy_metadata = metadata.clone();
    legacy_metadata.chunk_format_version = 1;
    let store = test_sqlite_store(&legacy_metadata);
    let connection = rusqlite::Connection::open(&store.path).unwrap();
    connection
        .execute_batch(
            "DROP TABLE chunks;
            DROP TABLE column_heightmaps;
            CREATE TABLE chunks (
                x INTEGER NOT NULL,
                z INTEGER NOT NULL,
                y INTEGER NOT NULL,
                blocks BLOB NOT NULL,
                PRIMARY KEY (x, z, y)
            ) WITHOUT ROWID;
            CREATE TABLE column_heightmaps (
                x INTEGER NOT NULL,
                z INTEGER NOT NULL,
                heightmap BLOB NOT NULL,
                PRIMARY KEY (x, z)
            ) WITHOUT ROWID;",
        )
        .unwrap();
    drop(connection);

    assert!(matches!(
        SqliteChunkStore::open(&store.path, &metadata),
        Err(ChunkStoreError::WorldMetadataMismatch {
            key,
            expected,
            found,
        }) if key == "chunk_format_version" && expected == "2" && found == "1"
    ));
}

#[test]
fn sqlite_store_rejects_dimension_generator_metadata_mismatch() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_sqlite_store(&metadata);
    let connection = rusqlite::Connection::open(&store.path).unwrap();
    connection
        .execute(
            "UPDATE world_metadata SET value = '999' WHERE key = ?1",
            ["dimension.1.generator_version"],
        )
        .unwrap();
    drop(connection);

    assert!(matches!(
        SqliteChunkStore::open(&store.path, &metadata),
        Err(ChunkStoreError::WorldMetadataMismatch {
            key,
            expected,
            found,
        }) if key == "dimension.1.generator_version" && expected == "1" && found == "999"
    ));
}

#[test]
fn stored_columns_sort_chunks_and_retain_column_metadata() {
    let column = ChunkColumn::new(-4, 7);
    let address = column_address(column);
    let heightmap = ChunkHeightmap {
        heights: [[23; crate::world::chunk::CHUNK_SIZE]; crate::world::chunk::CHUNK_SIZE],
    };
    let lower = chunk_with_block(Item::Dirt);
    let upper = chunk_with_block(Item::Stone);

    let stored = StoredColumn::try_new(
        address,
        WorldHeight::new(4).unwrap(),
        heightmap,
        vec![
            StoredChunk::new(address.chunk(3), upper.clone()),
            StoredChunk::new(address.chunk(0), lower.clone()),
        ],
    )
    .unwrap();

    assert_eq!(stored.position(), column);
    assert_eq!(stored.address(), address);
    assert_eq!(stored.height(), WorldHeight::new(4).unwrap());
    assert_eq!(stored.heightmap(), &heightmap);
    assert_eq!(
        stored.chunks()[0],
        StoredChunk::new(address.chunk(0), lower)
    );
    assert_eq!(
        stored.chunks()[1],
        StoredChunk::new(address.chunk(3), upper)
    );
}

#[test]
fn stored_columns_reject_invalid_positions() {
    let column = ChunkColumn::new(2, -3);
    let address = column_address(column);
    let chunk = Chunk::default();

    assert!(matches!(
        StoredColumn::try_new(
            address,
            WorldHeight::new(3).unwrap(),
            ChunkHeightmap::default(),
            vec![StoredChunk::new(
                chunk_address(ChunkPos::new(3, 0, -3)),
                chunk.clone(),
            )],
        ),
        Err(StoredColumnError::WrongColumn { .. })
    ));
    assert!(matches!(
        StoredColumn::try_new(
            address,
            WorldHeight::new(3).unwrap(),
            ChunkHeightmap::default(),
            vec![StoredChunk::new(
                ColumnAddress::new(DimensionId::GRASS_FLOOR, column).chunk(0),
                chunk.clone(),
            )],
        ),
        Err(StoredColumnError::WrongColumn { .. })
    ));
    assert!(matches!(
        StoredColumn::try_new(
            address,
            WorldHeight::new(3).unwrap(),
            ChunkHeightmap::default(),
            vec![StoredChunk::new(address.chunk(3), chunk.clone())],
        ),
        Err(StoredColumnError::YOutOfRange { .. })
    ));
    assert!(matches!(
        StoredColumn::try_new(
            address,
            WorldHeight::new(3).unwrap(),
            ChunkHeightmap::default(),
            vec![
                StoredChunk::new(address.chunk(1), chunk.clone()),
                StoredChunk::new(address.chunk(1), chunk),
            ],
        ),
        Err(StoredColumnError::DuplicateAddress(duplicate)) if duplicate == address.chunk(1)
    ));
}

#[test]
fn repository_exposes_configured_store_metadata() {
    let metadata = WorldMetadata::with_seed(42);
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata.clone()));

    assert_eq!(repository.metadata(), &metadata);
    assert_eq!(
        repository
            .catalog()
            .get(DimensionId::OVERWORLD)
            .map(|definition| definition.height()),
        Some(metadata.height())
    );
}

#[test]
fn repository_rejects_chunk_addresses_outside_dimension_height() {
    let metadata = WorldMetadata::with_seed(42).with_height_chunks(2).unwrap();
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata));

    for position in [ChunkPos::new(0, -1, 0), ChunkPos::new(0, 2, 0)] {
        let address = chunk_address(position);
        assert!(matches!(
            repository.load_chunk(address),
            Err(ChunkStoreError::ChunkAddressOutOfRange {
                address: rejected,
                ..
            }) if rejected == address
        ));
        assert!(matches!(
            repository.save_chunk(address, &Chunk::default(), &default_heightmap()),
            Err(ChunkStoreError::ChunkAddressOutOfRange {
                address: rejected,
                ..
            }) if rejected == address
        ));
    }
}

#[test]
fn repository_rejects_unknown_dimensions() {
    let repository = ChunkRepository::default();
    let unknown = DimensionId::new(u32::MAX);

    assert!(matches!(
        repository.load_chunk(ChunkAddress::new(unknown, ChunkPos::ZERO)),
        Err(ChunkStoreError::UnknownDimension { dimension }) if dimension == unknown
    ));
    assert!(matches!(
        repository.load_stored_column(ColumnAddress::new(unknown, ChunkColumn::new(0, 0))),
        Err(ChunkStoreError::UnknownDimension { dimension }) if dimension == unknown
    ));
}

#[test]
fn repository_rejects_player_positions_in_unknown_dimensions() {
    let repository = ChunkRepository::default();
    let unknown = DimensionId::new(u32::MAX);
    let player = stored_player(PlayerId::LOCAL, unknown, Vec3::new(1.0, 2.0, 3.0));

    assert!(matches!(
        repository.save_player(&player),
        Err(ChunkStoreError::UnknownDimension { dimension }) if dimension == unknown
    ));
}

#[test]
fn repository_rejects_columns_for_the_wrong_request_or_height() {
    let metadata = WorldMetadata::with_seed(42).with_height_chunks(3).unwrap();
    let requested = column_address(ChunkColumn::new(4, -7));
    let wrong_column = column_address(ChunkColumn::new(5, -7));
    let repository = ChunkRepository::new(MisdirectedColumnStore {
        metadata: metadata.clone(),
        returned_address: wrong_column,
        returned_height: metadata.height(),
    });
    assert!(matches!(
        repository.load_stored_column(requested),
        Err(ChunkStoreError::InvalidStoredColumn(
            StoredColumnError::RequestedColumnMismatch {
                requested: actual_request,
                returned,
            }
        )) if actual_request == requested && returned == wrong_column
    ));

    let wrong_height = WorldHeight::new(2).unwrap();
    let repository = ChunkRepository::new(MisdirectedColumnStore {
        metadata,
        returned_address: requested,
        returned_height: wrong_height,
    });
    assert!(matches!(
        repository.load_stored_column(requested),
        Err(ChunkStoreError::InvalidStoredColumn(
            StoredColumnError::HeightMismatch { returned, .. }
        )) if returned == wrong_height
    ));
}

#[test]
fn repository_rejects_columns_returned_for_another_dimension() {
    let metadata = WorldMetadata::with_seed(42);
    let requested = ColumnAddress::new(DimensionId::OVERWORLD, ChunkColumn::new(4, -7));
    let returned = ColumnAddress::new(DimensionId::GRASS_FLOOR, requested.column());
    let repository = ChunkRepository::new(MisdirectedColumnStore {
        metadata: metadata.clone(),
        returned_address: returned,
        returned_height: metadata.height(),
    });

    assert!(matches!(
        repository.load_stored_column(requested),
        Err(ChunkStoreError::InvalidStoredColumn(
            StoredColumnError::RequestedColumnMismatch {
                requested: actual_request,
                returned: actual_return,
            }
        )) if actual_request == requested && actual_return == returned
    ));
}

#[test]
fn development_world_paths_include_complete_format_identity() {
    let base = WorldMetadata::with_seed(1);
    let different_seed = WorldMetadata::with_seed(2);
    let mut different_generator = base.clone();
    different_generator.generator_version += 1;
    let mut different_chunk_format = base.clone();
    different_chunk_format.chunk_format_version += 1;
    let different_height = base.clone().with_height_chunks(4).unwrap();

    let path = development_world_path(&base);
    for different in [
        different_seed,
        different_generator,
        different_chunk_format,
        different_height,
    ] {
        assert_ne!(path, development_world_path(&different));
    }
    assert!(path.ends_with("seed-0000000000000001-g1-c2-h5.sqlite3"));
    #[cfg(feature = "turso-store")]
    assert!(development_turso_path(&base).ends_with("seed-0000000000000001-g1-c2-h5.turso"));
}
