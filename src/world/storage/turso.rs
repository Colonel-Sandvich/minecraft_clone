use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use crate::world::{
    chunk::{Chunk, ChunkHeightmap},
    definition::{ChunkAddress, ColumnAddress},
    generation::{WorldHeight, WorldMetadata},
};

use super::{
    ChunkStore, ChunkStoreError, ChunkStoreResult, SQL_CREATE_WORLD_METADATA,
    SQL_INSERT_METADATA_VALUE, SQL_SELECT_METADATA_VALUE, StoredChunk, StoredColumn,
    metadata_entries,
};

const SQL_CREATE_CHUNKS: &str = "CREATE TABLE IF NOT EXISTS chunks (
    dimension INTEGER NOT NULL,
    x INTEGER NOT NULL,
    z INTEGER NOT NULL,
    y INTEGER NOT NULL,
    blocks BLOB NOT NULL,
    PRIMARY KEY (dimension, x, z, y)
)";
const SQL_SELECT_CHUNK: &str = "SELECT blocks FROM chunks
WHERE dimension = ?1 AND x = ?2 AND z = ?3 AND y = ?4";
const SQL_SELECT_COLUMN: &str = "SELECT y, blocks FROM chunks
WHERE dimension = ?1 AND x = ?2 AND z = ?3 AND y >= 0 AND y < ?4
ORDER BY y";
const SQL_UPDATE_CHUNK: &str = "UPDATE chunks
SET blocks = ?5
WHERE dimension = ?1 AND x = ?2 AND z = ?3 AND y = ?4";
const SQL_INSERT_CHUNK: &str = "INSERT INTO chunks (
    dimension, x, z, y, blocks
) VALUES (?1, ?2, ?3, ?4, ?5)";

const SQL_CREATE_COLUMN_HEIGHTMAPS: &str = "CREATE TABLE IF NOT EXISTS column_heightmaps (
    dimension INTEGER NOT NULL,
    x INTEGER NOT NULL,
    z INTEGER NOT NULL,
    heightmap BLOB NOT NULL,
    PRIMARY KEY (dimension, x, z)
)";
const SQL_SELECT_COLUMN_HEIGHTMAP: &str =
    "SELECT heightmap FROM column_heightmaps WHERE dimension = ?1 AND x = ?2 AND z = ?3";
const SQL_UPSERT_COLUMN_HEIGHTMAP: &str =
    "INSERT INTO column_heightmaps (dimension, x, z, heightmap)
    VALUES (?1, ?2, ?3, ?4)
    ON CONFLICT(dimension, x, z) DO UPDATE SET heightmap = excluded.heightmap";

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
        for (key, value) in metadata_entries(metadata) {
            ensure_metadata_value(&connection, key, value).await?;
        }
        connection.execute(SQL_CREATE_CHUNKS, ()).await?;
        connection.execute(SQL_CREATE_COLUMN_HEIGHTMAPS, ()).await?;

        Ok(())
    }

    #[cfg(test)]
    pub(super) fn overwrite_metadata_for_test(&self, key: &str, value: &str) {
        self.runtime.block_on(async {
            let connection = self.database.connect().expect("test database must connect");
            connection
                .execute(
                    "UPDATE world_metadata SET value = ?2 WHERE key = ?1",
                    (key, value),
                )
                .await
                .expect("test metadata update must succeed");
        });
    }
}

