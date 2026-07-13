use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use crate::player::PlayerId;
use crate::world::{
    chunk::{Chunk, ChunkHeightmap, ChunkPos},
    definition::{ChunkAddress, ColumnAddress, DimensionId},
    generation::{WorldHeight, WorldMetadata},
};

use super::{
    ChunkStore, ChunkStoreError, ChunkStoreResult, SQL_CREATE_WORLD_METADATA,
    SQL_INSERT_METADATA_VALUE, SQL_SELECT_METADATA_VALUE, StoredChunk, StoredColumn, StoredPlayer,
    StoredPlayerPosition, metadata_entries,
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

const SQL_CREATE_PLAYERS: &str = "CREATE TABLE IF NOT EXISTS players (
    id INTEGER NOT NULL,
    dimension INTEGER NOT NULL,
    chunk_x INTEGER NOT NULL,
    chunk_z INTEGER NOT NULL,
    chunk_y INTEGER NOT NULL,
    local_x REAL NOT NULL CHECK (local_x >= 0.0 AND local_x < 16.0),
    local_z REAL NOT NULL CHECK (local_z >= 0.0 AND local_z < 16.0),
    local_y REAL NOT NULL CHECK (local_y >= 0.0 AND local_y < 16.0),
    PRIMARY KEY (id)
)";
const SQL_CREATE_PLAYERS_POSITION_INDEX: &str =
    "CREATE INDEX IF NOT EXISTS players_by_chunk_position
    ON players (dimension, chunk_x, chunk_z, chunk_y)";
const SQL_SELECT_PLAYER: &str = "SELECT
    dimension, chunk_x, chunk_z, chunk_y, local_x, local_z, local_y
    FROM players WHERE id = ?1";
const SQL_UPSERT_PLAYER: &str = "INSERT INTO players (
    id, dimension, chunk_x, chunk_z, chunk_y, local_x, local_z, local_y
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
ON CONFLICT(id) DO UPDATE SET
    dimension = excluded.dimension,
    chunk_x = excluded.chunk_x,
    chunk_z = excluded.chunk_z,
    chunk_y = excluded.chunk_y,
    local_x = excluded.local_x,
    local_z = excluded.local_z,
    local_y = excluded.local_y";

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
        connection.execute(SQL_CREATE_PLAYERS, ()).await?;
        connection
            .execute(SQL_CREATE_PLAYERS_POSITION_INDEX, ())
            .await?;

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

    #[cfg(test)]
    pub(super) fn drop_player_schema_for_test(&self) -> ChunkStoreResult<()> {
        self.runtime.block_on(async {
            let connection = self.database.connect()?;
            connection.execute("DROP TABLE players", ()).await?;
            Ok(())
        })
    }

    #[cfg(test)]
    pub(super) fn player_position_index_columns_for_test(&self) -> ChunkStoreResult<Vec<String>> {
        self.runtime.block_on(async {
            let connection = self.database.connect()?;
            let mut rows = connection
                .query("PRAGMA index_info('players_by_chunk_position')", ())
                .await?;
            let mut columns = Vec::new();
            while let Some(row) = rows.next().await? {
                columns.push(row.get::<String>(2)?);
            }
            Ok(columns)
        })
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

    fn load_player(&self, id: PlayerId) -> ChunkStoreResult<Option<StoredPlayer>> {
        self.runtime.block_on(async {
            let connection = self.database.connect()?;
            let mut rows = connection.query(SQL_SELECT_PLAYER, (id.get(),)).await?;
            let Some(row) = rows.next().await? else {
                return Ok(None);
            };
            let dimension_value = row.get::<i64>(0)?;
            let dimension = u32::try_from(dimension_value)
                .map(DimensionId::new)
                .map_err(|_| ChunkStoreError::InvalidPlayerDimension {
                    player: id,
                    value: dimension_value,
                })?;
            let chunk = ChunkPos::new(row.get::<i32>(1)?, row.get::<i32>(3)?, row.get::<i32>(2)?);
            let local =
                bevy::math::DVec3::new(row.get::<f64>(4)?, row.get::<f64>(6)?, row.get::<f64>(5)?);
            let position = StoredPlayerPosition::try_new(dimension, chunk, local)?;
            Ok(Some(StoredPlayer::new(id, position)))
        })
    }

    fn save_player(&self, player: &StoredPlayer) -> ChunkStoreResult<()> {
        self.runtime.block_on(async {
            let connection = self.database.connect()?;
            let position = player.position();
            let chunk = position.chunk();
            let local = position.local();
            connection
                .execute(
                    SQL_UPSERT_PLAYER,
                    (
                        player.id().get(),
                        i64::from(position.dimension().get()),
                        chunk.x(),
                        chunk.z(),
                        chunk.y(),
                        local.x,
                        local.z,
                        local.y,
                    ),
                )
                .await?;
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
        .join(format!("{}.turso", super::development_store_stem(metadata)))
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
