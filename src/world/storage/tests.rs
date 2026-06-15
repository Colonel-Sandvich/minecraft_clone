use std::{
    ops::Deref,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use bevy::prelude::*;

use super::*;
use crate::block::BlockType;
use crate::world::chunk::ChunkHeightmap;

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
    chunk.blocks[0][0][0] = block;
    chunk
}

fn default_heightmap() -> ChunkHeightmap {
    ChunkHeightmap::default()
}

#[test]
fn sqlite_store_roundtrips_full_chunks() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_sqlite_store(&metadata);
    let pos = ivec3(-2, 1, 3);
    let mut chunk = chunk_with_block(BlockType::Grass);
    chunk.blocks[15][15][15] = BlockType::OakLeaves;

    store
        .save_chunk(pos, &chunk, &default_heightmap())
        .unwrap();

    let (loaded, _l, _h) = store.load_chunk(pos).unwrap().unwrap();
    assert_eq!(loaded, chunk);
}

#[test]
fn sqlite_store_loads_columns_by_xz() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_sqlite_store(&metadata);
    let column = ivec2(-2, 3);
    let lower = chunk_with_block(BlockType::Grass);
    let upper = chunk_with_block(BlockType::Stone);
    let other_column = chunk_with_block(BlockType::Dirt);

    store
        .save_chunk(ivec3(column.x, 3, column.y), &upper, &default_heightmap())
        .unwrap();
    store
        .save_chunk(ivec3(column.x, 0, column.y), &lower, &default_heightmap())
        .unwrap();
    store
        .save_chunk(ivec3(column.x + 1, 0, column.y), &other_column, &default_heightmap())
        .unwrap();

    let column_data = store.load_stored_column(column).unwrap();
    assert_eq!(column_data.len(), 2);
    assert_eq!(column_data[0].pos, ivec3(column.x, 0, column.y));
    assert_eq!(column_data[0].chunk, lower);
    assert_eq!(column_data[1].pos, ivec3(column.x, 3, column.y));
    assert_eq!(column_data[1].chunk, upper);
}

#[test]
fn in_memory_store_loads_columns_by_xz() {
    let metadata = WorldMetadata::with_seed(42);
    let store = InMemoryChunkStore::new(metadata);
    let column = ivec2(2, -1);
    let lower = chunk_with_block(BlockType::OakLog);
    let upper = chunk_with_block(BlockType::OakLeaves);

    store
        .save_chunk(ivec3(column.x, 2, column.y), &upper, &default_heightmap())
        .unwrap();
    store
        .save_chunk(ivec3(column.x, 0, column.y), &lower, &default_heightmap())
        .unwrap();

    let column_data = store.load_stored_column(column).unwrap();
    assert_eq!(column_data.len(), 2);
    assert_eq!(column_data[0].pos, ivec3(column.x, 0, column.y));
    assert_eq!(column_data[0].chunk, lower);
    assert_eq!(column_data[1].pos, ivec3(column.x, 2, column.y));
    assert_eq!(column_data[1].chunk, upper);
}

#[test]
fn noop_store_discards_chunks() {
    let metadata = WorldMetadata::with_seed(42);
    let store = NoopChunkStore::new(metadata);
    let pos = ivec3(1, 0, 2);

    store
        .save_chunk(pos, &chunk_with_block(BlockType::Grass), &default_heightmap())
        .unwrap();

    assert_eq!(store.load_chunk(pos).unwrap(), None);
    assert_eq!(store.load_stored_column(ivec2(pos.x, pos.z)).unwrap(), []);
}

#[cfg(feature = "turso-store")]
#[test]
fn turso_store_roundtrips_full_chunks() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_turso_store(&metadata);
    let pos = ivec3(-2, 1, 3);
    let mut chunk = chunk_with_block(BlockType::Grass);
    chunk.blocks[15][15][15] = BlockType::OakLeaves;

    store
        .save_chunk(pos, &chunk, &default_heightmap())
        .unwrap();

    let (loaded, _l, _h) = store.load_chunk(pos).unwrap().unwrap();
    assert_eq!(loaded, chunk);
}

#[cfg(feature = "turso-store")]
#[test]
fn turso_store_loads_columns_by_xz() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_turso_store(&metadata);
    let column = ivec2(-2, 3);
    let lower = chunk_with_block(BlockType::Grass);
    let upper = chunk_with_block(BlockType::Stone);

    store
        .save_chunk(ivec3(column.x, 3, column.y), &upper, &default_heightmap())
        .unwrap();
    store
        .save_chunk(ivec3(column.x, 0, column.y), &lower, &default_heightmap())
        .unwrap();

    let column_data = store.load_stored_column(column).unwrap();
    assert_eq!(column_data.len(), 2);
    assert_eq!(column_data[0].pos, ivec3(column.x, 0, column.y));
    assert_eq!(column_data[0].chunk, lower);
    assert_eq!(column_data[1].pos, ivec3(column.x, 3, column.y));
    assert_eq!(column_data[1].chunk, upper);
}

#[test]
fn sqlite_store_rejects_world_metadata_mismatch() {
    let metadata = WorldMetadata::with_seed(42);
    let store = test_sqlite_store(&metadata);
    let mut incompatible = metadata.clone();
    incompatible.height_chunks += 1;

    assert!(store
        .save_chunk(IVec3::ZERO, &Chunk::default(), &default_heightmap())
        .is_ok());
    assert!(SqliteChunkStore::open(&store.path, &incompatible).is_err());
}

#[test]
fn repository_exposes_configured_store_metadata() {
    let metadata = WorldMetadata::with_seed(42);
    let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata.clone()));

    assert_eq!(repository.metadata(), &metadata);
}

#[test]
fn development_world_paths_include_seed() {
    let a = development_world_path(&WorldMetadata::with_seed(1));
    let b = development_world_path(&WorldMetadata::with_seed(2));

    assert_ne!(a, b);
    assert!(a.ends_with("seed-0000000000000001.sqlite3"));
}
