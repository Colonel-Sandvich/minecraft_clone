use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use bevy::prelude::*;
use rusqlite::{Connection, OptionalExtension, params};

use crate::world::{
    chunk::{Chunk, ChunkColumn, ChunkHeightmap, ChunkPos},
    generation::WorldMetadata,
};

use super::{
    ChunkStore, ChunkStoreResult, SQL_CREATE_WORLD_METADATA, SQL_INSERT_METADATA_VALUE,
    SQL_SELECT_METADATA_VALUE, StoredChunk, StoredColumn, metadata_entries,
};

const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const SQLITE_CACHE_SIZE_KIB: i32 = 16 * 1024;
const SQLITE_MMAP_SIZE_BYTES: i64 = 256 * 1024 * 1024;
const SQLITE_WAL_AUTOCHECKPOINT_PAGES: i32 = 1_000;

const SQL_CREATE_CHUNKS: &str = "CREATE TABLE IF NOT EXISTS chunks (
    x INTEGER NOT NULL,
    z INTEGER NOT NULL,
    y INTEGER NOT NULL,
    blocks BLOB NOT NULL,
    PRIMARY KEY (x, z, y)
) WITHOUT ROWID";
const SQL_SELECT_CHUNK: &str = "SELECT blocks FROM chunks WHERE x = ?1 AND z = ?2 AND y = ?3";
const SQL_SELECT_COLUMN: &str = "SELECT y, blocks FROM chunks
WHERE x = ?1 AND z = ?2 AND y >= 0 AND y < ?3
ORDER BY y";
const SQL_UPSERT_CHUNK: &str = "INSERT INTO chunks (x, z, y, blocks)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(x, z, y) DO UPDATE SET blocks = excluded.blocks";

const SQL_CREATE_COLUMN_HEIGHTMAPS: &str = "CREATE TABLE IF NOT EXISTS column_heightmaps (
    x INTEGER NOT NULL,
    z INTEGER NOT NULL,
    heightmap BLOB NOT NULL,
    PRIMARY KEY (x, z)
) WITHOUT ROWID";
const SQL_SELECT_COLUMN_HEIGHTMAP: &str =
    "SELECT heightmap FROM column_heightmaps WHERE x = ?1 AND z = ?2";
const SQL_UPSERT_COLUMN_HEIGHTMAP: &str = "INSERT INTO column_heightmaps (x, z, heightmap)
    VALUES (?1, ?2, ?3)
    ON CONFLICT(x, z) DO UPDATE SET heightmap = excluded.heightmap";

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
        connection.execute(SQL_CREATE_CHUNKS, [])?;
        connection.execute(SQL_CREATE_COLUMN_HEIGHTMAPS, [])?;

        for (key, value) in metadata_entries(metadata) {
            ensure_metadata_value(connection, key, value)?;
        }

        Ok(())
    }
}

impl ChunkStore for SqliteChunkStore {
    fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    fn load_chunk(&self, pos: IVec3) -> ChunkStoreResult<Option<(Chunk, ChunkHeightmap)>> {
        let connection = self.open_connection()?;
        let bytes = connection
            .query_row(SQL_SELECT_CHUNK, params![pos.x, pos.z, pos.y], |row| {
                row.get::<_, Vec<u8>>(0)
            })
            .optional()?;

        let Some(bytes) = bytes else {
            return Ok(None);
        };

        let chunk = Chunk::try_from_storage_bytes(&bytes)?;
        let heightmap = load_column_heightmap(&connection, pos.x, pos.z)?;

        Ok(Some((chunk, heightmap)))
    }

    fn load_stored_column(&self, column: ChunkColumn) -> ChunkStoreResult<StoredColumn> {
        let connection = self.open_connection()?;
        let mut statement = connection.prepare(SQL_SELECT_COLUMN)?;
        let rows = statement.query_map(
            params![column.x(), column.z(), self.metadata.height().chunks_i32()],
            |row| Ok((row.get::<_, i32>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )?;

        let mut chunks = Vec::new();
        for row in rows {
            let (y, bytes) = row?;
            let chunk = Chunk::try_from_storage_bytes(&bytes)?;
            chunks.push(StoredChunk::new(
                ChunkPos::new(column.x(), y, column.z()),
                chunk,
            ));
        }
        let heightmap = load_column_heightmap(&connection, column.x(), column.z())?;

        StoredColumn::try_new(column, self.metadata.height(), heightmap, chunks).map_err(Into::into)
    }

    fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        heightmap: &ChunkHeightmap,
    ) -> ChunkStoreResult<()> {
        let mut connection = self.open_connection()?;
        let blocks = chunk.to_storage_bytes();
        let tx = connection.transaction()?;
        tx.execute(SQL_UPSERT_CHUNK, params![pos.x, pos.z, pos.y, &blocks])?;
        save_column_heightmap(&tx, pos.x, pos.z, heightmap)?;
        tx.commit()?;

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
    PathBuf::from("saves")
        .join("dev")
        .join(format!("seed-{:016x}.sqlite3", metadata.seed))
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
    x: i32,
    z: i32,
) -> ChunkStoreResult<ChunkHeightmap> {
    let bytes = connection
        .query_row(SQL_SELECT_COLUMN_HEIGHTMAP, params![x, z], |row| {
            row.get::<_, Vec<u8>>(0)
        })
        .optional()?;

    Ok(bytes
        .as_deref()
        .map(ChunkHeightmap::from_bytes)
        .unwrap_or_default())
}

fn save_column_heightmap(
    connection: &Connection,
    x: i32,
    z: i32,
    heightmap: &ChunkHeightmap,
) -> ChunkStoreResult<()> {
    let bytes = heightmap.to_bytes();
    connection.execute(SQL_UPSERT_COLUMN_HEIGHTMAP, params![x, z, &bytes])?;
    Ok(())
}
