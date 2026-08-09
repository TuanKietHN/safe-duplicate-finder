//! Reusable deterministic fault boundaries for crash/durability safety tests.

use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};

use dedupe_core::{
    DedupeError, Result,
    model::{FileMetadataSnapshot, FileTransaction, TransactionState},
    ports::{MetadataProvider, SafeMover, TransactionJournal},
};

/// Named safety boundary that can fail deterministically on a selected hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FaultPoint {
    /// Persisting initial mutation intent.
    JournalCreate,
    /// Persisting successful preflight evidence.
    PreflightTransition,
    /// Persisting that filesystem mutation is about to begin.
    MovingTransition,
    /// Persisting that the rename returned successfully.
    MovedTransition,
    /// Persisting destination verification success.
    VerifiedTransition,
    /// Reading a metadata snapshot.
    MetadataSnapshot,
    /// Entering the platform filesystem move.
    FilesystemMove,
}

#[derive(Debug, Default)]
struct FaultState {
    fail_on_hit: BTreeMap<FaultPoint, u64>,
    hits: BTreeMap<FaultPoint, u64>,
}

/// Thread-safe deterministic fault schedule shared by adapter wrappers.
#[derive(Debug, Clone, Default)]
pub struct FaultInjector {
    state: Arc<Mutex<FaultState>>,
}

impl FaultInjector {
    /// Configure one boundary to fail on its one-based `hit` number.
    pub fn fail_on(&self, point: FaultPoint, hit: u64) -> Result<()> {
        if hit == 0 {
            return Err(DedupeError::InvalidInput(
                "fault hit number must be one-based".into(),
            ));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| DedupeError::State("fault injector lock poisoned".into()))?;
        state.fail_on_hit.insert(point, hit);
        Ok(())
    }

    /// Count one visit and fail exactly when the configured hit is reached.
    pub fn checkpoint(&self, point: FaultPoint) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| DedupeError::State("fault injector lock poisoned".into()))?;
        let hit = state.hits.entry(point).or_default();
        *hit = hit.saturating_add(1);
        let current_hit = *hit;
        if state.fail_on_hit.get(&point) == Some(&current_hit) {
            return Err(DedupeError::Durability(format!(
                "injected fault at {point:?} hit {current_hit}"
            )));
        }
        Ok(())
    }
}

/// Journal wrapper that injects failures at durable transaction boundaries.
pub struct FaultingJournal<J> {
    /// Wrapped real or in-memory journal.
    pub inner: J,
    /// Shared deterministic schedule.
    pub faults: FaultInjector,
}

impl<J: TransactionJournal> TransactionJournal for FaultingJournal<J> {
    fn create(&self, transaction: &FileTransaction) -> Result<()> {
        self.faults.checkpoint(FaultPoint::JournalCreate)?;
        self.inner.create(transaction)
    }

    fn transition(
        &self,
        transaction: &FileTransaction,
        next: TransactionState,
        verification: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        let point = match next {
            TransactionState::PreflightValidated => Some(FaultPoint::PreflightTransition),
            TransactionState::Moving => Some(FaultPoint::MovingTransition),
            TransactionState::MovedUnverified => Some(FaultPoint::MovedTransition),
            TransactionState::Verified => Some(FaultPoint::VerifiedTransition),
            _ => None,
        };
        if let Some(point) = point {
            self.faults.checkpoint(point)?;
        }
        self.inner
            .transition(transaction, next, verification, error)
    }
}

/// Metadata wrapper for locked/disconnected/change-during-read fixtures.
pub struct FaultingMetadata<P> {
    /// Wrapped provider.
    pub inner: P,
    /// Shared deterministic schedule.
    pub faults: FaultInjector,
}

impl<P: MetadataProvider> MetadataProvider for FaultingMetadata<P> {
    fn snapshot(&self, path: &Path) -> Result<FileMetadataSnapshot> {
        self.faults.checkpoint(FaultPoint::MetadataSnapshot)?;
        self.inner.snapshot(path)
    }
}

/// Move wrapper for full-volume/disconnect/permission simulations before mutation.
pub struct FaultingMover<M> {
    /// Wrapped platform mover.
    pub inner: M,
    /// Shared deterministic schedule.
    pub faults: FaultInjector,
}

impl<M: SafeMover> SafeMover for FaultingMover<M> {
    fn move_no_replace(
        &self,
        source: &Path,
        destination: &Path,
        expected: &FileMetadataSnapshot,
    ) -> Result<()> {
        self.faults.checkpoint(FaultPoint::FilesystemMove)?;
        self.inner.move_no_replace(source, destination, expected)
    }
}
