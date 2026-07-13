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
    definition::{ChunkAddress, ColumnAddress, DimensionCatalog, DimensionId},
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
    pub address: ChunkAddress,
    pub chunk: Chunk,
}

impl StoredChunk {
    pub fn new(address: ChunkAddress, chunk: Chunk) -> Self {
        Self { address, chunk }
    }

    pub const fn position(&self) -> ChunkPos {
        self.address.position()
    }
}

/// The validated persisted subset of one configured chunk column.
///
/// Missing Y positions are intentional: the loader fills them through world
/// generation. Stored chunks are always ordered from lowest to highest Y.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredColumn {
    address: ColumnAddress,
    height: WorldHeight,
    heightmap: ChunkHeightmap,
    chunks: Vec<StoredChunk>,
}

impl StoredColumn {
    pub fn try_new(
        address: ColumnAddress,
        height: WorldHeight,
        heightmap: ChunkHeightmap,
        mut chunks: Vec<StoredChunk>,
    ) -> Result<Self, StoredColumnError> {
        for stored in &chunks {
            if stored.address.column() != address {
                return Err(StoredColumnError::WrongColumn {
                    expected: address,
                    address: stored.address,
                });
            }

            if !height.contains_chunk(stored.position()) {
                return Err(StoredColumnError::YOutOfRange {
                    address: stored.address,
                    height,
                });
            }
        }

        chunks.sort_unstable_by_key(|stored| stored.position().y());
        if let Some(duplicate) = chunks
            .windows(2)
            .find(|pair| pair[0].address == pair[1].address)
            .map(|pair| pair[0].address)
        {
            return Err(StoredColumnError::DuplicateAddress(duplicate));
        }

        Ok(Self {
            address,
            height,
            heightmap,
            chunks,
        })
    }

    pub fn empty(address: ColumnAddress, height: WorldHeight) -> Result<Self, StoredColumnError> {
        Self::try_new(address, height, ChunkHeightmap::default(), Vec::new())
    }

    pub const fn address(&self) -> ColumnAddress {
        self.address
    }

