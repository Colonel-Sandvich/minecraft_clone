use std::{collections::HashMap, sync::Mutex};

use bevy::prelude::*;

use crate::world::{chunk::Chunk, generation::WorldMetadata};

use super::{ChunkStore, ChunkStoreError, ChunkStoreResult, StoredChunk};

pub struct InMemoryChunkStore {
    metadata: WorldMetadata,
    inner: Mutex<InMemoryChunkStoreInner>,
}

#[derive(Default)]
struct InMemoryChunkStoreInner {
    chunks: HashMap<IVec3, Vec<u8>>,
}

impl InMemoryChunkStore {
    pub fn new(metadata: WorldMetadata) -> Self {
        Self {
            metadata,
            inner: Mutex::default(),
        }
    }
}

impl Default for InMemoryChunkStore {
    fn default() -> Self {
        Self::new(WorldMetadata::default())
    }
}

impl ChunkStore for InMemoryChunkStore {
    fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    fn load_chunk(&self, pos: IVec3) -> ChunkStoreResult<Option<Chunk>> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;

        let Some(bytes) = inner.chunks.get(&pos) else {
            return Ok(None);
        };

        Ok(Some(Chunk::try_from_storage_bytes(bytes)?))
    }

    fn load_stored_column(&self, column: IVec2) -> ChunkStoreResult<Vec<StoredChunk>> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;

        let mut chunks = inner
            .chunks
            .iter()
            .filter(|(pos, _)| pos.x == column.x && pos.z == column.y)
            .map(|(pos, bytes)| {
                Ok(StoredChunk {
                    pos: *pos,
                    chunk: Chunk::try_from_storage_bytes(bytes)?,
                })
            })
            .collect::<ChunkStoreResult<Vec<_>>>()?;
        chunks.sort_by_key(|chunk| chunk.pos.y);

        Ok(chunks)
    }

    fn save_chunk(&self, pos: IVec3, chunk: &Chunk) -> ChunkStoreResult<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;

        inner.chunks.insert(pos, chunk.to_storage_bytes());
        Ok(())
    }
}

pub struct NoopChunkStore {
    metadata: WorldMetadata,
}

impl NoopChunkStore {
    pub const fn new(metadata: WorldMetadata) -> Self {
        Self { metadata }
    }
}

impl Default for NoopChunkStore {
    fn default() -> Self {
        Self::new(WorldMetadata::default())
    }
}

impl ChunkStore for NoopChunkStore {
    fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    fn load_chunk(&self, _pos: IVec3) -> ChunkStoreResult<Option<Chunk>> {
        Ok(None)
    }

    fn load_stored_column(&self, _column: IVec2) -> ChunkStoreResult<Vec<StoredChunk>> {
        Ok(Vec::new())
    }

    fn save_chunk(&self, _pos: IVec3, _chunk: &Chunk) -> ChunkStoreResult<()> {
        Ok(())
    }
}
