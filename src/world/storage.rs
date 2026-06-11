use std::{
    collections::HashMap,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use bevy::prelude::*;
use rusqlite::{Connection, ErrorCode, OptionalExtension, params};

use crate::world::{
    chunk::{Chunk, ChunkDecodeError},
    generation::WorldMetadata,
};

pub type ChunkStoreResult<T> = Result<T, ChunkStoreError>;

const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const SQLITE_CACHE_SIZE_KIB: i32 = 16 * 1024;
const SQLITE_MMAP_SIZE_BYTES: i64 = 256 * 1024 * 1024;
const SQLITE_WAL_AUTOCHECKPOINT_PAGES: i32 = 1_000;

const SQL_CREATE_WORLD_METADATA: &str = "CREATE TABLE IF NOT EXISTS world_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
)";
const SQL_CREATE_SQLITE_CHUNKS: &str = "CREATE TABLE IF NOT EXISTS chunks (
    x INTEGER NOT NULL,
    z INTEGER NOT NULL,
    y INTEGER NOT NULL,
    blocks BLOB NOT NULL,
    PRIMARY KEY (x, z, y)
) WITHOUT ROWID";
#[cfg(feature = "turso-store")]
const SQL_CREATE_TURSO_CHUNKS: &str = "CREATE TABLE IF NOT EXISTS chunks (
    x INTEGER NOT NULL,
    z INTEGER NOT NULL,
    y INTEGER NOT NULL,
    blocks BLOB NOT NULL,
    PRIMARY KEY (x, z, y)
)";
const SQL_SELECT_METADATA_VALUE: &str = "SELECT value FROM world_metadata WHERE key = ?1";
const SQL_INSERT_METADATA_VALUE: &str = "INSERT INTO world_metadata (key, value) VALUES (?1, ?2)";
const SQL_SELECT_CHUNK: &str = "SELECT blocks FROM chunks WHERE x = ?1 AND z = ?2 AND y = ?3";
const SQL_UPDATE_CHUNK: &str = "UPDATE chunks
SET blocks = ?4
WHERE x = ?1 AND z = ?2 AND y = ?3";
const SQL_INSERT_CHUNK: &str = "INSERT INTO chunks (
    x, z, y, blocks
) VALUES (?1, ?2, ?3, ?4)";

pub trait ChunkStore: Send + Sync + 'static {
    fn load_chunk(&self, pos: IVec3, metadata: &WorldMetadata) -> ChunkStoreResult<Option<Chunk>>;
    fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<()>;
}

#[derive(Resource, Clone)]
pub struct ChunkRepository {
    store: Arc<dyn ChunkStore>,
}

impl ChunkRepository {
    pub fn new(store: impl ChunkStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }

    pub fn load_chunk(
        &self,
        pos: IVec3,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<Option<Chunk>> {
        self.store.load_chunk(pos, metadata)
    }

    pub fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<()> {
        self.store.save_chunk(pos, chunk, metadata)
    }
}

impl Default for ChunkRepository {
    fn default() -> Self {
        Self::new(InMemoryChunkStore::default())
    }
}

#[derive(Default)]
pub struct InMemoryChunkStore {
    inner: Mutex<InMemoryChunkStoreInner>,
}

#[derive(Default)]
struct InMemoryChunkStoreInner {
    metadata: Option<WorldMetadata>,
    chunks: HashMap<IVec3, Vec<u8>>,
}

impl ChunkStore for InMemoryChunkStore {
    fn load_chunk(&self, pos: IVec3, metadata: &WorldMetadata) -> ChunkStoreResult<Option<Chunk>> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;
        ensure_store_metadata(&mut inner.metadata, metadata)?;

        let Some(bytes) = inner.chunks.get(&pos) else {
            return Ok(None);
        };

        Ok(Some(Chunk::try_from_storage_bytes(bytes)?))
    }

    fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| ChunkStoreError::LockPoisoned {
                store: "in-memory chunk store",
            })?;
        ensure_store_metadata(&mut inner.metadata, metadata)?;

        inner.chunks.insert(pos, chunk.to_storage_bytes());
        Ok(())
    }
}

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
        configure_sqlite_connection(&connection)?;
        Ok(connection)
    }

    fn initialize(connection: &Connection, metadata: &WorldMetadata) -> ChunkStoreResult<()> {
        configure_sqlite_database(connection)?;

        connection.execute(SQL_CREATE_WORLD_METADATA, [])?;
        connection.execute(SQL_CREATE_SQLITE_CHUNKS, [])?;

        for (key, value) in metadata_entries(metadata) {
            ensure_metadata_value(connection, key, value)?;
        }

        Ok(())
    }
}

