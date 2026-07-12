use std::{collections::HashMap, sync::Mutex};

use crate::world::{
    chunk::{Chunk, ChunkColumn, ChunkHeightmap, ChunkPos},
    generation::WorldMetadata,
};

use super::{ChunkStore, ChunkStoreError, ChunkStoreResult, StoredChunk, StoredColumn};

pub struct InMemoryChunkStore {
    metadata: WorldMetadata,
    inner: Mutex<InMemoryChunkStoreInner>,
}

#[derive(Default)]
struct InMemoryChunkStoreInner {
    chunks: HashMap<ChunkPos, Vec<u8>>,
    column_heightmaps: HashMap<ChunkColumn, Vec<u8>>,
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

    fn load_chunk(&self, position: ChunkPos) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;

        let Some(bytes) = inner.chunks.get(&position) else {
            return Ok(None);
        };

        let chunk = Chunk::try_from_storage_bytes(bytes)?;
        let heightmap = inner
            .column_heightmaps
            .get(&ChunkColumn::from(position))
            .map(|b| ChunkHeightmap::from_bytes(b))
            .unwrap_or_default();

        Ok(Some((chunk, heightmap)))
    }

    fn load_stored_column(&self, column: ChunkColumn) -> ChunkStoreResult<StoredColumn> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;

        let chunks = inner
            .chunks
            .iter()
            .filter(|(position, _)| ChunkColumn::from(**position) == column)
            .map(|(&position, bytes)| {
                let chunk = Chunk::try_from_storage_bytes(bytes)?;
                Ok(StoredChunk::new(position, chunk))
            })
            .collect::<ChunkStoreResult<Vec<_>>>()?;
        let heightmap = inner
            .column_heightmaps
            .get(&column)
            .map(|bytes| ChunkHeightmap::from_bytes(bytes))
            .unwrap_or_default();

        StoredColumn::try_new(column, self.metadata.height(), heightmap, chunks).map_err(Into::into)
    }

    fn save_chunk(
        &self,
        position: ChunkPos,
        chunk: &Chunk,
        heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;

        inner.chunks.insert(position, chunk.to_storage_bytes());
        inner
            .column_heightmaps
            .insert(ChunkColumn::from(position), heightmap.to_bytes());
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

    fn load_chunk(&self, _position: ChunkPos) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        Ok(None)
    }

    fn load_stored_column(&self, column: ChunkColumn) -> ChunkStoreResult<StoredColumn> {
        StoredColumn::empty(column, self.metadata.height()).map_err(Into::into)
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
