use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::*;
use crate::block::BlockType;
use crate::world::chunk::{ChunkColumn, ChunkHeightmap, ChunkPos};
use crate::world::definition::{ChunkAddress, ColumnAddress, DimensionId};
use crate::world::generation::WorldHeight;

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

fn chunk_with_block(block: BlockType) -> Chunk {
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

fn assert_addressed_store_contract(store: &impl ChunkStore, height: WorldHeight) {
    let position = ChunkPos::new(2, 0, -3);
    let overworld = ChunkAddress::new(DimensionId::OVERWORLD, position);
    let grass_floor = ChunkAddress::new(DimensionId::GRASS_FLOOR, position);
    let overworld_chunk = chunk_with_block(BlockType::Stone);
    let grass_floor_chunk = chunk_with_block(BlockType::Grass);
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
                &chunk_with_block(BlockType::Glass),
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
    let mut chunk = chunk_with_block(BlockType::Grass);
    chunk.set_cell_xyz(15, 15, 15, BlockType::OakLeaves.into());

    store
        .save_chunk(address, &chunk, &default_heightmap())
        .unwrap();

    let (loaded, _h) = store.load_chunk(address).unwrap().unwrap();
    assert_eq!(loaded, chunk);
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
    let lower = chunk_with_block(BlockType::Grass);
    let upper = chunk_with_block(BlockType::Stone);
    let other_column = chunk_with_block(BlockType::Dirt);

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
    let lower = chunk_with_block(BlockType::OakLog);
    let upper = chunk_with_block(BlockType::OakLeaves);

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
            &chunk_with_block(BlockType::Grass),
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

#[cfg(feature = "turso-store")]
#[test]
fn turso_store_roundtrips_full_chunks() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_turso_store(&metadata);
    let address = chunk_address(ChunkPos::new(-2, 1, 3));
    let mut chunk = chunk_with_block(BlockType::Grass);
    chunk.set_cell_xyz(15, 15, 15, BlockType::OakLeaves.into());

    store
        .save_chunk(address, &chunk, &default_heightmap())
        .unwrap();

    let (loaded, _h) = store.load_chunk(address).unwrap().unwrap();
    assert_eq!(loaded, chunk);
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
    let lower = chunk_with_block(BlockType::Grass);
    let upper = chunk_with_block(BlockType::Stone);

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
    let lower = chunk_with_block(BlockType::Dirt);
    let upper = chunk_with_block(BlockType::Stone);

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
