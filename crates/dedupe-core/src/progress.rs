//! Monotonic progress counters safe to snapshot from UI and CLI adapters.

use std::{
    ops::Deref,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// Immutable progress snapshot.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ProgressSnapshot {
    /// Discovered eligible files.
    pub discovered_files: u64,
    /// Completed file stages.
    pub processed_files: u64,
    /// Bytes read across sampled/full stages.
    pub bytes_read: u64,
    /// Isolated file errors.
    pub errors: u64,
    /// Skipped files.
    pub skipped: u64,
    /// Files that changed during evidence collection.
    pub unstable: u64,
}

/// Lock-free monotonic counter set.
#[derive(Debug, Default)]
pub struct ProgressCounters {
    discovered_files: AtomicU64,
    processed_files: AtomicU64,
    bytes_read: AtomicU64,
    errors: AtomicU64,
    skipped: AtomicU64,
    unstable: AtomicU64,
}

#[derive(Debug)]
struct ProgressSubscriber {
    sender: Sender<ProgressSnapshot>,
    drain: Receiver<ProgressSnapshot>,
    alive: Arc<AtomicBool>,
}

/// Bounded newest-value receiver. Dropping it unregisters the subscriber on the next publish.
#[derive(Debug)]
pub struct ProgressSubscription {
    receiver: Receiver<ProgressSnapshot>,
    alive: Arc<AtomicBool>,
}

impl Deref for ProgressSubscription {
    type Target = Receiver<ProgressSnapshot>;

    fn deref(&self) -> &Self::Target {
        &self.receiver
    }
}

impl Drop for ProgressSubscription {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

/// Non-blocking bounded fan-out for UI/CLI progress observers.
///
/// Each subscriber keeps only its newest unconsumed snapshot, so slow UI consumers cannot apply
/// backpressure to scan or hashing workers.
#[derive(Debug, Default)]
pub struct ProgressFanout {
    subscribers: Mutex<Vec<ProgressSubscriber>>,
}

impl ProgressFanout {
    /// Subscribe to newest-value notifications with a one-snapshot bound.
    #[must_use]
    pub fn subscribe(&self) -> ProgressSubscription {
        let (sender, receiver) = bounded(1);
        let alive = Arc::new(AtomicBool::new(true));
        self.subscribers.lock().push(ProgressSubscriber {
            sender,
            drain: receiver.clone(),
            alive: Arc::clone(&alive),
        });
        ProgressSubscription { receiver, alive }
    }

    /// Publish without blocking. A slow subscriber's stale snapshot is replaced by this one.
    pub fn publish(&self, snapshot: ProgressSnapshot) {
        self.subscribers.lock().retain(|subscriber| {
            if !subscriber.alive.load(Ordering::Acquire) {
                return false;
            }
            match subscriber.sender.try_send(snapshot) {
                Ok(()) => true,
                Err(TrySendError::Full(latest)) => {
                    let _ = subscriber.drain.try_recv();
                    !matches!(
                        subscriber.sender.try_send(latest),
                        Err(TrySendError::Disconnected(_))
                    )
                }
                Err(TrySendError::Disconnected(_)) => false,
            }
        });
    }
}

impl ProgressCounters {
    /// Record one discovered file.
    pub fn discovered(&self) {
        self.discovered_files.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one processed file stage.
    pub fn processed(&self) {
        self.processed_files.fetch_add(1, Ordering::Relaxed);
    }

    /// Add bytes with saturation rather than wrapping.
    pub fn add_bytes(&self, bytes: u64) {
        let _ = self
            .bytes_read
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_add(bytes))
            });
    }

    /// Record one isolated error.
    pub fn error(&self) {
        self.errors.fetch_add(1, Ordering::Relaxed);
    }

    /// Add a bounded batch of isolated errors.
    pub fn add_errors(&self, count: u64) {
        let _ = self
            .errors
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_add(count))
            });
    }

    /// Record one skipped file.
    pub fn skipped(&self) {
        self.skipped.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one unstable file.
    pub fn unstable(&self) {
        self.unstable.fetch_add(1, Ordering::Relaxed);
    }

    /// Add a bounded batch of files observed changing during evidence collection.
    pub fn add_unstable(&self, count: u64) {
        let _ = self
            .unstable
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_add(count))
            });
    }

    /// Capture a consistent-enough monotonic display snapshot.
    #[must_use]
    pub fn snapshot(&self) -> ProgressSnapshot {
        ProgressSnapshot {
            discovered_files: self.discovered_files.load(Ordering::Relaxed),
            processed_files: self.processed_files.load(Ordering::Relaxed),
            bytes_read: self.bytes_read.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
            unstable: self.unstable.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod fanout_tests {
    use super::{ProgressFanout, ProgressSnapshot};

    #[test]
    fn slow_subscriber_keeps_only_newest_monotonic_snapshot()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let fanout = ProgressFanout::default();
        let receiver = fanout.subscribe();
        fanout.publish(ProgressSnapshot {
            discovered_files: 1,
            ..ProgressSnapshot::default()
        });
        fanout.publish(ProgressSnapshot {
            discovered_files: 2,
            ..ProgressSnapshot::default()
        });
        fanout.publish(ProgressSnapshot {
            discovered_files: 3,
            ..ProgressSnapshot::default()
        });
        assert_eq!(receiver.recv()?.discovered_files, 3);
        assert!(receiver.try_recv().is_err());
        Ok(())
    }

    #[test]
    fn disconnected_subscriber_is_pruned_without_blocking_publish() {
        let fanout = ProgressFanout::default();
        drop(fanout.subscribe());
        fanout.publish(ProgressSnapshot::default());
        assert!(fanout.subscribers.lock().is_empty());
    }
}