    pub const fn position(&self) -> ChunkColumn {
        self.address.column()
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

    fn validate_request(
        &self,
        requested_address: ColumnAddress,
        expected_height: WorldHeight,
    ) -> Result<(), StoredColumnError> {
        if self.address != requested_address {
            return Err(StoredColumnError::RequestedColumnMismatch {
                requested: requested_address,
                returned: self.address,
            });
        }
        if self.height != expected_height {
            return Err(StoredColumnError::HeightMismatch {
                expected: expected_height,
                returned: self.height,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredColumnError {
    WrongColumn {
        expected: ColumnAddress,
        address: ChunkAddress,
    },
    YOutOfRange {
        address: ChunkAddress,
        height: WorldHeight,
    },
    RequestedColumnMismatch {
        requested: ColumnAddress,
        returned: ColumnAddress,
    },
    HeightMismatch {
        expected: WorldHeight,
        returned: WorldHeight,
    },
    DuplicateAddress(ChunkAddress),
}

impl std::fmt::Display for StoredColumnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongColumn { expected, address } => write!(
                f,
                "stored chunk {address:?} does not belong to column {expected:?}"
            ),
            Self::YOutOfRange { address, height } => write!(
                f,
                "stored chunk {address:?} is outside configured height 0..{}",
                height.chunks()
            ),
            Self::RequestedColumnMismatch {
                requested,
                returned,
            } => write!(
                f,
                "store returned column {returned:?} for request {requested:?}"
            ),
            Self::HeightMismatch { expected, returned } => write!(
                f,
                "store returned height {} for configured height {}",
                returned.chunks(),
                expected.chunks()
            ),
            Self::DuplicateAddress(address) => {
                write!(f, "stored column contains duplicate chunk {address:?}")
            }
        }
    }
}

impl std::error::Error for StoredColumnError {}

pub trait ChunkStore: Send + Sync + 'static {
    fn metadata(&self) -> &WorldMetadata;

    fn load_chunk(
        &self,
        address: ChunkAddress,
    ) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>>;

    fn load_stored_column(
        &self,
        address: ColumnAddress,
        height: WorldHeight,
    ) -> ChunkStoreResult<StoredColumn> {
        let mut chunks = Vec::new();
        let mut heightmap = ChunkHeightmap::default();
        for y in 0..height.chunks_i32() {
            let chunk_address = address.chunk(y);
            if let Some((chunk, loaded_heightmap)) = self.load_chunk(chunk_address)? {
                heightmap = loaded_heightmap;
                chunks.push(StoredChunk::new(chunk_address, chunk));
            }
        }

        StoredColumn::try_new(address, height, heightmap, chunks).map_err(Into::into)
    }

    fn save_chunk(
        &self,
        address: ChunkAddress,
        chunk: &Chunk,
        heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()>;
}

#[derive(Resource, Clone)]
pub struct ChunkRepository {
    metadata: WorldMetadata,
    catalog: DimensionCatalog,
    store: Arc<dyn ChunkStore>,
}

impl ChunkRepository {
    pub fn new(store: impl ChunkStore) -> Self {
        let metadata = store.metadata().clone();
        let catalog = DimensionCatalog::for_world(&metadata);
        Self {
            metadata,
            catalog,
            store: Arc::new(store),
        }
    }

    pub fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    pub fn catalog(&self) -> &DimensionCatalog {
        &self.catalog
    }

    pub fn load_chunk(
        &self,
        address: ChunkAddress,
    ) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        self.validate_address(address)?;
        self.store.load_chunk(address)
    }

    pub fn load_stored_column(&self, address: ColumnAddress) -> ChunkStoreResult<StoredColumn> {
        let height = self.dimension_height(address.dimension())?;
        let stored = self.store.load_stored_column(address, height)?;
        stored.validate_request(address, height)?;
        Ok(stored)
    }

    pub fn save_chunk(
        &self,
        address: ChunkAddress,
        chunk: &Chunk,
        heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()> {
        self.validate_address(address)?;
        self.store.save_chunk(address, chunk, heightmap)
    }

    pub fn dimension_height(&self, dimension: DimensionId) -> ChunkStoreResult<WorldHeight> {
        self.catalog
            .get(dimension)
            .map(|definition| definition.height())
            .ok_or(ChunkStoreError::UnknownDimension { dimension })
    }

    fn validate_address(&self, address: ChunkAddress) -> ChunkStoreResult<()> {
        let height = self.dimension_height(address.dimension())?;
        if !height.contains_chunk(address.position()) {
            return Err(ChunkStoreError::ChunkAddressOutOfRange { address, height });
        }
        Ok(())
    }
}

impl Default for ChunkRepository {
    fn default() -> Self {
        Self::new(InMemoryChunkStore::new(WorldMetadata::default()))
    }
}

pub(crate) fn metadata_entries(metadata: &WorldMetadata) -> Vec<(String, String)> {
    let catalog = DimensionCatalog::for_world(metadata);
    let dimension_ids = catalog
        .iter()
        .map(|definition| definition.id().to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mut entries = vec![
        ("seed".to_owned(), metadata.seed.to_string()),
        (
            "generator_version".to_owned(),
            metadata.generator_version.to_string(),
        ),
        (
            "chunk_format_version".to_owned(),
            metadata.chunk_format_version.to_string(),
        ),
        (
            "height_chunks".to_owned(),
            metadata.height_chunks().to_string(),
        ),
        ("dimension_ids".to_owned(), dimension_ids),
    ];

    for definition in catalog.iter() {
        let prefix = format!("dimension.{}", definition.id());
        entries.extend([
            (
                format!("{prefix}.generator_family"),
                definition.generator().family().to_owned(),
            ),
            (
                format!("{prefix}.generator_version"),
                definition.generator().version().to_string(),
            ),
            (
                format!("{prefix}.height_chunks"),
                definition.height().chunks().to_string(),
            ),
        ]);
    }

    entries
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
    UnknownDimension {
        dimension: DimensionId,
    },
    ChunkAddressOutOfRange {
        address: ChunkAddress,
        height: WorldHeight,
    },
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
            Self::UnknownDimension { dimension } => {
                write!(f, "unknown dimension {dimension}")
            }
            Self::ChunkAddressOutOfRange { address, height } => write!(
                f,
                "chunk {address:?} is outside configured height 0..{}",
                height.chunks()
            ),
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

impl ChunkStoreError {
    pub fn is_transient(&self) -> bool {
        match self {
            Self::Sqlite { code, .. } => matches!(
                code,
                Some(
                    ErrorCode::DatabaseBusy
                        | ErrorCode::DatabaseLocked
                        | ErrorCode::OperationInterrupted
                )
            ),
            Self::Io { kind, .. } => matches!(
                kind,
                ErrorKind::Interrupted | ErrorKind::TimedOut | ErrorKind::WouldBlock
            ),
            #[cfg(feature = "turso-store")]
            Self::Turso { kind, .. } => match kind {
                TursoStoreErrorKind::Busy
                | TursoStoreErrorKind::BusySnapshot
                | TursoStoreErrorKind::Interrupt => true,
                TursoStoreErrorKind::Io(kind) => matches!(
                    kind,
                    ErrorKind::Interrupted | ErrorKind::TimedOut | ErrorKind::WouldBlock
                ),
                TursoStoreErrorKind::Other => false,
            },
            _ => false,
        }
    }
}

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