impl ChunkStore for SqliteChunkStore {
    fn load_chunk(&self, pos: IVec3, metadata: &WorldMetadata) -> ChunkStoreResult<Option<Chunk>> {
        validate_world_metadata(&self.metadata, metadata)?;

        let connection = self.open_connection()?;
        let bytes = connection
            .query_row(SQL_SELECT_CHUNK, params![pos.x, pos.z, pos.y], |row| {
                row.get::<_, Vec<u8>>(0)
            })
            .optional()?;

        let Some(bytes) = bytes else {
            return Ok(None);
        };

        Ok(Some(Chunk::try_from_storage_bytes(&bytes)?))
    }

    fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<()> {
        validate_world_metadata(&self.metadata, metadata)?;

        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let blocks = chunk.to_storage_bytes();
        if transaction.execute(SQL_UPDATE_CHUNK, params![pos.x, pos.z, pos.y, &blocks])? == 0 {
            transaction.execute(SQL_INSERT_CHUNK, params![pos.x, pos.z, pos.y, &blocks])?;
        }
        transaction.commit()?;

        Ok(())
    }
}

#[cfg(feature = "turso-store")]
pub struct TursoChunkStore {
    database: turso::Database,
    runtime: tokio::runtime::Runtime,
    metadata: WorldMetadata,
}

#[cfg(feature = "turso-store")]
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
        connection.execute(SQL_CREATE_TURSO_CHUNKS, ()).await?;

        for (key, value) in metadata_entries(metadata) {
            ensure_turso_metadata_value(&connection, key, value).await?;
        }

        Ok(())
    }
}

#[cfg(feature = "turso-store")]
impl ChunkStore for TursoChunkStore {
    fn load_chunk(&self, pos: IVec3, metadata: &WorldMetadata) -> ChunkStoreResult<Option<Chunk>> {
        validate_world_metadata(&self.metadata, metadata)?;

        self.runtime.block_on(async {
            let connection = self.database.connect()?;
            let mut rows = connection
                .query(SQL_SELECT_CHUNK, (pos.x, pos.z, pos.y))
                .await?;
            let Some(row) = rows.next().await? else {
                return Ok(None);
            };
            let bytes = row.get::<Vec<u8>>(0)?;

            Ok(Some(Chunk::try_from_storage_bytes(&bytes)?))
        })
    }

    fn save_chunk(
        &self,
        pos: IVec3,
        chunk: &Chunk,
        metadata: &WorldMetadata,
    ) -> ChunkStoreResult<()> {
        validate_world_metadata(&self.metadata, metadata)?;
        let blocks = chunk.to_storage_bytes();

        self.runtime.block_on(async {
            let mut connection = self.database.connect()?;
            let transaction = connection.transaction().await?;
            let changed = transaction
                .execute(SQL_UPDATE_CHUNK, (pos.x, pos.z, pos.y, blocks.clone()))
                .await?;
            if changed == 0 {
                transaction
                    .execute(SQL_INSERT_CHUNK, (pos.x, pos.z, pos.y, blocks))
                    .await?;
            }
            transaction.commit().await?;

            Ok(())
        })
    }
}

#[cfg(feature = "turso-store")]
async fn ensure_turso_metadata_value(
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

#[cfg(feature = "turso-store")]
fn path_to_string(path: &Path) -> ChunkStoreResult<String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| ChunkStoreError::Io {
            kind: ErrorKind::InvalidInput,
            message: format!("database path is not valid UTF-8: {}", path.display()),
        })
}