impl ChunkStore for TursoChunkStore {
    fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    fn load_chunk(
        &self,
        address: ChunkAddress,
    ) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        self.runtime.block_on(async {
            let connection = self.database.connect()?;
            let position = address.position();
            let bytes = {
                let mut rows = connection
                    .query(
                        SQL_SELECT_CHUNK,
                        (
                            i64::from(address.dimension().get()),
                            position.x(),
                            position.z(),
                            position.y(),
                        ),
                    )
                    .await?;
                let Some(row) = rows.next().await? else {
                    return Ok(None);
                };
                row.get::<Vec<u8>>(0)?
            };
            let chunk = Chunk::try_from_storage_bytes(&bytes)?;
            let heightmap = load_column_heightmap(&connection, address.column()).await?;

            Ok(Some((chunk, heightmap)))
        })
    }

    fn load_stored_column(
        &self,
        address: ColumnAddress,
        height: WorldHeight,
    ) -> ChunkStoreResult<StoredColumn> {
        self.runtime.block_on(async {
            let connection = self.database.connect()?;
            let column = address.column();
            let mut rows = connection
                .query(
                    SQL_SELECT_COLUMN,
                    (
                        i64::from(address.dimension().get()),
                        column.x(),
                        column.z(),
                        height.chunks_i32(),
                    ),
                )
                .await?;
            let mut chunks = Vec::new();
            while let Some(row) = rows.next().await? {
                let y = row.get::<i32>(0)?;
                let bytes = row.get::<Vec<u8>>(1)?;
                let chunk = Chunk::try_from_storage_bytes(&bytes)?;
                chunks.push(StoredChunk::new(address.chunk(y), chunk));
            }
            let heightmap = load_column_heightmap(&connection, address).await?;

            StoredColumn::try_new(address, height, heightmap, chunks).map_err(Into::into)
        })
    }

    fn save_chunk(
        &self,
        address: ChunkAddress,
        chunk: &Chunk,
        heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()> {
        let blocks = chunk.to_storage_bytes();
        let heightmap_bytes = heightmap.to_bytes();

        self.runtime.block_on(async {
            let mut connection = self.database.connect()?;
            let transaction = connection.transaction().await?;
            let position = address.position();
            let dimension = i64::from(address.dimension().get());
            let changed = transaction
                .execute(
                    SQL_UPDATE_CHUNK,
                    (
                        dimension,
                        position.x(),
                        position.z(),
                        position.y(),
                        blocks.clone(),
                    ),
                )
                .await?;
            if changed == 0 {
                transaction
                    .execute(
                        SQL_INSERT_CHUNK,
                        (dimension, position.x(), position.z(), position.y(), blocks),
                    )
                    .await?;
            }
            save_column_heightmap(&transaction, address.column(), &heightmap_bytes).await?;
            transaction.commit().await?;

            Ok(())
        })
    }
}

async fn ensure_metadata_value(
    connection: &turso::Connection,
    key: String,
    expected: String,
) -> ChunkStoreResult<()> {
    let mut rows = connection
        .query(SQL_SELECT_METADATA_VALUE, (key.as_str(),))
        .await?;
    let Some(row) = rows.next().await? else {
        connection
            .execute(SQL_INSERT_METADATA_VALUE, (key.as_str(), expected))
            .await?;
        return Ok(());
    };

    let existing = row.get::<String>(0)?;
    if existing == expected {
        Ok(())
    } else {
        Err(ChunkStoreError::WorldMetadataMismatch {
            key,
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
    address: ColumnAddress,
) -> ChunkStoreResult<ChunkHeightmap> {
    let column = address.column();
    let mut rows = connection
        .query(
            SQL_SELECT_COLUMN_HEIGHTMAP,
            (i64::from(address.dimension().get()), column.x(), column.z()),
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(ChunkHeightmap::default());
    };
    let bytes = row.get::<Vec<u8>>(0)?;
    Ok(ChunkHeightmap::from_bytes(&bytes))
}

async fn save_column_heightmap(
    connection: &turso::Connection,
    address: ColumnAddress,
    heightmap_bytes: &[u8],
) -> ChunkStoreResult<()> {
    let column = address.column();
    connection
        .execute(
            SQL_UPSERT_COLUMN_HEIGHTMAP,
            (
                i64::from(address.dimension().get()),
                column.x(),
                column.z(),
                heightmap_bytes.to_vec(),
            ),
        )
        .await?;
    Ok(())
}
