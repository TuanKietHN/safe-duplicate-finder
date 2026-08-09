//! Bounded runtime download scheduler.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use thiserror::Error;

/// Scheduler contract failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SchedulerError {
    /// Releases intentionally support at most two independent downloads.
    #[error("số worker phải nằm trong 1..=2")]
    InvalidWorkerLimit,
    /// A worker terminated before publishing every result.
    #[error("worker kết thúc trước khi hoàn tất")]
    WorkerTerminated,
}

/// Run independent work with a strict one- or two-worker cap while preserving input order.
pub fn run_bounded<T, R, F>(
    items: Vec<T>,
    worker_limit: usize,
    work: F,
) -> Result<Vec<R>, SchedulerError>
where
    T: Send,
    R: Send,
    F: Fn(T) -> R + Sync,
{
    if !(1..=2).contains(&worker_limit) {
        return Err(SchedulerError::InvalidWorkerLimit);
    }
    if items.is_empty() {
        return Ok(Vec::new());
    }

    let count = items.len();
    let slots = Arc::new(
        items
            .into_iter()
            .map(|item| Mutex::new(Some(item)))
            .collect::<Vec<_>>(),
    );
    let next = Arc::new(AtomicUsize::new(0));
    let (sender, receiver) = mpsc::channel();

    std::thread::scope(|scope| {
        for _ in 0..worker_limit.min(count) {
            let slots = Arc::clone(&slots);
            let next = Arc::clone(&next);
            let sender = sender.clone();
            let work = &work;
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(slot) = slots.get(index) else {
                        break;
                    };
                    let item = slot
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .take();
                    let Some(item) = item else {
                        continue;
                    };
                    if sender.send((index, work(item))).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);
    });

    let mut results = (0..count).map(|_| None).collect::<Vec<Option<R>>>();
    for (index, result) in receiver {
        if let Some(slot) = results.get_mut(index) {
            *slot = Some(result);
        }
    }
    results
        .into_iter()
        .map(|result| result.ok_or(SchedulerError::WorkerTerminated))
        .collect()
}
