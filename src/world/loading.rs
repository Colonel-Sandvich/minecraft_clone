use bevy::prelude::*;

use crate::world::{
    chunk::{Chunk, ChunkColumn, ChunkHeightmap, ChunkPos},
    generation::{WorldHeight, generate_chunk},
    storage::{ChunkRepository, ChunkStoreError},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkLoadError {
    pub kind: ChunkLoadErrorKind,
    pub source: ChunkStoreError,
}

impl ChunkLoadError {
    pub fn transient(source: ChunkStoreError) -> Self {
        Self {
            kind: ChunkLoadErrorKind::Transient,
            source,
        }
    }

    pub fn permanent(source: ChunkStoreError) -> Self {
        Self {
            kind: ChunkLoadErrorKind::Permanent,
            source,
        }
    }

    pub const fn is_transient(&self) -> bool {
        matches!(self.kind, ChunkLoadErrorKind::Transient)
    }
}

impl std::fmt::Display for ChunkLoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} chunk load error: {}", self.kind, self.source)
    }
}

impl std::error::Error for ChunkLoadError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkLoadErrorKind {
    Transient,
    Permanent,
}

impl std::fmt::Display for ChunkLoadErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transient => write!(f, "transient"),
            Self::Permanent => write!(f, "permanent"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkLoadSource {
    Stored,
    Generated,
}

/// One fully resolved subchunk in a loaded column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedColumnChunk {
    pub position: ChunkPos,
    pub chunk: Chunk,
    pub source: ChunkLoadSource,
}

/// A complete configured-height column, ordered from lowest to highest Y.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedColumn {
    pub position: ChunkColumn,
    pub height: WorldHeight,
    pub heightmap: ChunkHeightmap,
    chunks: Vec<LoadedColumnChunk>,
}

impl LoadedColumn {
    fn new(
        position: ChunkColumn,
        height: WorldHeight,
        heightmap: ChunkHeightmap,
        chunks: Vec<LoadedColumnChunk>,
    ) -> Self {
        debug_assert!(
            chunks
                .iter()
                .enumerate()
                .all(|(y, loaded)| { loaded.position == position.chunk(y as i32) })
        );
        Self {
            position,
            height,
            heightmap,
            chunks,
        }
    }

    pub fn chunks(&self) -> &[LoadedColumnChunk] {
        &self.chunks
    }

    pub fn into_chunks(self) -> Vec<LoadedColumnChunk> {
        self.chunks
    }
}

pub type ColumnLoadResult = Result<LoadedColumn, ChunkLoadError>;

/// Loads persisted chunks in one store call and deterministically generates
/// every missing Y position, producing a complete configured-height column.
pub fn load_or_generate_column(
    position: ChunkColumn,
    repository: ChunkRepository,
) -> ColumnLoadResult {
    let stored = repository
        .load_stored_column(position)
        .map_err(classify_load_error)?;
    let height = repository.metadata().height();
    let height_chunks = height.chunks();
    debug_assert_eq!(stored.position(), position);
    debug_assert_eq!(stored.height(), height);

    let (heightmap, stored_chunks) = stored.into_parts();
    let mut stored_chunks = stored_chunks.into_iter().peekable();
    let mut chunks = Vec::with_capacity(height_chunks);
    for y in 0..height_chunks {
        let chunk_position = position.chunk(y as i32);
        let (chunk, source) = match stored_chunks.peek() {
            Some(stored) if stored.position == chunk_position => {
                let stored = stored_chunks
                    .next()
                    .expect("peeked stored chunk must exist");
                (stored.chunk, ChunkLoadSource::Stored)
            }
            _ => (
                generate_chunk(repository.metadata(), chunk_position.as_ivec3()),
                ChunkLoadSource::Generated,
            ),
        };
        chunks.push(LoadedColumnChunk {
            position: chunk_position,
            chunk,
            source,
        });
    }
    debug_assert!(stored_chunks.next().is_none());

    Ok(LoadedColumn::new(position, height, heightmap, chunks))
}

