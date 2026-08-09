//! Byte-derived installer progress. The UI never owns or increments these counters.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use thiserror::Error;

const SPEED_WINDOW_MS: u64 = 5_000;

/// Per-runtime display state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    /// Not examined yet.
    Pending,
    /// Inspecting installed/cache evidence.
    Prechecking,
    /// Runtime is already installed.
    InstalledValid,
    /// Completed cache passed size and SHA-256.
    CacheValid,
    /// Network bytes are being written.
    Downloading,
    /// Full completed bytes are being hashed.
    Verifying,
    /// Verified runtime installer is running.
    Installing,
    /// Runtime is installed and re-detected.
    Completed,
    /// User cancellation was observed.
    Cancelled,
    /// Operation failed closed.
    Failed,
}

/// Immutable per-item snapshot for the native UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemSnapshot {
    /// Stable runtime identifier.
    pub id: String,
    /// Current state.
    pub state: ItemState,
    /// Bytes present in cache/part file.
    pub received_bytes: u64,
    /// Exact expected artifact bytes.
    pub size_bytes: u64,
    /// Optional concise state/error text.
    pub message: String,
}

/// Immutable aggregate snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgressSnapshot {
    /// Sum of required artifact sizes for this fixed manifest/session.
    pub required_download_bytes: u64,
    /// Sum of cache/part bytes, clamped by exact item sizes.
    pub received_bytes: u64,
    /// Bytes physically received from the network in this process run.
    pub network_bytes_this_run: u64,
    /// Rolling network throughput.
    pub bytes_per_second: u64,
    /// Remaining byte ETA; absent until speed is non-zero.
    pub eta_seconds: Option<u64>,
    /// Overall byte-weighted progress from 0 through 10,000.
    pub overall_basis_points: u16,
    /// Deterministically ordered item snapshots.
    pub items: Vec<ItemSnapshot>,
}

#[derive(Debug)]
struct ItemProgress {
    size_bytes: u64,
    required: bool,
    received_bytes: u64,
    state: ItemState,
    message: String,
}

#[derive(Debug)]
struct Inner {
    items: BTreeMap<String, ItemProgress>,
    total: u64,
    network_total: u64,
    samples: VecDeque<(u64, u64)>,
}

/// Thread-safe source of truth for real byte progress.
#[derive(Debug)]
pub struct ProgressBook {
    inner: Mutex<Inner>,
    created: Instant,
}

/// Invalid counter update.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProgressError {
    /// Item ID was not present in this manifest.
    #[error("không có runtime: {0}")]
    UnknownItem(String),
    /// A count exceeded its exact item/aggregate size.
    #[error("bộ đếm byte vượt phạm vi")]
    CounterOverflow,
    /// Duplicate item identifier.
    #[error("runtime bị trùng ID: {0}")]
    DuplicateItem(String),
}

impl ProgressBook {
    /// Build a progress book from `(id, exact_size)` pairs.
    pub fn new<I, S>(items: I) -> Result<Self, ProgressError>
    where
        I: IntoIterator<Item = (S, u64)>,
        S: Into<String>,
    {
        Self::new_with_required(
            items
                .into_iter()
                .map(|(id, size_bytes)| (id, size_bytes, true)),
        )
    }

    /// Build a book while excluding already-installed items from aggregate download totals.
    pub fn new_with_required<I, S>(items: I) -> Result<Self, ProgressError>
    where
        I: IntoIterator<Item = (S, u64, bool)>,
        S: Into<String>,
    {
        let mut map = BTreeMap::new();
        let mut total = 0_u64;
        for (id, size_bytes, required) in items {
            let id = id.into();
            if required {
                total = total
                    .checked_add(size_bytes)
                    .ok_or(ProgressError::CounterOverflow)?;
            }
            if map
                .insert(
                    id.clone(),
                    ItemProgress {
                        size_bytes,
                        required,
                        received_bytes: 0,
                        state: ItemState::Pending,
                        message: String::new(),
                    },
                )
                .is_some()
            {
                return Err(ProgressError::DuplicateItem(id));
            }
        }
        let samples = VecDeque::from([(0, 0)]);
        Ok(Self {
            inner: Mutex::new(Inner {
                items: map,
                total,
                network_total: 0,
                samples,
            }),
            created: Instant::now(),
        })
    }

