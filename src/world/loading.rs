use std::io::ErrorKind;

use bevy::prelude::*;
use rusqlite::ErrorCode;

use crate::world::{
    chunk::Chunk,
    generation::{WorldMetadata, generate_chunk},
    storage::{ChunkRepository, ChunkStoreError},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkLoadRequest {
    pub pos: IVec3,
    pub metadata: WorldMetadata,
}

impl ChunkLoadRequest {
    pub const fn new(pos: IVec3, metadata: WorldMetadata) -> Self {
        Self { pos, metadata }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkLoadOutput {
    pub pos: IVec3,
    pub result: ChunkLoadResult,
}

pub type ChunkLoadResult = Result<LoadedChunk, ChunkLoadError>;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedChunk {
    pub chunk: Chunk,
    pub source: ChunkLoadSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkLoadSource {
    Stored,
    Generated,
}

pub fn load_or_generate_chunk(
    request: ChunkLoadRequest,
    repository: ChunkRepository,
) -> ChunkLoadOutput {
    match repository.load_chunk(request.pos, &request.metadata) {
        Ok(Some(chunk)) => ChunkLoadOutput {
            pos: request.pos,
            result: Ok(LoadedChunk {
                chunk,
                source: ChunkLoadSource::Stored,
            }),
        },
        Ok(None) => {
            let chunk = generate_chunk(&request.metadata, request.pos);

            ChunkLoadOutput {
                pos: request.pos,
                result: Ok(LoadedChunk {
                    chunk,
                    source: ChunkLoadSource::Generated,
                }),
            }
        }
        Err(error) => ChunkLoadOutput {
            pos: request.pos,
            result: Err(classify_load_error(error)),
        },
    }
}

pub fn classify_load_error(error: ChunkStoreError) -> ChunkLoadError {
    match &error {
        ChunkStoreError::Sqlite { code, .. } if is_transient_sqlite_error(*code) => {
            ChunkLoadError::transient(error)
        }
        ChunkStoreError::Io { kind, .. } if is_transient_io_error(*kind) => {
            ChunkLoadError::transient(error)
        }
        _ => ChunkLoadError::permanent(error),
    }
}

fn is_transient_sqlite_error(code: Option<ErrorCode>) -> bool {
    matches!(
        code,
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked | ErrorCode::OperationInterrupted)
    )
}

fn is_transient_io_error(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::Interrupted | ErrorKind::TimedOut | ErrorKind::WouldBlock
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_chunks_are_not_saved_by_loading() {
        let metadata = WorldMetadata::with_seed(77);
        let repository = ChunkRepository::default();
        let pos = ivec3(3, 0, -1);
        let request = ChunkLoadRequest::new(pos, metadata.clone());

        let generated = load_or_generate_chunk(request.clone(), repository.clone());
        let stored = load_or_generate_chunk(request, repository.clone());

        let generated = generated.result.unwrap();
        let stored = stored.result.unwrap();

        assert_eq!(generated.source, ChunkLoadSource::Generated);
        assert_eq!(stored.source, ChunkLoadSource::Generated);
        assert_eq!(generated.chunk, stored.chunk);
        assert_eq!(repository.load_chunk(pos, &metadata).unwrap(), None);
    }

    #[test]
    fn storage_errors_are_not_regenerated_over() {
        let metadata = WorldMetadata::with_seed(77);
        let repository = ChunkRepository::default();
        let pos = ivec3(3, 0, -1);
        let mut incompatible = metadata.clone();
        incompatible.generator_version += 1;

        repository
            .save_chunk(pos, &Chunk::default(), &metadata)
            .unwrap();

        let loaded = load_or_generate_chunk(
            ChunkLoadRequest::new(pos, incompatible.clone()),
            repository.clone(),
        );

        let error = loaded.result.unwrap_err();
        assert_eq!(error.kind, ChunkLoadErrorKind::Permanent);
        assert!(matches!(
            error.source,
            ChunkStoreError::WorldMetadataMismatch { .. }
        ));
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
}
