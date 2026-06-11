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
    chunks: Mutex<HashMap<IVec3, StoredChunk>>,
}

struct StoredChunk {
    format_version: u32,
    generator_version: u32,
    height_chunks: usize,
    blocks: Vec<u8>,
}

impl ChunkStore for InMemoryChunkStore {
    fn load_chunk(&self, pos: IVec3, metadata: &WorldMetadata) -> ChunkStoreResult<Option<Chunk>> {
        let chunks = self
            .chunks
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;
        let Some(stored) = chunks.get(&pos) else {
            return Ok(None);
        };

        validate_chunk_metadata(
            pos,
            stored.format_version,
            stored.generator_version,
            stored.height_chunks,
            metadata,
        )?;

        Ok(Some(Chunk::try_from_storage_bytes(&stored.blocks)?))
    }

    fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<()> {
        self.chunks
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?
            .insert(
                pos,
                StoredChunk {
                    format_version: metadata.chunk_format_version,
                    generator_version: metadata.generator_version,
                    height_chunks: metadata.height_chunks,
                    blocks: chunk.to_storage_bytes(),
                },
            );
        Ok(())
    }
}

pub struct SqliteChunkStore {
    connection: Mutex<Connection>,
}

impl SqliteChunkStore {
    pub fn open(path: impl AsRef<Path>, metadata: &WorldMetadata) -> ChunkStoreResult<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)?;
        }

        let store = Self {
            connection: Mutex::new(Connection::open(path)?),
        };
        store.initialize(metadata)?;
        Ok(store)
    }

    pub fn open_in_memory(metadata: &WorldMetadata) -> ChunkStoreResult<Self> {
        let store = Self {
            connection: Mutex::new(Connection::open_in_memory()?),
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
                format_version INTEGER NOT NULL,
                generator_version INTEGER NOT NULL,
                height_chunks INTEGER NOT NULL,
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
        let connection = self.connection()?;
        let row = connection
            .query_row(
                "SELECT format_version, generator_version, height_chunks, blocks
                FROM chunks
                WHERE x = ?1 AND y = ?2 AND z = ?3",
                params![pos.x, pos.y, pos.z],
                |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Vec<u8>>(3)?,
                    ))
                },
            )
            .optional()?;

        let Some((format_version, generator_version, height_chunks, bytes)) = row else {
            return Ok(None);
        };
        let height_chunks = usize::try_from(height_chunks)
            .map_err(|_| ChunkStoreError::InvalidStoredHeight { pos, height_chunks })?;

        validate_chunk_metadata(
            pos,
            format_version,
            generator_version,
            height_chunks,
            metadata,
        )?;

        Ok(Some(Chunk::try_from_storage_bytes(&bytes)?))
    }

    fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<()> {
        let connection = self.connection()?;
        let height_chunks = i64::try_from(metadata.height_chunks).map_err(|_| {
            ChunkStoreError::HeightChunksOutOfRange {
                height_chunks: metadata.height_chunks,
            }
        })?;
        connection.execute(
            "INSERT INTO chunks (
                x, y, z, format_version, generator_version, height_chunks, blocks
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(x, y, z) DO UPDATE SET
                format_version = excluded.format_version,
                generator_version = excluded.generator_version,
                height_chunks = excluded.height_chunks,
                blocks = excluded.blocks",
            params![
                pos.x,
                pos.y,
                pos.z,
                metadata.chunk_format_version,
                metadata.generator_version,
                height_chunks,
                chunk.to_storage_bytes(),
            ],
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

fn validate_chunk_metadata(
    pos: IVec3,
    format_version: u32,
    generator_version: u32,
    height_chunks: usize,
    metadata: &WorldMetadata,
) -> ChunkStoreResult<()> {
    if format_version != metadata.chunk_format_version
        || generator_version != metadata.generator_version
        || height_chunks != metadata.height_chunks
    {
        return Err(ChunkStoreError::ChunkMetadataMismatch {
            pos,
            format_version,
            generator_version,
            height_chunks,
            expected_format_version: metadata.chunk_format_version,
            expected_generator_version: metadata.generator_version,
            expected_height_chunks: metadata.height_chunks,
        });
    }

    Ok(())
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
    ChunkMetadataMismatch {
        pos: IVec3,
        format_version: u32,
        generator_version: u32,
        height_chunks: usize,
        expected_format_version: u32,
        expected_generator_version: u32,
        expected_height_chunks: usize,
    },
    InvalidStoredHeight {
        pos: IVec3,
        height_chunks: i64,
    },
    HeightChunksOutOfRange {
        height_chunks: usize,
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
            Self::ChunkMetadataMismatch {
                pos,
                format_version,
                generator_version,
                height_chunks,
                expected_format_version,
                expected_generator_version,
                expected_height_chunks,
            } => write!(
                f,
                "chunk at {pos:?} has incompatible metadata: format {format_version}, generator {generator_version}, height {height_chunks}; expected format {expected_format_version}, generator {expected_generator_version}, height {expected_height_chunks}"
            ),
            Self::InvalidStoredHeight { pos, height_chunks } => {
                write!(
                    f,
                    "chunk at {pos:?} has invalid stored height {height_chunks}"
                )
            }
            Self::HeightChunksOutOfRange { height_chunks } => write!(
                f,
                "height_chunks {height_chunks} does not fit in SQLite integer"
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
