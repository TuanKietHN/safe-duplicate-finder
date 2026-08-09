//! Cooperative pause, resume, and cancellation at explicit safe boundaries.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use parking_lot::{Condvar, Mutex};

use crate::{DedupeError, Result};

#[derive(Debug, Default)]
struct PauseState {
    paused: bool,
}

/// Cloneable control handle shared by bounded workers.
#[derive(Debug, Clone, Default)]
pub struct ControlToken {
    cancelled: Arc<AtomicBool>,
    pause: Arc<(Mutex<PauseState>, Condvar)>,
}

impl ControlToken {
    /// Create a running token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Request pause; workers stop at their next checkpoint.
    pub fn pause(&self) {
        self.pause.0.lock().paused = true;
    }

    /// Resume all paused workers.
    pub fn resume(&self) {
        let (lock, signal) = &*self.pause;
        lock.lock().paused = false;
        signal.notify_all();
    }

    /// Request cancellation; paused workers are also released to observe it.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.pause.1.notify_all();
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Safe work boundary used between files and streaming chunks.
    pub fn checkpoint(&self) -> Result<()> {
        if self.is_cancelled() {
            return Err(DedupeError::Cancelled);
        }
        let (lock, signal) = &*self.pause;
        let mut state = lock.lock();
        while state.paused && !self.is_cancelled() {
            signal.wait(&mut state);
        }
        if self.is_cancelled() {
            Err(DedupeError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use super::ControlToken;

    #[test]
    fn cancellation_is_observed_at_checkpoint() {
        let token = ControlToken::new();
        token.cancel();
        assert!(token.checkpoint().is_err());
    }

    #[test]
    fn resume_releases_paused_worker() {
        let token = ControlToken::new();
        token.pause();
        let worker_token = token.clone();
        let handle = thread::spawn(move || worker_token.checkpoint());
        thread::sleep(Duration::from_millis(20));
        token.resume();
        assert!(handle.join().is_ok_and(|result| result.is_ok()));
    }
}