    /// Set bytes already present before a new network read.
    pub fn set_existing_bytes(&self, id: &str, bytes: u64) -> Result<(), ProgressError> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let item = inner
            .items
            .get_mut(id)
            .ok_or_else(|| ProgressError::UnknownItem(id.to_owned()))?;
        if bytes > item.size_bytes {
            return Err(ProgressError::CounterOverflow);
        }
        item.received_bytes = bytes;
        Ok(())
    }

    /// Include/exclude an item from aggregate download totals after installed-runtime preflight.
    pub fn set_required(&self, id: &str, required: bool) -> Result<(), ProgressError> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (was_required, size_bytes) = {
            let item = inner
                .items
                .get_mut(id)
                .ok_or_else(|| ProgressError::UnknownItem(id.to_owned()))?;
            let previous = item.required;
            item.required = required;
            (previous, item.size_bytes)
        };
        if was_required == required {
            return Ok(());
        }
        inner.total = if required {
            inner
                .total
                .checked_add(size_bytes)
                .ok_or(ProgressError::CounterOverflow)?
        } else {
            inner.total.saturating_sub(size_bytes)
        };
        Ok(())
    }

    /// Add bytes using this book's shared monotonic process origin.
    pub fn record_network_bytes_now(&self, id: &str, bytes: u64) -> Result<(), ProgressError> {
        self.record_network_bytes(id, bytes, self.elapsed_ms().max(1))
    }

    /// Milliseconds since this progress book was created.
    #[must_use]
    pub fn elapsed_ms(&self) -> u64 {
        u64::try_from(self.created.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Add bytes after they were successfully written to the partial file.
    pub fn record_network_bytes(
        &self,
        id: &str,
        bytes: u64,
        elapsed_ms: u64,
    ) -> Result<(), ProgressError> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let item = inner
            .items
            .get_mut(id)
            .ok_or_else(|| ProgressError::UnknownItem(id.to_owned()))?;
        item.received_bytes = item
            .received_bytes
            .checked_add(bytes)
            .filter(|value| *value <= item.size_bytes)
            .ok_or(ProgressError::CounterOverflow)?;
        inner.network_total = inner
            .network_total
            .checked_add(bytes)
            .ok_or(ProgressError::CounterOverflow)?;
        let network_total = inner.network_total;
        inner.samples.push_back((elapsed_ms, network_total));
        while inner.samples.len() > 2
            && inner
                .samples
                .get(1)
                .is_some_and(|sample| elapsed_ms.saturating_sub(sample.0) > SPEED_WINDOW_MS)
        {
            inner.samples.pop_front();
        }
        Ok(())
    }

    /// Replace one item's display state.
    pub fn set_state(&self, id: &str, state: ItemState) -> Result<(), ProgressError> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let item = inner
            .items
            .get_mut(id)
            .ok_or_else(|| ProgressError::UnknownItem(id.to_owned()))?;
        item.state = state;
        Ok(())
    }

    /// Replace one item's concise display message.
    pub fn set_message(&self, id: &str, message: impl Into<String>) -> Result<(), ProgressError> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let item = inner
            .items
            .get_mut(id)
            .ok_or_else(|| ProgressError::UnknownItem(id.to_owned()))?;
        item.message = message.into();
        Ok(())
    }

    /// Capture a progress snapshot at monotonic milliseconds since process start.
    #[must_use]
    pub fn snapshot(&self, elapsed_ms: u64) -> ProgressSnapshot {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let received_bytes = inner
            .items
            .values()
            .filter(|item| item.required)
            .fold(0_u64, |sum, item| sum.saturating_add(item.received_bytes));
        let (bytes_per_second, eta_seconds) = speed_and_eta(&inner, elapsed_ms, received_bytes);
        let overall = if inner.total == 0 {
            10_000
        } else {
            let basis_points = u128::from(received_bytes) * 10_000 / u128::from(inner.total);
            u16::try_from(basis_points.min(10_000)).unwrap_or(10_000)
        };
        ProgressSnapshot {
            required_download_bytes: inner.total,
            received_bytes,
            network_bytes_this_run: inner.network_total,
            bytes_per_second,
            eta_seconds,
            overall_basis_points: overall,
            items: inner
                .items
                .iter()
                .map(|(id, item)| ItemSnapshot {
                    id: id.clone(),
                    state: item.state,
                    received_bytes: item.received_bytes,
                    size_bytes: item.size_bytes,
                    message: item.message.clone(),
                })
                .collect(),
        }
    }
}

fn speed_and_eta(inner: &Inner, now_ms: u64, received_bytes: u64) -> (u64, Option<u64>) {
    let Some(&(first_ms, first_bytes)) = inner.samples.front() else {
        return (0, None);
    };
    let elapsed = now_ms.saturating_sub(first_ms);
    let new_bytes = inner.network_total.saturating_sub(first_bytes);
    let speed = if elapsed == 0 {
        0
    } else {
        u64::try_from(u128::from(new_bytes) * 1_000 / u128::from(elapsed)).unwrap_or(u64::MAX)
    };
    let remaining = inner.total.saturating_sub(received_bytes);
    let eta = if speed == 0 {
        None
    } else {
        Some(remaining.div_ceil(speed))
    };
    (speed, eta)
}
