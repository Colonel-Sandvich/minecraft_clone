use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use super::*;
use crate::block::BlockType;
use crate::world::chunk::{ChunkColumn, ChunkHeightmap, ChunkPos};
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

struct MisdirectedColumnStore {
    metadata: WorldMetadata,
    returned_column: ChunkColumn,
    returned_height: WorldHeight,
}

impl ChunkStore for MisdirectedColumnStore {
    fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    fn load_chunk(&self, _position: ChunkPos) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        Ok(None)
    }

    fn load_stored_column(&self, _column: ChunkColumn) -> ChunkStoreResult<StoredColumn> {
        StoredColumn::empty(self.returned_column, self.returned_height).map_err(Into::into)
    }

    fn save_chunk(
        &self,
        _position: ChunkPos,
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
    let position = ChunkPos::new(-2, 1, 3);
    let mut chunk = chunk_with_block(BlockType::Grass);
    chunk.set_cell_xyz(15, 15, 15, BlockType::OakLeaves.into());

    store
        .save_chunk(position, &chunk, &default_heightmap())
        .unwrap();

    let (loaded, _h) = store.load_chunk(position).unwrap().unwrap();
    assert_eq!(loaded, chunk);
}

#[test]
fn sqlite_store_loads_columns_by_xz() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_sqlite_store(&metadata);
    let column = ChunkColumn::new(-2, 3);
    let lower = chunk_with_block(BlockType::Grass);
    let upper = chunk_with_block(BlockType::Stone);
    let other_column = chunk_with_block(BlockType::Dirt);

    store
        .save_chunk(column.chunk(3), &upper, &default_heightmap())
        .unwrap();
    store
        .save_chunk(column.chunk(0), &lower, &default_heightmap())
        .unwrap();
    store
        .save_chunk(
            ChunkPos::new(column.x() + 1, 0, column.z()),
            &other_column,
            &default_heightmap(),
        )
        .unwrap();

    let column_data = store.load_stored_column(column).unwrap();
    assert_eq!(column_data.position(), column);
    assert_eq!(column_data.chunks().len(), 2);
    assert_eq!(column_data.chunks()[0].position, column.chunk(0));
    assert_eq!(column_data.chunks()[0].chunk, lower);
    assert_eq!(column_data.chunks()[1].position, column.chunk(3));
    assert_eq!(column_data.chunks()[1].chunk, upper);
}

#[test]
fn in_memory_store_loads_columns_by_xz() {
    let metadata = WorldMetadata::with_seed(42);
    let store = InMemoryChunkStore::new(metadata);
    let column = ChunkColumn::new(2, -1);
    let lower = chunk_with_block(BlockType::OakLog);
    let upper = chunk_with_block(BlockType::OakLeaves);

    store
        .save_chunk(column.chunk(2), &upper, &default_heightmap())
        .unwrap();
    store
        .save_chunk(column.chunk(0), &lower, &default_heightmap())
        .unwrap();

    let column_data = store.load_stored_column(column).unwrap();
    assert_eq!(column_data.chunks().len(), 2);
    assert_eq!(column_data.chunks()[0].position, column.chunk(0));
    assert_eq!(column_data.chunks()[0].chunk, lower);
    assert_eq!(column_data.chunks()[1].position, column.chunk(2));
    assert_eq!(column_data.chunks()[1].chunk, upper);
}

#[test]
fn noop_store_discards_chunks() {
    let metadata = WorldMetadata::with_seed(42);
    let store = NoopChunkStore::new(metadata);
    let position = ChunkPos::new(1, 0, 2);

    store
        .save_chunk(
            position,
            &chunk_with_block(BlockType::Grass),
            &default_heightmap(),
        )
        .unwrap();

    assert_eq!(store.load_chunk(position).unwrap(), None);
    assert!(
        store
            .load_stored_column(ChunkColumn::from(position))
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
    let position = ChunkPos::new(-2, 1, 3);
    let mut chunk = chunk_with_block(BlockType::Grass);
    chunk.set_cell_xyz(15, 15, 15, BlockType::OakLeaves.into());

    store
        .save_chunk(position, &chunk, &default_heightmap())
        .unwrap();

    let (loaded, _h) = store.load_chunk(position).unwrap().unwrap();
    assert_eq!(loaded, chunk);
}

#[cfg(feature = "turso-store")]
#[test]
fn turso_store_loads_columns_by_xz() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_turso_store(&metadata);
    let column = ChunkColumn::new(-2, 3);
    let lower = chunk_with_block(BlockType::Grass);
    let upper = chunk_with_block(BlockType::Stone);

    store
        .save_chunk(column.chunk(3), &upper, &default_heightmap())
        .unwrap();
    store
        .save_chunk(column.chunk(0), &lower, &default_heightmap())
        .unwrap();

    let column_data = store.load_stored_column(column).unwrap();
    assert_eq!(column_data.chunks().len(), 2);
    assert_eq!(column_data.chunks()[0].position, column.chunk(0));
    assert_eq!(column_data.chunks()[0].chunk, lower);
    assert_eq!(column_data.chunks()[1].position, column.chunk(3));
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
            .save_chunk(ChunkPos::ZERO, &Chunk::default(), &default_heightmap())
            .is_ok()
    );
    assert!(SqliteChunkStore::open(&store.path, &incompatible).is_err());
}

