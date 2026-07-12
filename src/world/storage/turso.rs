use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use crate::world::{
    chunk::{Chunk, ChunkColumn, ChunkHeightmap, ChunkPos},
    generation::WorldMetadata,
};

use super::{
    ChunkStore, ChunkStoreError, ChunkStoreResult, SQL_CREATE_WORLD_METADATA,
    SQL_INSERT_METADATA_VALUE, SQL_SELECT_METADATA_VALUE, StoredChunk, StoredColumn,
    metadata_entries,
};

const SQL_CREATE_CHUNKS: &str = "CREATE TABLE IF NOT EXISTS chunks (
    x INTEGER NOT NULL,
    z INTEGER NOT NULL,
    y INTEGER NOT NULL,
    blocks BLOB NOT NULL,
    PRIMARY KEY (x, z, y)
)";
const SQL_SELECT_CHUNK: &str = "SELECT blocks FROM chunks WHERE x = ?1 AND z = ?2 AND y = ?3";
const SQL_SELECT_COLUMN: &str = "SELECT y, blocks FROM chunks
WHERE x = ?1 AND z = ?2 AND y >= 0 AND y < ?3
ORDER BY y";
const SQL_UPDATE_CHUNK: &str = "UPDATE chunks
SET blocks = ?4
WHERE x = ?1 AND z = ?2 AND y = ?3";
const SQL_INSERT_CHUNK: &str = "INSERT INTO chunks (
    x, z, y, blocks
) VALUES (?1, ?2, ?3, ?4)";

const SQL_CREATE_COLUMN_HEIGHTMAPS: &str = "CREATE TABLE IF NOT EXISTS column_heightmaps (
    x INTEGER NOT NULL,
    z INTEGER NOT NULL,
    heightmap BLOB NOT NULL,
    PRIMARY KEY (x, z)
)";
const SQL_SELECT_COLUMN_HEIGHTMAP: &str =
    "SELECT heightmap FROM column_heightmaps WHERE x = ?1 AND z = ?2";
const SQL_UPSERT_COLUMN_HEIGHTMAP: &str = "INSERT INTO column_heightmaps (x, z, heightmap)
    VALUES (?1, ?2, ?3)
    ON CONFLICT(x, z) DO UPDATE SET heightmap = excluded.heightmap";

pub struct TursoChunkStore {
    database: turso::Database,
    runtime: tokio::runtime::Runtime,
    metadata: WorldMetadata,
}

impl TursoChunkStore {
    pub fn open(path: impl AsRef<Path>, metadata: &WorldMetadata) -> ChunkStoreResult<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let path = path_to_string(path)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|error| ChunkStoreError::Runtime {
                message: error.to_string(),
            })?;
        let database = runtime.block_on(async {
            turso::Builder::new_local(&path)
                .experimental_multiprocess_wal(true)
                .build()
                .await
        })?;
        let store = Self {
            database,
            runtime,
            metadata: metadata.clone(),
        };
        store.runtime.block_on(store.initialize(metadata))?;
        Ok(store)
    }

    async fn initialize(&self, metadata: &WorldMetadata) -> ChunkStoreResult<()> {
        let connection = self.database.connect()?;
        connection.execute(SQL_CREATE_WORLD_METADATA, ()).await?;
        connection.execute(SQL_CREATE_CHUNKS, ()).await?;
        connection.execute(SQL_CREATE_COLUMN_HEIGHTMAPS, ()).await?;

        for (key, value) in metadata_entries(metadata) {
            ensure_metadata_value(&connection, key, value).await?;
        }

        Ok(())
    }
}

