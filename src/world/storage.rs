use std::{
    collections::HashMap,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use bevy::prelude::*;
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};

use crate::world::{
    chunk::{Chunk, ChunkDecodeError},
    generation::WorldMetadata,
};

pub type ChunkStoreResult<T> = Result<T, ChunkStoreError>;

pub trait ChunkStore: Send + Sync + 'static {
    fn load_chunk(&self, pos: IVec3, metadata: &WorldMetadata) -> ChunkStoreResult<Option<Chunk>>;
    fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<()>;
}

#[derive(Resource, Clone)]
pub struct ChunkRepository {
    store: Arc<dyn ChunkStore>,
}

impl ChunkRepository {
    pub fn new(store: impl ChunkStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }

    pub fn load_chunk(
        &self,
        pos: IVec3,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<Option<Chunk>> {
        self.store.load_chunk(pos, metadata)
    }

    pub fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<()> {
        self.store.save_chunk(pos, chunk, metadata)
    }
}

impl Default for ChunkRepository {
    fn default() -> Self {
        Self::new(InMemoryChunkStore::default())
    }
}

#[derive(Default)]
pub struct InMemoryChunkStore {
    inner: Mutex<InMemoryChunkStoreInner>,
}

#[derive(Default)]
struct InMemoryChunkStoreInner {
    metadata: Option<WorldMetadata>,
    chunks: HashMap<IVec3, Vec<u8>>,
}

impl ChunkStore for InMemoryChunkStore {
    fn load_chunk(&self, pos: IVec3, metadata: &WorldMetadata) -> ChunkStoreResult<Option<Chunk>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;
        ensure_store_metadata(&mut inner.metadata, metadata)?;

        let Some(bytes) = inner.chunks.get(&pos) else {
            return Ok(None);
        };

        Ok(Some(Chunk::try_from_storage_bytes(bytes)?))
    }

    fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;
        ensure_store_metadata(&mut inner.metadata, metadata)?;

        inner.chunks.insert(pos, chunk.to_storage_bytes());
        Ok(())
    }
}

pub struct SqliteChunkStore {
    connection: Mutex<Connection>,
    metadata: WorldMetadata,
}

impl SqliteChunkStore {
    pub fn open(path: impl AsRef<Path>, metadata: &WorldMetadata) -> ChunkStoreResult<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }

        let store = Self {
            connection: Mutex::new(Connection::open(path)?),
            metadata: metadata.clone(),
        };
        store.initialize(metadata)?;
        Ok(store)
    }

    pub fn open_in_memory(metadata: &WorldMetadata) -> ChunkStoreResult<Self> {
        let store = Self {
            connection: Mutex::new(Connection::open_in_memory()?),
            metadata: metadata.clone(),
        };
        store.initialize(metadata)?;
        Ok(store)
    }

    fn initialize(&self, metadata: &WorldMetadata) -> ChunkStoreResult<()> {
        let connection = self.connection()?;

        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS world_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS chunks (
                x INTEGER NOT NULL,
                y INTEGER NOT NULL,
                z INTEGER NOT NULL,
                blocks BLOB NOT NULL,
                PRIMARY KEY (x, y, z)
            );",
        )?;

        ensure_metadata_value(&connection, "seed", metadata.seed.to_string())?;
        ensure_metadata_value(
            &connection,
            "generator_version",
            metadata.generator_version.to_string(),
        )?;
        ensure_metadata_value(
            &connection,
            "chunk_format_version",
            metadata.chunk_format_version.to_string(),
        )?;
        ensure_metadata_value(
            &connection,
            "height_chunks",
            metadata.height_chunks.to_string(),
        )?;

        Ok(())
    }

    fn connection(&self) -> ChunkStoreResult<std::sync::MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "sqlite chunk store",
            })
    }
}

impl ChunkStore for SqliteChunkStore {
    fn load_chunk(&self, pos: IVec3, metadata: &WorldMetadata) -> ChunkStoreResult<Option<Chunk>> {
        validate_world_metadata(&self.metadata, metadata)?;

        let connection = self.connection()?;
        let bytes = connection
            .query_row(
                "SELECT blocks
                FROM chunks
                WHERE x = ?1 AND y = ?2 AND z = ?3",
                params![pos.x, pos.y, pos.z],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;

        let Some(bytes) = bytes else {
            return Ok(None);
        };

        Ok(Some(Chunk::try_from_storage_bytes(&bytes)?))
    }

    fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<()> {
        validate_world_metadata(&self.metadata, metadata)?;

        let connection = self.connection()?;
        connection.execute(
            "INSERT INTO chunks (
                x, y, z, blocks
            ) VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(x, y, z) DO UPDATE SET
                blocks = excluded.blocks",
            params![pos.x, pos.y, pos.z, chunk.to_storage_bytes()],
        )?;

        Ok(())
    }
}

pub fn development_world_path(metadata: &WorldMetadata) -> PathBuf {
    PathBuf::from("saves")
        .join("dev")
        .join(format!("seed-{:016x}.sqlite3", metadata.seed))
}

fn ensure_metadata_value(
    connection: &Connection,
    key: &str,
    expected: String,
) -> ChunkStoreResult<()> {
    let existing = connection
        .query_row(
            "SELECT value FROM world_metadata WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    match existing {
        Some(existing) if existing == expected => Ok(()),
        Some(existing) => Err(ChunkStoreError::WorldMetadataMismatch {
            key: key.to_owned(),
            expected,
            found: existing,
        }),
        None => {
            connection.execute(
                "INSERT INTO world_metadata (key, value) VALUES (?1, ?2)",
                params![key, expected],
            )?;
            Ok(())
        }
    }
}