#[test]
fn stored_columns_sort_chunks_and_retain_column_metadata() {
    let column = ChunkColumn::new(-4, 7);
    let heightmap = ChunkHeightmap {
        heights: [[23; crate::world::chunk::CHUNK_SIZE]; crate::world::chunk::CHUNK_SIZE],
    };
    let lower = chunk_with_block(BlockType::Dirt);
    let upper = chunk_with_block(BlockType::Stone);

    let stored = StoredColumn::try_new(
        column,
        WorldHeight::new(4).unwrap(),
        heightmap,
        vec![
            StoredChunk::new(column.chunk(3), upper.clone()),
            StoredChunk::new(column.chunk(0), lower.clone()),
        ],
    )
    .unwrap();

    assert_eq!(stored.position(), column);
    assert_eq!(stored.height(), WorldHeight::new(4).unwrap());
    assert_eq!(stored.heightmap(), &heightmap);
    assert_eq!(stored.chunks()[0], StoredChunk::new(column.chunk(0), lower));
    assert_eq!(stored.chunks()[1], StoredChunk::new(column.chunk(3), upper));
}

#[test]
fn stored_columns_reject_invalid_positions() {
    let column = ChunkColumn::new(2, -3);
    let chunk = Chunk::default();

    assert!(matches!(
        StoredColumn::try_new(
            column,
            WorldHeight::new(3).unwrap(),
            ChunkHeightmap::default(),
            vec![StoredChunk::new(ChunkPos::new(3, 0, -3), chunk.clone())],
        ),
        Err(StoredColumnError::WrongColumn { .. })
    ));
    assert!(matches!(
        StoredColumn::try_new(
            column,
            WorldHeight::new(3).unwrap(),
            ChunkHeightmap::default(),
            vec![StoredChunk::new(column.chunk(3), chunk.clone())],
        ),
        Err(StoredColumnError::YOutOfRange { .. })
    ));
    assert!(matches!(
        StoredColumn::try_new(
            column,
            WorldHeight::new(3).unwrap(),
            ChunkHeightmap::default(),
            vec![
                StoredChunk::new(column.chunk(1), chunk.clone()),
                StoredChunk::new(column.chunk(1), chunk),
            ],
        ),
        Err(StoredColumnError::DuplicatePosition(position)) if position == column.chunk(1)
    ));
}

#[test]
fn repository_exposes_configured_store_metadata() {
    let metadata = WorldMetadata::with_seed(42);
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata.clone()));

    assert_eq!(repository.metadata(), &metadata);
}

#[test]
fn repository_rejects_chunk_positions_outside_world_height() {
    let metadata = WorldMetadata::with_seed(42).with_height_chunks(2).unwrap();
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata));

    for position in [ChunkPos::new(0, -1, 0), ChunkPos::new(0, 2, 0)] {
        assert!(matches!(
            repository.load_chunk(position),
            Err(ChunkStoreError::ChunkPositionOutOfRange {
                position: rejected,
                ..
            }) if rejected == position
        ));
        assert!(matches!(
            repository.save_chunk(position, &Chunk::default(), &default_heightmap()),
            Err(ChunkStoreError::ChunkPositionOutOfRange {
                position: rejected,
                ..
            }) if rejected == position
        ));
    }
}

#[test]
fn repository_rejects_columns_for_the_wrong_request_or_height() {
    let metadata = WorldMetadata::with_seed(42).with_height_chunks(3).unwrap();
    let requested = ChunkColumn::new(4, -7);
    let wrong_column = ChunkColumn::new(5, -7);
    let repository = ChunkRepository::new(MisdirectedColumnStore {
        metadata: metadata.clone(),
        returned_column: wrong_column,
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
        returned_column: requested,
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
fn development_world_paths_include_seed() {
    let a = development_world_path(&WorldMetadata::with_seed(1));
    let b = development_world_path(&WorldMetadata::with_seed(2));

    assert_ne!(a, b);
    assert!(a.ends_with("seed-0000000000000001.sqlite3"));
}
