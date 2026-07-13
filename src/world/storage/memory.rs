use std::{collections::HashMap, sync::Mutex};

use crate::player::PlayerId;
use crate::world::{
    chunk::{Chunk, ChunkHeightmap},
    definition::{ChunkAddress, ColumnAddress},
    generation::{WorldHeight, WorldMetadata},
};

use super::{
    ChunkStore, ChunkStoreError, ChunkStoreResult, StoredChunk, StoredColumn, StoredPlayer,
};

pub struct InMemoryChunkStore {
    metadata: WorldMetadata,
    inner: Mutex<InMemoryChunkStoreInner>,
}

#[derive(Default)]
struct InMemoryChunkStoreInner {
    columns: HashMap<ColumnAddress, InMemoryStoredColumn>,
    players: HashMap<PlayerId, StoredPlayer>,
}

#[derive(Default)]
struct InMemoryStoredColumn {
    chunks: HashMap<i32, Vec<u8>>,
    heightmap: Vec<u8>,
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

    fn load_chunk(
        &self,
        address: ChunkAddress,
    ) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;

        let Some(column) = inner.columns.get(&address.column()) else {
            return Ok(None);
        };
        let Some(bytes) = column.chunks.get(&address.position().y()) else {
            return Ok(None);
        };

        let chunk = Chunk::try_from_storage_bytes(bytes)?;
        let heightmap = if column.heightmap.is_empty() {
            ChunkHeightmap::default()
        } else {
            ChunkHeightmap::from_bytes(&column.heightmap)
        };

        Ok(Some((chunk, heightmap)))
    }

    fn load_stored_column(
        &self,
        address: ColumnAddress,
        height: WorldHeight,
    ) -> ChunkStoreResult<StoredColumn> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;

        let Some(column) = inner.columns.get(&address) else {
            return StoredColumn::empty(address, height).map_err(Into::into);
        };
        let chunks = column
            .chunks
            .iter()
            .filter(|(y, _)| (0..height.chunks_i32()).contains(y))
            .map(|(&y, bytes)| {
                let chunk = Chunk::try_from_storage_bytes(bytes)?;
                Ok(StoredChunk::new(address.chunk(y), chunk))
            })
            .collect::<ChunkStoreResult<Vec<_>>>()?;
        let heightmap = if column.heightmap.is_empty() {
            ChunkHeightmap::default()
        } else {
            ChunkHeightmap::from_bytes(&column.heightmap)
        };

        StoredColumn::try_new(address, height, heightmap, chunks).map_err(Into::into)
    }

    fn save_chunk(
        &self,
        address: ChunkAddress,
        chunk: &Chunk,
        heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;

        let column = inner.columns.entry(address.column()).or_default();
        column
            .chunks
            .insert(address.position().y(), chunk.to_storage_bytes());
        column.heightmap = heightmap.to_bytes();
        Ok(())
    }

    fn load_player(&self, id: PlayerId) -> ChunkStoreResult<Option<StoredPlayer>> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;
        Ok(inner.players.get(&id).cloned())
    }

    fn save_player(&self, player: &StoredPlayer) -> ChunkStoreResult<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;
        inner.players.insert(player.id(), player.clone());
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

    fn load_chunk(
        &self,
        _address: ChunkAddress,
    ) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        Ok(None)
    }

    fn load_stored_column(
        &self,
        address: ColumnAddress,
        height: WorldHeight,
    ) -> ChunkStoreResult<StoredColumn> {
        StoredColumn::empty(address, height).map_err(Into::into)
    }

    fn save_chunk(
        &self,
        _address: ChunkAddress,
        _chunk: &Chunk,
        _heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()> {
        Ok(())
    }

    fn load_player(&self, _id: PlayerId) -> ChunkStoreResult<Option<StoredPlayer>> {
        Ok(None)
    }

    fn save_player(&self, _player: &StoredPlayer) -> ChunkStoreResult<()> {
        Ok(())
    }
}