impl ChunkStore for TursoChunkStore {
    fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    fn load_chunk(&self, position: ChunkPos) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        self.runtime.block_on(async {
            let connection = self.database.connect()?;
            let bytes = {
                let mut rows = connection
                    .query(SQL_SELECT_CHUNK, (position.x(), position.z(), position.y()))
                    .await?;
                let Some(row) = rows.next().await? else {
                    return Ok(None);
                };
                row.get::<Vec<u8>>(0)?
            };
            let chunk = Chunk::try_from_storage_bytes(&bytes)?;
            let heightmap = load_column_heightmap(&connection, position.x(), position.z()).await?;

            Ok(Some((chunk, heightmap)))
        })
    }

    fn load_stored_column(&self, column: ChunkColumn) -> ChunkStoreResult<StoredColumn> {
        self.runtime.block_on(async {
            let connection = self.database.connect()?;
            let mut rows = connection
                .query(
                    SQL_SELECT_COLUMN,
                    (column.x(), column.z(), self.metadata.height().chunks_i32()),
                )
                .await?;
            let mut chunks = Vec::new();
            while let Some(row) = rows.next().await? {
                let y = row.get::<i32>(0)?;
                let bytes = row.get::<Vec<u8>>(1)?;
                let chunk = Chunk::try_from_storage_bytes(&bytes)?;
                chunks.push(StoredChunk::new(
                    ChunkPos::new(column.x(), y, column.z()),
                    chunk,
                ));
            }
            let heightmap = load_column_heightmap(&connection, column.x(), column.z()).await?;

            StoredColumn::try_new(column, self.metadata.height(), heightmap, chunks)
                .map_err(Into::into)
        })
    }

    fn save_chunk(
        &self,
        position: ChunkPos,
        chunk: &Chunk,
        heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()> {
        let blocks = chunk.to_storage_bytes();
        let heightmap_bytes = heightmap.to_bytes();

        self.runtime.block_on(async {
            let mut connection = self.database.connect()?;
            let transaction = connection.transaction().await?;
            let changed = transaction
                .execute(
                    SQL_UPDATE_CHUNK,
                    (position.x(), position.z(), position.y(), blocks.clone()),
                )
                .await?;
            if changed == 0 {
                transaction
                    .execute(
                        SQL_INSERT_CHUNK,
                        (position.x(), position.z(), position.y(), blocks),
                    )
                    .await?;
            }
            save_column_heightmap(&transaction, position.x(), position.z(), &heightmap_bytes)
                .await?;
            transaction.commit().await?;

            Ok(())
        })
    }
}

async fn ensure_metadata_value(
    connection: &turso::Connection,
    key: &'static str,
    expected: String,
) -> ChunkStoreResult<()> {
    let mut rows = connection.query(SQL_SELECT_METADATA_VALUE, (key,)).await?;
    let Some(row) = rows.next().await? else {
        connection
            .execute(SQL_INSERT_METADATA_VALUE, (key, expected))
            .await?;
        return Ok(());
    };

    let existing = row.get::<String>(0)?;
    if existing == expected {
        Ok(())
    } else {
        Err(ChunkStoreError::WorldMetadataMismatch {
            key: key.to_owned(),
            expected,
            found: existing,
        })
    }
}

fn path_to_string(path: &Path) -> ChunkStoreResult<String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| ChunkStoreError::Io {
            kind: ErrorKind::InvalidInput,
            message: format!("database path is not valid UTF-8: {}", path.display()),
        })
}

pub fn development_turso_path(metadata: &WorldMetadata) -> PathBuf {
    PathBuf::from("saves")
        .join("dev")
        .join(format!("seed-{:016x}.turso", metadata.seed))
}

async fn load_column_heightmap(
    connection: &turso::Connection,
    x: i32,
    z: i32,
) -> ChunkStoreResult<ChunkHeightmap> {
    let mut rows = connection
        .query(SQL_SELECT_COLUMN_HEIGHTMAP, (x, z))
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(ChunkHeightmap::default());
    };
    let bytes = row.get::<Vec<u8>>(0)?;
    Ok(ChunkHeightmap::from_bytes(&bytes))
}

async fn save_column_heightmap(
    connection: &turso::Connection,
    x: i32,
    z: i32,
    heightmap_bytes: &[u8],
) -> ChunkStoreResult<()> {
    connection
        .execute(
            SQL_UPSERT_COLUMN_HEIGHTMAP,
            (x, z, heightmap_bytes.to_vec()),
        )
        .await?;
    Ok(())
}
