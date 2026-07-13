use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use rusqlite::{Connection, OptionalExtension, params};

use crate::player::PlayerId;
use crate::world::{
    chunk::{Chunk, ChunkHeightmap, ChunkPos},
    definition::{ChunkAddress, ColumnAddress, DimensionId},
    generation::{WorldHeight, WorldMetadata},
};

use super::{
    ChunkStore, ChunkStoreResult, SQL_CREATE_WORLD_METADATA, SQL_INSERT_METADATA_VALUE,
    SQL_SELECT_METADATA_VALUE, StoredChunk, StoredColumn, StoredPlayer, StoredPlayerPosition,
    metadata_entries,
};

const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const SQLITE_CACHE_SIZE_KIB: i32 = 16 * 1024;
const SQLITE_MMAP_SIZE_BYTES: i64 = 256 * 1024 * 1024;
const SQLITE_WAL_AUTOCHECKPOINT_PAGES: i32 = 1_000;

const SQL_CREATE_CHUNKS: &str = "CREATE TABLE IF NOT EXISTS chunks (
    dimension INTEGER NOT NULL,
    x INTEGER NOT NULL,
    z INTEGER NOT NULL,
    y INTEGER NOT NULL,
    blocks BLOB NOT NULL,
    PRIMARY KEY (dimension, x, z, y)
) WITHOUT ROWID";
const SQL_SELECT_CHUNK: &str = "SELECT blocks FROM chunks
WHERE dimension = ?1 AND x = ?2 AND z = ?3 AND y = ?4";
const SQL_SELECT_COLUMN: &str = "SELECT y, blocks FROM chunks
WHERE dimension = ?1 AND x = ?2 AND z = ?3 AND y >= 0 AND y < ?4
ORDER BY y";
const SQL_UPSERT_CHUNK: &str = "INSERT INTO chunks (dimension, x, z, y, blocks)
VALUES (?1, ?2, ?3, ?4, ?5)
ON CONFLICT(dimension, x, z, y) DO UPDATE SET blocks = excluded.blocks";

const SQL_CREATE_COLUMN_HEIGHTMAPS: &str = "CREATE TABLE IF NOT EXISTS column_heightmaps (
    dimension INTEGER NOT NULL,
    x INTEGER NOT NULL,
    z INTEGER NOT NULL,
    heightmap BLOB NOT NULL,
    PRIMARY KEY (dimension, x, z)
) WITHOUT ROWID";
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
) WITHOUT ROWID";
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

pub struct SqliteChunkStore {
    path: PathBuf,
    metadata: WorldMetadata,
}

impl SqliteChunkStore {
    pub fn open(path: impl AsRef<Path>, metadata: &WorldMetadata) -> ChunkStoreResult<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let store = Self {
            path,
            metadata: metadata.clone(),
        };
        let connection = store.open_connection()?;
        Self::initialize(&connection, metadata)?;
        Ok(store)
    }

    fn open_connection(&self) -> ChunkStoreResult<Connection> {
        let connection = Connection::open(&self.path)?;
        configure_connection(&connection)?;
        Ok(connection)
    }

    fn initialize(connection: &Connection, metadata: &WorldMetadata) -> ChunkStoreResult<()> {
        configure_database(connection)?;

        connection.execute(SQL_CREATE_WORLD_METADATA, [])?;
        for (key, value) in metadata_entries(metadata) {
            ensure_metadata_value(connection, &key, value)?;
        }
        connection.execute(SQL_CREATE_CHUNKS, [])?;
        connection.execute(SQL_CREATE_COLUMN_HEIGHTMAPS, [])?;
        connection.execute(SQL_CREATE_PLAYERS, [])?;
        connection.execute(SQL_CREATE_PLAYERS_POSITION_INDEX, [])?;

        Ok(())
    }
}

impl ChunkStore for SqliteChunkStore {
    fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    fn load_chunk(
        &self,
        address: ChunkAddress,
    ) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        let connection = self.open_connection()?;
        let position = address.position();
        let bytes = connection
            .query_row(
                SQL_SELECT_CHUNK,
                params![
                    i64::from(address.dimension().get()),
                    position.x(),
                    position.z(),
                    position.y()
                ],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;

        let Some(bytes) = bytes else {
            return Ok(None);
        };

        let chunk = Chunk::try_from_storage_bytes(&bytes)?;
        let heightmap = load_column_heightmap(&connection, address.column())?;

        Ok(Some((chunk, heightmap)))
    }

