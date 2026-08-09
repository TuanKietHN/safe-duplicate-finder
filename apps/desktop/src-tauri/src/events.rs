//! Bounded newest-only desktop events; safety state remains durable in `SQLite`.

use std::{collections::VecDeque, sync::Arc};

use chrono::Utc;
use dedupe_core::progress::ProgressSnapshot;
use parking_lot::Mutex;
use serde::Serialize;
use uuid::Uuid;

const EVENT_CAPACITY: usize = 32;

/// Versioned scan event returned to the desktop without an unbounded payload stream.
#[derive(Debug, Clone, Serialize)]
pub struct DesktopEvent {
    /// Event contract version.
    pub schema_version: u16,
    /// Process-local monotonic sequence; a gap tells the UI to refresh durable state.
    pub sequence: u64,
    /// Owning project.
    pub project_id: Uuid,
    /// Owning scan session.
    pub session_id: Uuid,
    /// RFC 3339 emission time.
    pub emitted_at: String,
    /// Stable event kind.
    pub kind: String,
    /// Newest monotonic counters.
    pub progress: ProgressSnapshot,
}

#[derive(Debug, Default)]
struct EventState {
    next_sequence: u64,
    pending: VecDeque<DesktopEvent>,
    dropped: u64,
}

/// Cloneable fixed-capacity event hub keeping at most one pending event per session.
#[derive(Debug, Clone, Default)]
pub struct EventHub {
    state: Arc<Mutex<EventState>>,
}

impl EventHub {
    /// Publish newest progress without blocking a scan worker.
    pub fn publish_scan(
        &self,
        project_id: Uuid,
        session_id: Uuid,
        kind: impl Into<String>,
        progress: ProgressSnapshot,
    ) {
        let mut state = self.state.lock();
        state.next_sequence = state.next_sequence.saturating_add(1);
        let sequence = state.next_sequence;
        if let Some(index) = state
            .pending
            .iter()
            .position(|event| event.session_id == session_id)
        {
            state.pending.remove(index);
            state.dropped = state.dropped.saturating_add(1);
        }
        if state.pending.len() == EVENT_CAPACITY {
            state.pending.pop_front();
            state.dropped = state.dropped.saturating_add(1);
        }
        state.pending.push_back(DesktopEvent {
            schema_version: 1,
            sequence,
            project_id,
            session_id,
            emitted_at: Utc::now().to_rfc3339(),
            kind: kind.into(),
            progress,
        });
    }

    /// Take the newest pending event for one session, if any.
    #[must_use]
    pub fn take(&self, session_id: Uuid) -> Option<DesktopEvent> {
        let mut state = self.state.lock();
        let index = state
            .pending
            .iter()
            .position(|event| event.session_id == session_id)?;
        state.pending.remove(index)
    }

    #[cfg(test)]
    fn metrics(&self) -> (usize, u64) {
        let state = self.state.lock();
        (state.pending.len(), state.dropped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_event_replaces_stale_event_for_same_session() {
        let hub = EventHub::default();
        let project = Uuid::new_v4();
        let session = Uuid::new_v4();
        for processed_files in 0..1_000 {
            hub.publish_scan(
                project,
                session,
                "scan://snapshot",
                ProgressSnapshot {
                    processed_files,
                    ..ProgressSnapshot::default()
                },
            );
        }
        assert_eq!(hub.metrics(), (1, 999));
        let event = hub.take(session);
        assert!(event.is_some_and(|event| event.progress.processed_files == 999));
        assert_eq!(hub.metrics().0, 0);
    }

    #[test]
    fn global_pending_events_never_exceed_fixed_capacity() {
        let hub = EventHub::default();
        let project = Uuid::new_v4();
        for _ in 0..100 {
            hub.publish_scan(
                project,
                Uuid::new_v4(),
                "scan://state",
                ProgressSnapshot::default(),
            );
        }
        let (pending, dropped) = hub.metrics();
        assert_eq!(pending, EVENT_CAPACITY);
        assert_eq!(dropped, 100 - EVENT_CAPACITY as u64);
    }
}
