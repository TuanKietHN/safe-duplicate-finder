//! Shared local engine state and cooperative scan controls.

use std::{collections::HashMap, sync::Arc};

use dedupe_core::progress::ProgressCounters;
use dedupe_store::{Database, ScanRepository};
use parking_lot::Mutex;
use uuid::Uuid;

use crate::events::EventHub;

/// One active background scan.
#[derive(Clone)]
pub struct ScanJob {
    /// Live monotonic counters.
    pub progress: Arc<ProgressCounters>,
}

/// Tauri-managed application services.
#[derive(Clone)]
pub struct EngineState {
    /// Durable local state.
    pub database: Database,
    /// Bounded set of active jobs keyed by session.
    pub jobs: Arc<Mutex<HashMap<Uuid, ScanJob>>>,
    /// Fixed-capacity newest-only events; durable state remains authoritative.
    pub events: EventHub,
    /// Keeps both local log writers alive for the application lifetime.
    _logs: Arc<dedupe_core::logging::LogGuards>,
}

impl EngineState {
    /// Construct a state owner around an initialized database.
    pub fn new(
        database: Database,
        logs: dedupe_core::logging::LogGuards,
    ) -> dedupe_core::Result<Self> {
        let interrupted = ScanRepository::new(database.clone()).mark_incomplete_interrupted()?;
        if interrupted != 0 {
            tracing::warn!(
                interrupted,
                "Đã đánh dấu các phiên quét mất tiến trình để tiếp tục rõ ràng"
            );
        }
        Ok(Self {
            database,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            events: EventHub::default(),
            _logs: Arc::new(logs),
        })
    }
}
