//! Hexagonal ports implemented by storage and platform crates.

use std::path::Path;

use crate::{
    Result,
    model::{FileMetadataSnapshot, FileTransaction, TransactionState},
    permanent_delete::{PermanentDeleteBatch, PermanentDeleteItem, PermanentDeleteItemState},
};

/// Platform-specific metadata and physical identity provider.
pub trait MetadataProvider: Send + Sync {
    /// Capture a complete metadata snapshot without following a link target unexpectedly.
    fn snapshot(&self, path: &Path) -> Result<FileMetadataSnapshot>;
}

/// Filesystem mutation port with no-overwrite and same-volume guarantees.
pub trait SafeMover: Send + Sync {
    /// Move the already-proven physical file on the same volume and fail if destination exists.
    fn move_no_replace(
        &self,
        source: &Path,
        destination: &Path,
        expected: &FileMetadataSnapshot,
    ) -> Result<()>;
}

/// Handle-bound irreversible deletion. Implementations must revalidate the opened physical file.
pub trait SafeDeleter: Send + Sync {
    /// Delete exactly the file represented by `expected`, never a subsequently replaced path.
    fn delete_exact(&self, expected: &FileMetadataSnapshot) -> Result<()>;
}

/// Durable append-only permanent-deletion journal and current-state projection.
pub trait PermanentDeleteJournal: Send + Sync {
    /// Persist a prepared batch and all selected quarantine entries.
    fn create_batch(&self, batch: &PermanentDeleteBatch) -> Result<()>;
    /// Load one prepared or interrupted batch with its immutable evidence.
    fn load_batch(&self, batch_id: uuid::Uuid) -> Result<PermanentDeleteBatch>;
    /// Append and commit a batch transition before returning.
    fn transition_batch(
        &self,
        batch: &PermanentDeleteBatch,
        next: crate::permanent_delete::PermanentDeleteBatchState,
        error: Option<&str>,
    ) -> Result<()>;
    /// Append and commit an item transition before returning.
    fn transition_item(
        &self,
        batch: &PermanentDeleteBatch,
        item: &PermanentDeleteItem,
        next: PermanentDeleteItemState,
        error: Option<&str>,
    ) -> Result<()>;
}

/// Durable append-only transaction journal. Every successful call is stable before it returns.
pub trait TransactionJournal: Send + Sync {
    /// Persist `planned` intent before any mutation preflight.
    fn create(&self, transaction: &FileTransaction) -> Result<()>;
    /// Append a transition and update the current-state projection atomically.
    fn transition(
        &self,
        transaction: &FileTransaction,
        next: TransactionState,
        verification: Option<&str>,
        error: Option<&str>,
    ) -> Result<()>;
}

/// Durable sink used by the streaming scanner; implementations batch internally.
pub trait ScanSink {
    /// Persist one observation. Returning an error stops the scan before any mutation exists.
    fn record(&mut self, snapshot: &FileMetadataSnapshot) -> Result<()>;
    /// Persist or display one isolated path error while allowing unrelated files to continue.
    fn record_error(&mut self, path: &Path, error: &crate::DedupeError) -> Result<()>;
    /// Flush the current batch durably.
    fn flush(&mut self) -> Result<()>;
}