fn configure_sqlite_connection(connection: &Connection) -> ChunkStoreResult<()> {
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

fn configure_sqlite_database(connection: &Connection) -> ChunkStoreResult<()> {
    connection.execute_batch("PRAGMA journal_mode = WAL;")?;

    Ok(())
}

pub fn development_world_path(metadata: &WorldMetadata) -> PathBuf {
    PathBuf::from("saves")
        .join("dev")
        .join(format!("seed-{:016x}.sqlite3", metadata.seed))
}

fn metadata_entries(metadata: &WorldMetadata) -> [(&'static str, String); 4] {
    [
        ("seed", metadata.seed.to_string()),
        ("generator_version", metadata.generator_version.to_string()),
        (
            "chunk_format_version",
            metadata.chunk_format_version.to_string(),
        ),
        ("height_chunks", metadata.height_chunks.to_string()),
    ]
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
        Some(existing) => Err(ChunkStoreError::WorldMetadataMismatch {
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

fn ensure_store_metadata(
    stored: &mut Option<WorldMetadata>,
    metadata: &WorldMetadata,
) -> ChunkStoreResult<()> {
    if let Some(stored) = stored {
        validate_world_metadata(stored, metadata)
    } else {
        *stored = Some(metadata.clone());
        Ok(())
    }
}

fn validate_world_metadata(
    expected: &WorldMetadata,
    found: &WorldMetadata,
) -> ChunkStoreResult<()> {
    if expected.seed != found.seed {
        return Err(world_metadata_mismatch("seed", expected.seed, found.seed));
    }

    if expected.generator_version != found.generator_version {
        return Err(world_metadata_mismatch(
            "generator_version",
            expected.generator_version,
            found.generator_version,
        ));
    }

    if expected.chunk_format_version != found.chunk_format_version {
        return Err(world_metadata_mismatch(
            "chunk_format_version",
            expected.chunk_format_version,
            found.chunk_format_version,
        ));
    }

    if expected.height_chunks != found.height_chunks {
        return Err(world_metadata_mismatch(
            "height_chunks",
            expected.height_chunks,
            found.height_chunks,
        ));
    }

    Ok(())
}

fn world_metadata_mismatch(
    key: &str,
    expected: impl ToString,
    found: impl ToString,
) -> ChunkStoreError {
    ChunkStoreError::WorldMetadataMismatch {
        key: key.to_owned(),
        expected: expected.to_string(),
        found: found.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChunkStoreError {
    LockPoisoned {
        store: &'static str,
    },
    Sqlite {
        code: Option<ErrorCode>,
        extended_code: Option<i32>,
        message: String,
    },
    Io {
        kind: ErrorKind,
        message: String,
    },
    Decode(ChunkDecodeError),
    WorldMetadataMismatch {
        key: String,
        expected: String,
        found: String,
    },
    #[cfg(feature = "turso-store")]
    Turso {
        message: String,
    },
    #[cfg(feature = "turso-store")]
    Runtime {
        message: String,
    },
}

impl std::fmt::Display for ChunkStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LockPoisoned { store } => write!(f, "{store} lock poisoned"),
            Self::Sqlite {
                code,
                extended_code,
                message,
            } => write!(f, "sqlite error {code:?}/{extended_code:?}: {message}"),
            Self::Io { kind, message } => write!(f, "io error {kind:?}: {message}"),
            Self::Decode(error) => write!(f, "chunk decode error: {error}"),
            Self::WorldMetadataMismatch {
                key,
                expected,
                found,
            } => write!(
                f,
                "world metadata mismatch for {key}: expected {expected}, found {found}"
            ),
            #[cfg(feature = "turso-store")]
            Self::Turso { message } => write!(f, "turso error: {message}"),
            #[cfg(feature = "turso-store")]
            Self::Runtime { message } => write!(f, "runtime error: {message}"),
        }
    }
}

impl std::error::Error for ChunkStoreError {}

impl From<rusqlite::Error> for ChunkStoreError {
    fn from(value: rusqlite::Error) -> Self {
        let code = value.sqlite_error_code();
        let extended_code = match &value {
            rusqlite::Error::SqliteFailure(error, _) => Some(error.extended_code),
            _ => None,
        };
        Self::Sqlite {
            code,
            extended_code,
            message: value.to_string(),
        }
    }
}

impl From<std::io::Error> for ChunkStoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io {
            kind: value.kind(),
            message: value.to_string(),
        }
    }
}

impl From<ChunkDecodeError> for ChunkStoreError {
    fn from(value: ChunkDecodeError) -> Self {
        Self::Decode(value)
    }
}

#[cfg(feature = "turso-store")]
impl From<turso::Error> for ChunkStoreError {
    fn from(value: turso::Error) -> Self {
        Self::Turso {
            message: value.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::BlockType;
    use std::{
        ops::Deref,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_TEST_STORE_ID: AtomicU64 = AtomicU64::new(0);

    struct TestSqliteStore {
        store: SqliteChunkStore,
        path: PathBuf,
    }

    #[cfg(feature = "turso-store")]
    struct TestTursoStore {
        store: TursoChunkStore,
        path: PathBuf,
    }

    impl Deref for TestSqliteStore {
        type Target = SqliteChunkStore;

        fn deref(&self) -> &Self::Target {
            &self.store
        }
    }

    impl Drop for TestSqliteStore {
        fn drop(&mut self) {
            remove_test_store_files(&self.path);
        }
    }

    #[cfg(feature = "turso-store")]
    impl Deref for TestTursoStore {
        type Target = TursoChunkStore;

        fn deref(&self) -> &Self::Target {
            &self.store
        }
    }

    #[cfg(feature = "turso-store")]
    impl Drop for TestTursoStore {
        fn drop(&mut self) {
            remove_test_store_files(&self.path);
        }
    }

    fn test_store_path(prefix: &str) -> PathBuf {
        let id = NEXT_TEST_STORE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "minecraft_clone-{prefix}-{}-{id}.sqlite3",
            std::process::id()
        ))
    }

    fn remove_test_store_files(path: &Path) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{}", path.display(), suffix));
        }
    }

    fn test_sqlite_store(metadata: &WorldMetadata) -> TestSqliteStore {
        let path = test_store_path("sqlite-test");
        remove_test_store_files(&path);
        let store = SqliteChunkStore::open(&path, metadata).unwrap();

        TestSqliteStore { store, path }
    }

    #[cfg(feature = "turso-store")]
    fn test_turso_store(metadata: &WorldMetadata) -> TestTursoStore {
        let path = test_store_path("turso-test");
        remove_test_store_files(&path);
        let store = TursoChunkStore::open(&path, metadata).unwrap();

        TestTursoStore { store, path }
    }

    #[test]
    fn sqlite_store_roundtrips_full_chunks() {
        let metadata = WorldMetadata::with_seed(42);
        let store = test_sqlite_store(&metadata);
        let pos = ivec3(-2, 1, 3);
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::Grass;
        chunk.blocks[15][15][15] = BlockType::OakLeaves;

        store.save_chunk(pos, &chunk, &metadata).unwrap();

        assert_eq!(store.load_chunk(pos, &metadata).unwrap(), Some(chunk));
    }

    #[cfg(feature = "turso-store")]
    #[test]
    fn turso_store_roundtrips_full_chunks() {
        let metadata = WorldMetadata::with_seed(42);
        let store = test_turso_store(&metadata);
        let pos = ivec3(-2, 1, 3);
        let mut chunk = Chunk::default();
        chunk.blocks[0][0][0] = BlockType::Grass;
        chunk.blocks[15][15][15] = BlockType::OakLeaves;

        store.save_chunk(pos, &chunk, &metadata).unwrap();

        assert_eq!(store.load_chunk(pos, &metadata).unwrap(), Some(chunk));
    }

    #[test]
    fn sqlite_store_rejects_world_metadata_mismatch() {
        let metadata = WorldMetadata::with_seed(42);
        let store = test_sqlite_store(&metadata);
        let mut incompatible = metadata.clone();
        incompatible.height_chunks += 1;

        assert!(
            store
                .save_chunk(IVec3::ZERO, &Chunk::default(), &metadata)
                .is_ok()
        );
        assert!(store.load_chunk(IVec3::ZERO, &incompatible).is_err());
    }

    #[test]
    fn in_memory_store_rejects_world_metadata_mismatch() {
        let metadata = WorldMetadata::with_seed(42);
        let store = InMemoryChunkStore::default();
        let mut incompatible = metadata.clone();
        incompatible.generator_version += 1;

        assert!(
            store
                .save_chunk(IVec3::ZERO, &Chunk::default(), &metadata)
                .is_ok()
        );
        assert!(store.load_chunk(IVec3::ZERO, &incompatible).is_err());
    }

    #[test]
    fn development_world_paths_include_seed() {
        let a = development_world_path(&WorldMetadata::with_seed(1));
        let b = development_world_path(&WorldMetadata::with_seed(2));

        assert_ne!(a, b);
        assert!(a.ends_with("seed-0000000000000001.sqlite3"));
    }
}