    fn load_stored_column(
        &self,
        address: ColumnAddress,
        height: WorldHeight,
    ) -> ChunkStoreResult<StoredColumn> {
        let connection = self.open_connection()?;
        let mut statement = connection.prepare(SQL_SELECT_COLUMN)?;
        let column = address.column();
        let rows = statement.query_map(
            params![
                i64::from(address.dimension().get()),
                column.x(),
                column.z(),
                height.chunks_i32()
            ],
            |row| Ok((row.get::<_, i32>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )?;

        let mut chunks = Vec::new();
        for row in rows {
            let (y, bytes) = row?;
            let chunk = Chunk::try_from_storage_bytes(&bytes)?;
            chunks.push(StoredChunk::new(address.chunk(y), chunk));
        }
        let heightmap = load_column_heightmap(&connection, address)?;

        StoredColumn::try_new(address, height, heightmap, chunks).map_err(Into::into)
    }

    fn save_chunk(
        &self,
        address: ChunkAddress,
        chunk: &Chunk,
        heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()> {
        let mut connection = self.open_connection()?;
        let position = address.position();
        let blocks = chunk.to_storage_bytes();
        let tx = connection.transaction()?;
        tx.execute(
            SQL_UPSERT_CHUNK,
            params![
                i64::from(address.dimension().get()),
                position.x(),
                position.z(),
                position.y(),
                &blocks
            ],
        )?;
        save_column_heightmap(&tx, address.column(), heightmap)?;
        tx.commit()?;

        Ok(())
    }

    fn load_player(&self, id: PlayerId) -> ChunkStoreResult<Option<StoredPlayer>> {
        let connection = self.open_connection()?;
        let row = connection
            .query_row(SQL_SELECT_PLAYER, params![id.get()], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i32>(1)?,
                    row.get::<_, i32>(2)?,
                    row.get::<_, i32>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, f64>(6)?,
                ))
            })
            .optional()?;

        let Some((dimension, chunk_x, chunk_z, chunk_y, local_x, local_z, local_y)) = row else {
            return Ok(None);
        };
        let dimension = u32::try_from(dimension)
            .map(DimensionId::new)
            .map_err(|_| super::ChunkStoreError::InvalidPlayerDimension {
                player: id,
                value: dimension,
            })?;
        let position = StoredPlayerPosition::try_new(
            dimension,
            ChunkPos::new(chunk_x, chunk_y, chunk_z),
            bevy::math::DVec3::new(local_x, local_y, local_z),
        )?;
        Ok(Some(StoredPlayer::new(id, position)))
    }

    fn save_player(&self, player: &StoredPlayer) -> ChunkStoreResult<()> {
        let connection = self.open_connection()?;
        let position = player.position();
        let chunk = position.chunk();
        let local = position.local();
        connection.execute(
            SQL_UPSERT_PLAYER,
            params![
                player.id().get(),
                i64::from(position.dimension().get()),
                chunk.x(),
                chunk.z(),
                chunk.y(),
                local.x,
                local.z,
                local.y,
            ],
        )?;
        Ok(())
    }
}

fn configure_connection(connection: &Connection) -> ChunkStoreResult<()> {
    connection.busy_timeout(SQLITE_BUSY_TIMEOUT)?;
    connection.execute_batch(&format!(
        "PRAGMA synchronous = NORMAL;
        PRAGMA temp_store = MEMORY;
        PRAGMA cache_size = -{SQLITE_CACHE_SIZE_KIB};
        PRAGMA mmap_size = {SQLITE_MMAP_SIZE_BYTES};
        PRAGMA wal_autocheckpoint = {SQLITE_WAL_AUTOCHECKPOINT_PAGES};"
    ))?;

    Ok(())
}

fn configure_database(connection: &Connection) -> ChunkStoreResult<()> {
    connection.execute_batch("PRAGMA journal_mode = WAL;")?;

    Ok(())
}

pub fn development_world_path(metadata: &WorldMetadata) -> PathBuf {
    PathBuf::from("saves").join("dev").join(format!(
        "{}.sqlite3",
        super::development_store_stem(metadata)
    ))
}

fn ensure_metadata_value(
    connection: &Connection,
    key: &str,
    expected: String,
) -> ChunkStoreResult<()> {
    let existing = connection
        .query_row(SQL_SELECT_METADATA_VALUE, params![key], |row| {
            row.get::<_, String>(0)
        })
        .optional()?;

    match existing {
        Some(existing) if existing == expected => Ok(()),
        Some(existing) => Err(super::ChunkStoreError::WorldMetadataMismatch {
            key: key.to_owned(),
            expected,
            found: existing,
        }),
        None => {
            connection.execute(SQL_INSERT_METADATA_VALUE, params![key, expected])?;
            Ok(())
        }
    }
}

fn load_column_heightmap(
    connection: &Connection,
    address: ColumnAddress,
) -> ChunkStoreResult<ChunkHeightmap> {
    let column = address.column();
    let bytes = connection
        .query_row(
            SQL_SELECT_COLUMN_HEIGHTMAP,
            params![i64::from(address.dimension().get()), column.x(), column.z()],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()?;

    Ok(bytes
        .as_deref()
        .map(ChunkHeightmap::from_bytes)
        .unwrap_or_default())
}

fn save_column_heightmap(
    connection: &Connection,
    address: ColumnAddress,
    heightmap: &ChunkHeightmap,
) -> ChunkStoreResult<()> {
    let column = address.column();
    let bytes = heightmap.to_bytes();
    connection.execute(
        SQL_UPSERT_COLUMN_HEIGHTMAP,
        params![
            i64::from(address.dimension().get()),
            column.x(),
            column.z(),
            &bytes
        ],
    )?;
    Ok(())
}