fn ensure_store_metadata(
    stored: &mut Option<WorldMetadata>,
    metadata: &WorldMetadata,
) -> ChunkStoreResult<()> {
    if let Some(stored) = stored {
        validate_world_metadata(stored, metadata)
    } else {
        *stored = Some(metadata.clone());
        Ok(())
    }
}

fn validate_world_metadata(
    expected: &WorldMetadata,
    found: &WorldMetadata,
) -> ChunkStoreResult<()> {
    if expected.seed != found.seed {
        return Err(world_metadata_mismatch("seed", expected.seed, found.seed));
    }

    if expected.generator_version != found.generator_version {
        return Err(world_metadata_mismatch(
            "generator_version",
            expected.generator_version,
            found.generator_version,
        ));
    }

    if expected.chunk_format_version != found.chunk_format_version {
        return Err(world_metadata_mismatch(
            "chunk_format_version",
            expected.chunk_format_version,
            found.chunk_format_version,
        ));
    }

    if expected.height_chunks != found.height_chunks {
        return Err(world_metadata_mismatch(
            "height_chunks",
            expected.height_chunks,
            found.height_chunks,
        ));
    }

    Ok(())
}

fn world_metadata_mismatch(
    key: &str,
    expected: impl ToString,
    found: impl ToString,
) -> ChunkStoreError {
    ChunkStoreError::WorldMetadataMismatch {
        key: key.to_owned(),
        expected: expected.to_string(),
        found: found.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkStoreError {
    LockPoisoned {
        store: &'static str,
    },
    Sqlite {
        code: Option<ErrorCode>,
        extended_code: Option<i32>,
        message: String,
    },
    Io {
        kind: ErrorKind,
        message: String,
    },
    Decode(ChunkDecodeError),
    WorldMetadataMismatch {
        key: String,
        expected: String,
        found: String,
    },
}

impl std::fmt::Display for ChunkStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LockPoisoned { store } => write!(f, "{store} lock poisoned"),
            Self::Sqlite {
                code,
                extended_code,
                message,
            } => write!(f, "sqlite error {code:?}/{extended_code:?}: {message}"),
            Self::Io { kind, message } => write!(f, "io error {kind:?}: {message}"),
            Self::Decode(error) => write!(f, "chunk decode error: {error}"),
            Self::WorldMetadataMismatch {
                key,
                expected,
                found,
            } => write!(
                f,
                "world metadata mismatch for {key}: expected {expected}, found {found}"
            ),
        }
    }
}

impl std::error::Error for ChunkStoreError {}

impl From<rusqlite::Error> for ChunkStoreError {
    fn from(value: rusqlite::Error) -> Self {
        let code = value.sqlite_error_code();
        let extended_code = match &value {
            rusqlite::Error::SqliteFailure(error, _) => Some(error.extended_code),
            _ => None,
        };
        Self::Sqlite {
            code,
            extended_code,
            message: value.to_string(),
        }
    }
}

impl From<std::io::Error> for ChunkStoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io {
            kind: value.kind(),
            message: value.to_string(),
        }
    }
}

impl From<ChunkDecodeError> for ChunkStoreError {
    fn from(value: ChunkDecodeError) -> Self {
        Self::Decode(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockType;

    #[test]
    fn sqlite_store_roundtrips_full_chunks() {
        let metadata = WorldMetadata::with_seed(42);
        let store = SqliteChunkStore::open_in_memory(&metadata).unwrap();
        let pos = ivec3(-2, 1, 3);
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::Grass;
        chunk.blocks[15][15][15] = BlockType::OakLeaves;

        store.save_chunk(pos, &chunk, &metadata).unwrap();

        assert_eq!(store.load_chunk(pos, &metadata).unwrap(), Some(chunk));
    }

    #[test]
    fn sqlite_store_rejects_world_metadata_mismatch() {
        let metadata = WorldMetadata::with_seed(42);
        let store = SqliteChunkStore::open_in_memory(&metadata).unwrap();
        let mut incompatible = metadata.clone();
        incompatible.height_chunks += 1;

        assert!(
            store
                .save_chunk(IVec3::ZERO, &Chunk::default(), &metadata)
                .is_ok()
        );
        assert!(store.load_chunk(IVec3::ZERO, &incompatible).is_err());
    }

    #[test]
    fn in_memory_store_rejects_world_metadata_mismatch() {
        let metadata = WorldMetadata::with_seed(42);
        let store = InMemoryChunkStore::default();
        let mut incompatible = metadata.clone();
        incompatible.generator_version += 1;

        assert!(
            store
                .save_chunk(IVec3::ZERO, &Chunk::default(), &metadata)
                .is_ok()
        );
        assert!(store.load_chunk(IVec3::ZERO, &incompatible).is_err());
    }

    #[test]
    fn development_world_paths_include_seed() {
        let a = development_world_path(&WorldMetadata::with_seed(1));
        let b = development_world_path(&WorldMetadata::with_seed(2));

        assert_ne!(a, b);
        assert!(a.ends_with("seed-0000000000000001.sqlite3"));
    }
}
