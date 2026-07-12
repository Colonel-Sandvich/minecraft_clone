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
    chunk::{Chunk, ChunkColumn, ChunkDecodeError, ChunkHeightmap, ChunkPos},
    generation::{WorldHeight, WorldMetadata},
};

pub use memory::{InMemoryChunkStore, NoopChunkStore};
pub use sqlite::{SqliteChunkStore, development_world_path};

#[cfg(feature = "turso-store")]
pub use turso_backend::{TursoChunkStore, development_turso_path};

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
    pub position: ChunkPos,
    pub chunk: Chunk,
}

impl StoredChunk {
    pub fn new(position: impl Into<ChunkPos>, chunk: Chunk) -> Self {
        Self {
            position: position.into(),
            chunk,
        }
    }
}

/// The validated persisted subset of one configured chunk column.
///
/// Missing Y positions are intentional: the loader fills them through world
/// generation. Stored chunks are always ordered from lowest to highest Y.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredColumn {
    position: ChunkColumn,
    height: WorldHeight,
    heightmap: ChunkHeightmap,
    chunks: Vec<StoredChunk>,
}

impl StoredColumn {
    pub fn try_new(
        position: ChunkColumn,
        height: WorldHeight,
        heightmap: ChunkHeightmap,
        mut chunks: Vec<StoredChunk>,
    ) -> Result<Self, StoredColumnError> {
        for stored in &chunks {
            let actual_column = ChunkColumn::from(stored.position);
            if actual_column != position {
                return Err(StoredColumnError::WrongColumn {
                    expected: position,
                    position: stored.position,
                });
            }

            let y = stored.position.as_ivec3().y;
            if !(0..height.chunks_i32()).contains(&y) {
                return Err(StoredColumnError::YOutOfRange {
                    position: stored.position,
                    height,
                });
            }
        }

        chunks.sort_unstable_by_key(|stored| stored.position.as_ivec3().y);
        if let Some(duplicate) = chunks
            .windows(2)
            .find(|pair| pair[0].position == pair[1].position)
            .map(|pair| pair[0].position)
        {
            return Err(StoredColumnError::DuplicatePosition(duplicate));
        }

        Ok(Self {
            position,
            height,
            heightmap,
            chunks,
        })
    }

    pub fn empty(position: ChunkColumn, height: WorldHeight) -> Result<Self, StoredColumnError> {
        Self::try_new(position, height, ChunkHeightmap::default(), Vec::new())
    }

    pub const fn position(&self) -> ChunkColumn {
        self.position
    }

    pub const fn height(&self) -> WorldHeight {
        self.height
    }

    pub const fn heightmap(&self) -> &ChunkHeightmap {
        &self.heightmap
    }

    pub fn chunks(&self) -> &[StoredChunk] {
        &self.chunks
    }

    pub fn into_parts(self) -> (ChunkHeightmap, Vec<StoredChunk>) {
        (self.heightmap, self.chunks)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredColumnError {
    WrongColumn {
        expected: ChunkColumn,
        position: ChunkPos,
    },
    YOutOfRange {
        position: ChunkPos,
        height: WorldHeight,
    },
    DuplicatePosition(ChunkPos),
}

impl std::fmt::Display for StoredColumnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongColumn { expected, position } => write!(
                f,
                "stored chunk {position:?} does not belong to column {expected:?}"
            ),
            Self::YOutOfRange { position, height } => write!(
                f,
                "stored chunk {position:?} is outside configured height 0..{}",
                height.chunks()
            ),
            Self::DuplicatePosition(position) => {
                write!(f, "stored column contains duplicate chunk {position:?}")
            }
        }
    }
}

impl std::error::Error for StoredColumnError {}

pub trait ChunkStore: Send + Sync + 'static {
    fn metadata(&self) -> &WorldMetadata;

    fn load_chunk(&self, pos: IVec3) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>>;

    fn load_stored_column(&self, column: ChunkColumn) -> ChunkStoreResult<StoredColumn> {
        let mut chunks = Vec::new();
        let mut heightmap = ChunkHeightmap::default();
        let height = self.metadata().height();
        for y in 0..height.chunks_i32() {
            let position = column.chunk(y);
            if let Some((chunk, loaded_heightmap)) = self.load_chunk(position.as_ivec3())? {
                heightmap = loaded_heightmap;
                chunks.push(StoredChunk::new(position, chunk));
            }
        }

        StoredColumn::try_new(column, height, heightmap, chunks).map_err(Into::into)
    }

    fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()>;
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

    pub fn load_chunk(
        &self,
        position: impl Into<ChunkPos>,
    ) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        self.store.load_chunk(position.into().as_ivec3())
    }

    pub fn load_stored_column(&self, column: ChunkColumn) -> ChunkStoreResult<StoredColumn> {
        self.store.load_stored_column(column)
    }

    pub fn save_chunk(
        &self,
        position: impl Into<ChunkPos>,
        chunk: &Chunk,
        heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()> {
        self.store
            .save_chunk(position.into().as_ivec3(), chunk, heightmap)
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
        ("height_chunks", metadata.height_chunks().to_string()),
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
    InvalidStoredColumn(StoredColumnError),
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
            Self::InvalidStoredColumn(error) => write!(f, "invalid stored column: {error}"),
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

impl From<StoredColumnError> for ChunkStoreError {
    fn from(value: StoredColumnError) -> Self {
        Self::InvalidStoredColumn(value)
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
