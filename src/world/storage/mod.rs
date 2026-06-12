mod memory;
mod sqlite;

#[cfg(test)]
mod tests;

#[cfg(feature = "turso-store")]
#[path = "turso.rs"]
mod turso_backend;

use std::{io::ErrorKind, sync::Arc};

use bevy::prelude::*;
use rusqlite::ErrorCode;

use crate::world::{
    chunk::{Chunk, ChunkDecodeError},
    generation::WorldMetadata,
};

pub use memory::{InMemoryChunkStore, NoopChunkStore};
pub use sqlite::{SqliteChunkStore, development_world_path};

#[cfg(feature = "turso-store")]
pub use turso_backend::TursoChunkStore;

pub type ChunkStoreResult<T> = Result<T, ChunkStoreError>;

pub(crate) const SQL_CREATE_WORLD_METADATA: &str = "CREATE TABLE IF NOT EXISTS world_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
)";
pub(crate) const SQL_SELECT_METADATA_VALUE: &str =
    "SELECT value FROM world_metadata WHERE key = ?1";
pub(crate) const SQL_INSERT_METADATA_VALUE: &str =
    "INSERT INTO world_metadata (key, value) VALUES (?1, ?2)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredChunk {
    pub pos: IVec3,
    pub chunk: Chunk,
}

pub trait ChunkStore: Send + Sync + 'static {
    fn metadata(&self) -> &WorldMetadata;

    fn load_chunk(&self, pos: IVec3) -> ChunkStoreResult<Option<Chunk>>;

    fn load_stored_column(&self, column: IVec2) -> ChunkStoreResult<Vec<StoredChunk>> {
        let mut chunks = Vec::new();
        for y in 0..self.metadata().height_chunks as i32 {
            let pos = ivec3(column.x, y, column.y);
            if let Some(chunk) = self.load_chunk(pos)? {
                chunks.push(StoredChunk { pos, chunk });
            }
        }

        Ok(chunks)
    }

    fn save_chunk(&self, pos: IVec3, chunk: &Chunk) -> ChunkStoreResult<()>;
}

#[derive(Resource, Clone)]
pub struct ChunkRepository {
    metadata: WorldMetadata,
    store: Arc<dyn ChunkStore>,
}

impl ChunkRepository {
    pub fn new(store: impl ChunkStore) -> Self {
        let metadata = store.metadata().clone();
        Self {
            metadata,
            store: Arc::new(store),
        }
    }

    pub fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    pub fn load_chunk(&self, pos: IVec3) -> ChunkStoreResult<Option<Chunk>> {
        self.store.load_chunk(pos)
    }

    pub fn load_stored_column(&self, column: IVec2) -> ChunkStoreResult<Vec<StoredChunk>> {
        self.store.load_stored_column(column)
    }

    pub fn save_chunk(&self, pos: IVec3, chunk: &Chunk) -> ChunkStoreResult<()> {
        self.store.save_chunk(pos, chunk)
    }
}

impl Default for ChunkRepository {
    fn default() -> Self {
        Self::new(InMemoryChunkStore::new(WorldMetadata::default()))
    }
}

pub(crate) fn metadata_entries(metadata: &WorldMetadata) -> [(&'static str, String); 4] {
    [
        ("seed", metadata.seed.to_string()),
        ("generator_version", metadata.generator_version.to_string()),
        (
            "chunk_format_version",
            metadata.chunk_format_version.to_string(),
        ),
        ("height_chunks", metadata.height_chunks.to_string()),
    ]
}

#[cfg(feature = "turso-store")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TursoStoreErrorKind {
    Busy,
    BusySnapshot,
    Interrupt,
    Io(ErrorKind),
    Other,
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
    #[cfg(feature = "turso-store")]
    Turso {
        kind: TursoStoreErrorKind,
        message: String,
    },
    #[cfg(feature = "turso-store")]
    Runtime {
        message: String,
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
            #[cfg(feature = "turso-store")]
            Self::Turso { kind, message } => write!(f, "turso error {kind:?}: {message}"),
            #[cfg(feature = "turso-store")]
            Self::Runtime { message } => write!(f, "runtime error: {message}"),
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

#[cfg(feature = "turso-store")]
impl From<::turso::Error> for ChunkStoreError {
    fn from(value: ::turso::Error) -> Self {
        let kind = match &value {
            ::turso::Error::Busy(_) => TursoStoreErrorKind::Busy,
            ::turso::Error::BusySnapshot(_) => TursoStoreErrorKind::BusySnapshot,
            ::turso::Error::Interrupt(_) => TursoStoreErrorKind::Interrupt,
            ::turso::Error::IoError(kind, _) => TursoStoreErrorKind::Io(*kind),
            _ => TursoStoreErrorKind::Other,
        };

        Self::Turso {
            kind,
            message: value.to_string(),
        }
    }
}
