//! Guarded database open and versioned initial migration.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use parking_lot::{Mutex, MutexGuard};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

/// Result of a safe database maintenance pass that preserves every history row.
#[derive(Debug, Clone, Serialize)]
pub struct DatabaseMaintenance {
    /// Database, WAL and shared-memory bytes before maintenance.
    pub before_bytes: u64,
    /// Database, WAL and shared-memory bytes after maintenance.
    pub after_bytes: u64,
    /// Bytes returned to the filesystem.
    pub reclaimed_bytes: u64,
}

/// `SQLite` adapter error.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// `SQLite` returned an error.
    #[error("SQLite thất bại: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Database path is unsafe.
    #[error("Đường dẫn cơ sở dữ liệu không an toàn: {0}")]
    UnsafePath(String),
    /// Filesystem operation failed.
    #[error("I/O cơ sở dữ liệu thất bại khi {operation} tại {path}: {source}")]
    Io {
        /// Operation name.
        operation: &'static str,
        /// Affected path.
        path: PathBuf,
        /// Operating-system error.
        source: std::io::Error,
    },
}

/// Thread-safe database owner. Writes are serialized by the connection mutex.
#[derive(Clone)]
pub struct Database {
    connection: Arc<Mutex<Connection>>,
    path: Arc<PathBuf>,
}

impl Database {
    /// Open/create a database outside scan and quarantine roots.
    pub fn open(path: &Path, forbidden_roots: &[std::path::PathBuf]) -> Result<Self, StoreError> {
        let absolute = std::path::absolute(path).map_err(|error| {
            StoreError::UnsafePath(format!("Không thể xác định {}: {error}", path.display()))
        })?;
        if forbidden_roots
            .iter()
            .any(|root| absolute.starts_with(root))
        {
            return Err(StoreError::UnsafePath(format!(
                "Cơ sở dữ liệu {} nằm trong thư mục nguồn/cách ly",
                absolute.display()
            )));
        }
        if let Some(parent) = absolute.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                StoreError::UnsafePath(format!("Không thể tạo {}: {error}", parent.display()))
            })?;
        }
        let connection = Connection::open(&absolute)?;
        connection.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000;",
        )?;
        let migrated = connection
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_migrations'",
                [],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !migrated {
            connection.execute_batch(include_str!("../migrations/0001_initial.sql"))?;
        }
        let version: i64 = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        if version < 2 {
            connection.execute_batch(include_str!("../migrations/0002_scan_control.sql"))?;
        }
        let version: i64 = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        if version < 3 {
            connection.execute_batch(include_str!("../migrations/0003_permanent_delete.sql"))?;
        }
        let version: i64 = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        if version < 4 {
            connection.execute_batch(include_str!("../migrations/0004_immediate_delete.sql"))?;
        }
        let version: i64 = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        if version < 5 {
            connection.execute_batch(include_str!("../migrations/0005_scan_block_reason.sql"))?;
        }
        let version: i64 = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        if version < 6 {
            connection.execute_batch(include_str!("../migrations/0006_history_indexes.sql"))?;
        }
        let version: i64 = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        if version != 6 {
            return Err(StoreError::UnsafePath(format!(
                "Không hỗ trợ phiên bản schema cơ sở dữ liệu {version}; cần phiên bản 6"
            )));
        }
        let integrity: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(StoreError::UnsafePath(format!(
                "quick_check cơ sở dữ liệu thất bại: {integrity}"
            )));
        }
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            path: Arc::new(absolute),
        })
    }

    /// Serialize a short database operation.
    pub fn connection(&self) -> MutexGuard<'_, Connection> {
        self.connection.lock()
    }

    /// Absolute database path, used to open independent read snapshots and to reject scan roots.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.path.as_path()
    }

    /// Open a separate read-only connection for streaming candidate groups while the writer remains
    /// available for short commits.
    pub fn read_connection(&self) -> Result<Connection, StoreError> {
        let connection = Connection::open_with_flags(
            self.path(),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000;")?;
        Ok(connection)
    }

    /// Create a consistent, no-overwrite `SQLite` backup after checkpointing the WAL.
    pub fn backup_to(&self, destination: &Path) -> Result<PathBuf, StoreError> {
        let destination = std::path::absolute(destination).map_err(|source| StoreError::Io {
            operation: "xác định đích sao lưu",
            path: destination.to_path_buf(),
            source,
        })?;
        if destination.as_path() == self.path() || destination.exists() {
            return Err(StoreError::UnsafePath(format!(
                "Đích sao lưu đã tồn tại hoặc chính là cơ sở dữ liệu đang dùng: {}",
                destination.display()
            )));
        }
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                operation: "tạo thư mục sao lưu",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let connection = self.connection();
        connection.execute_batch("PRAGMA wal_checkpoint(FULL);")?;
        connection.execute("VACUUM INTO ?1", [destination.to_string_lossy().as_ref()])?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&destination)
            .map_err(|source| StoreError::Io {
                operation: "mở bản sao lưu đã hoàn tất để đồng bộ",
                path: destination.clone(),
                source,
            })?;
        file.sync_all().map_err(|source| StoreError::Io {
            operation: "đồng bộ bản sao lưu đã hoàn tất",
            path: destination.clone(),
            source,
        })?;
        Ok(destination)
    }

    /// Checkpoint, optimize and vacuum the database without deleting durable evidence rows.
    pub fn compact(&self) -> Result<DatabaseMaintenance, StoreError> {
        let before_bytes = database_files_size(self.path())?;
        let connection = self.connection();
        connection.execute_batch(
            "PRAGMA wal_checkpoint(TRUNCATE);
             PRAGMA optimize;
             VACUUM;
             PRAGMA wal_checkpoint(TRUNCATE);",
        )?;
        let integrity: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if integrity != "ok" {
            return Err(StoreError::UnsafePath(format!(
                "quick_check sau bảo trì thất bại: {integrity}"
            )));
        }
        drop(connection);
        let after_bytes = database_files_size(self.path())?;
        Ok(DatabaseMaintenance {
            before_bytes,
            after_bytes,
            reclaimed_bytes: before_bytes.saturating_sub(after_bytes),
        })
    }
}

fn database_files_size(path: &Path) -> Result<u64, StoreError> {
    let mut total = file_size(path)?;
    for suffix in ["-wal", "-shm"] {
        total = total.saturating_add(file_size(&PathBuf::from(format!(
            "{}{}",
            path.display(),
            suffix
        )))?);
    }
    Ok(total)
}

fn file_size(path: &Path) -> Result<u64, StoreError> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(source) => Err(StoreError::Io {
            operation: "đọc kích thước dữ liệu cục bộ",
            path: path.to_path_buf(),
            source,
        }),
    }
}

impl std::fmt::Debug for Database {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Database").finish_non_exhaustive()
    }
}