pub fn classify_load_error(error: ChunkStoreError) -> ChunkLoadError {
    if error.is_transient() {
        ChunkLoadError::transient(error)
    } else {
        ChunkLoadError::permanent(error)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::ErrorKind,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::*;
    #[cfg(feature = "turso-store")]
    use crate::world::storage::TursoStoreErrorKind;
    use crate::{
        block::BlockType,
        world::{
            generation::WorldMetadata,
            storage::{
                ChunkStore, ChunkStoreResult, InMemoryChunkStore, StoredChunk, StoredColumn,
            },
        },
    };
    use rusqlite::ErrorCode;

    struct CountingColumnStore {
        metadata: WorldMetadata,
        loads: Arc<AtomicUsize>,
        stored: StoredColumn,
    }

    impl ChunkStore for CountingColumnStore {
        fn metadata(&self) -> &WorldMetadata {
            &self.metadata
        }

        fn load_chunk(
            &self,
            _position: ChunkPos,
        ) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
            panic!("column loading must not fall back to one store call per subchunk")
        }

        fn load_stored_column(&self, column: ChunkColumn) -> ChunkStoreResult<StoredColumn> {
            assert_eq!(column, self.stored.position());
            self.loads.fetch_add(1, Ordering::Relaxed);
            Ok(self.stored.clone())
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
    fn column_loading_reads_storage_once_and_generates_every_missing_y() {
        let metadata = WorldMetadata::with_seed(77).with_height_chunks(3).unwrap();
        let position = ChunkColumn::new(-5, 8);
        let mut persisted = Chunk::default();
        persisted.set_cell_xyz(0, 0, 0, BlockType::Glowstone.into());
        let heightmap = ChunkHeightmap {
            heights: [[19; crate::world::chunk::CHUNK_SIZE]; crate::world::chunk::CHUNK_SIZE],
        };
        let stored = StoredColumn::try_new(
            position,
            metadata.height(),
            heightmap,
            vec![StoredChunk::new(position.chunk(1), persisted.clone())],
        )
        .unwrap();
        let loads = Arc::new(AtomicUsize::new(0));
        let repository = ChunkRepository::new(CountingColumnStore {
            metadata: metadata.clone(),
            loads: loads.clone(),
            stored,
        });

        let loaded = load_or_generate_column(position, repository).unwrap();

        assert_eq!(loads.load(Ordering::Relaxed), 1);
        assert_eq!(loaded.position, position);
        assert_eq!(loaded.heightmap, heightmap);
        assert_eq!(loaded.chunks().len(), 3);
        for (y, chunk) in loaded.chunks().iter().enumerate() {
            assert_eq!(chunk.position, position.chunk(y as i32));
        }
        assert_eq!(loaded.chunks()[0].source, ChunkLoadSource::Generated);
        assert_eq!(loaded.chunks()[1].source, ChunkLoadSource::Stored);
        assert_eq!(loaded.chunks()[1].chunk, persisted);
        assert_eq!(loaded.chunks()[2].source, ChunkLoadSource::Generated);
        assert_eq!(
            loaded.chunks()[0].chunk,
            generate_chunk(&metadata, position.chunk(0).as_ivec3())
        );
        assert_eq!(
            loaded.chunks()[2].chunk,
            generate_chunk(&metadata, position.chunk(2).as_ivec3())
        );
    }

    #[test]
    fn empty_stored_columns_produce_deterministic_full_generated_columns() {
        let metadata = WorldMetadata::with_seed(91).with_height_chunks(2).unwrap();
        let repository = ChunkRepository::new(InMemoryChunkStore::new(metadata.clone()));
        let position = ChunkColumn::new(3, -6);

        let first = load_or_generate_column(position, repository.clone()).unwrap();
        let second = load_or_generate_column(position, repository.clone()).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.chunks().len(), metadata.height_chunks());
        assert!(
            first
                .chunks()
                .iter()
                .all(|chunk| chunk.source == ChunkLoadSource::Generated)
        );
        assert!(
            repository
                .load_stored_column(position)
                .unwrap()
                .chunks()
                .is_empty()
        );
    }

    #[test]
    fn load_error_classification_is_explicit() {
        let transient = classify_load_error(ChunkStoreError::Sqlite {
            code: Some(ErrorCode::DatabaseBusy),
            extended_code: Some(5),
            message: "database is busy".to_owned(),
        });
        let permanent = classify_load_error(ChunkStoreError::Sqlite {
            code: Some(ErrorCode::DatabaseCorrupt),
            extended_code: Some(11),
            message: "database disk image is malformed".to_owned(),
        });
        let permission = classify_load_error(ChunkStoreError::Io {
            kind: ErrorKind::PermissionDenied,
            message: "permission denied".to_owned(),
        });

        assert_eq!(transient.kind, ChunkLoadErrorKind::Transient);
        assert_eq!(permanent.kind, ChunkLoadErrorKind::Permanent);
        assert_eq!(permission.kind, ChunkLoadErrorKind::Permanent);
    }

    #[cfg(feature = "turso-store")]
    #[test]
    fn turso_busy_errors_are_transient() {
        let error = classify_load_error(ChunkStoreError::Turso {
            kind: TursoStoreErrorKind::Busy,
            message: "database is busy".to_owned(),
        });

        assert_eq!(error.kind, ChunkLoadErrorKind::Transient);
    }
}
