//! Conservative defaults and a bounded, volume-aware worker pool.

use std::{collections::HashMap, sync::Arc, time::Duration};

use crossbeam_channel::{Receiver, bounded};
use parking_lot::{Condvar, Mutex};

use crate::{DedupeError, Result, control::ControlToken, model::WorkerConfig};

/// Coarse storage profile. Unknown and network storage use the safest queue depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageProfile {
    /// Rotational disk.
    Hdd,
    /// SATA solid-state disk.
    SataSsd,
    /// `NVMe` solid-state disk.
    Nvme,
    /// Network share.
    Network,
    /// Unidentified storage.
    Unknown,
}

/// One owned job assigned to a physical volume key.
#[derive(Debug)]
pub struct VolumeJob<T> {
    /// Stable volume identifier. Unknown identities should share one conservative key.
    pub volume_id: String,
    /// File- or stage-specific work item.
    pub value: T,
}

/// Conservative default based on storage, capped by logical CPU count.
#[must_use]
pub fn recommended(profile: StorageProfile, logical_cpus: usize) -> WorkerConfig {
    let full_hash_workers_per_volume = match profile {
        StorageProfile::Hdd | StorageProfile::Network | StorageProfile::Unknown => 1,
        StorageProfile::SataSsd => 2,
        StorageProfile::Nvme => 4.min(logical_cpus.saturating_div(3).max(1)),
    };
    WorkerConfig {
        metadata_workers: logical_cpus.saturating_div(2).clamp(1, 8),
        full_hash_workers_per_volume,
        queue_capacity: 1024,
    }
}

/// Run owned jobs with a fixed global pool, bounded queues, and a separate full-read limit per volume.
///
/// The callback must keep long operations cooperative by checking `control`; the scheduler checks it
/// before queueing, before waiting for a volume permit, and immediately before starting each job.
/// Results preserve input order even though execution is parallel. No thread is created per file.
pub fn run_volume_jobs<T, R, F>(
    jobs: Vec<VolumeJob<T>>,
    workers: WorkerConfig,
    control: &ControlToken,
    work: F,
) -> Result<Vec<R>>
where
    T: Send,
    R: Send,
    F: Fn(T, &ControlToken) -> Result<R> + Sync,
{
    if jobs.is_empty() {
        return Ok(Vec::new());
    }
    let job_count = jobs.len();
    let worker_count = workers.metadata_workers.clamp(1, 64).min(job_count);
    let per_volume = workers.full_hash_workers_per_volume.clamp(1, 64);
    let queue_capacity = workers.queue_capacity.clamp(1, 65_536);
    let limiters = Arc::new(volume_limiters(&jobs, per_volume));
    let (job_sender, job_receiver) = bounded::<(usize, VolumeJob<T>)>(queue_capacity);
    let (result_sender, result_receiver) = bounded::<(usize, Result<R>)>(queue_capacity);

    std::thread::scope(|scope| -> Result<Vec<R>> {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let jobs = job_receiver.clone();
            let results = result_sender.clone();
            let worker_control = control.clone();
            let worker_limiters = Arc::clone(&limiters);
            let work = &work;
            handles.push(scope.spawn(move || {
                run_worker(&jobs, &results, &worker_limiters, &worker_control, work);
            }));
        }
        drop(job_receiver);

        let producer_results = result_sender.clone();
        let producer_control = control.clone();
        let producer = scope.spawn(move || {
            for (index, job) in jobs.into_iter().enumerate() {
                if let Err(error) = producer_control.checkpoint() {
                    let _ = producer_results.send((index, Err(error)));
                    return;
                }
                if job_sender.send((index, job)).is_err() {
                    return;
                }
            }
        });
        drop(result_sender);

        let mut ordered = (0..job_count).map(|_| None).collect::<Vec<_>>();
        let mut first_error = None;
        for (index, result) in result_receiver {
            match result {
                Ok(value) if first_error.is_none() => ordered[index] = Some(value),
                Err(error) if first_error.is_none() => {
                    first_error = Some(error);
                    control.cancel();
                }
                Ok(_) | Err(_) => {}
            }
        }
        producer
            .join()
            .map_err(|_| DedupeError::State("Bộ tạo lịch theo ổ đĩa gặp panic".into()))?;
        for handle in handles {
            handle
                .join()
                .map_err(|_| DedupeError::State("Luồng lập lịch theo ổ đĩa gặp panic".into()))?;
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        ordered
            .into_iter()
            .map(|value| {
                value.ok_or_else(|| {
                    DedupeError::State("Bộ lập lịch theo ổ đĩa làm mất tác vụ đã hoàn tất".into())
                })
            })
            .collect()
    })
}

fn run_worker<T, R, F>(
    jobs: &Receiver<(usize, VolumeJob<T>)>,
    results: &crossbeam_channel::Sender<(usize, Result<R>)>,
    limiters: &HashMap<String, Arc<VolumeLimiter>>,
    control: &ControlToken,
    work: &F,
) where
    T: Send,
    R: Send,
    F: Fn(T, &ControlToken) -> Result<R> + Sync,
{
    while let Ok((index, job)) = jobs.recv() {
        let result = (|| {
            control.checkpoint()?;
            let limiter = limiters.get(&job.volume_id).ok_or_else(|| {
                DedupeError::State(format!(
                    "Bộ lập lịch theo ổ đĩa không có bộ giới hạn cho {}",
                    job.volume_id
                ))
            })?;
            let _permit = limiter.acquire(control)?;
            control.checkpoint()?;
            work(job.value, control)
        })();
        let stop = result.is_err();
        if results.send((index, result)).is_err() || stop {
            return;
        }
    }
}

fn volume_limiters<T>(
    jobs: &[VolumeJob<T>],
    per_volume: usize,
) -> HashMap<String, Arc<VolumeLimiter>> {
    jobs.iter()
        .map(|job| job.volume_id.clone())
        .fold(HashMap::new(), |mut limiters, volume_id| {
            limiters
                .entry(volume_id)
                .or_insert_with(|| Arc::new(VolumeLimiter::new(per_volume)));
            limiters
        })
}

#[derive(Debug)]
struct VolumeLimiter {
    available: Mutex<usize>,
    condition: Condvar,
    capacity: usize,
}

impl VolumeLimiter {
    fn new(capacity: usize) -> Self {
        Self {
            available: Mutex::new(capacity),
            condition: Condvar::new(),
            capacity,
        }
    }

    fn acquire(&self, control: &ControlToken) -> Result<VolumePermit<'_>> {
        let mut available = self.available.lock();
        loop {
            control.checkpoint()?;
            if *available > 0 {
                *available -= 1;
                return Ok(VolumePermit { limiter: self });
            }
            self.condition
                .wait_for(&mut available, Duration::from_millis(25));
        }
    }

    fn release(&self) {
        let mut available = self.available.lock();
        *available = available.saturating_add(1).min(self.capacity);
        self.condition.notify_one();
    }
}

struct VolumePermit<'a> {
    limiter: &'a VolumeLimiter,
}

impl Drop for VolumePermit<'_> {
    fn drop(&mut self) {
        self.limiter.release();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use parking_lot::Mutex;

    use super::*;

    #[test]
    fn recommended_profiles_are_conservative_and_cpu_bounded() {
        assert_eq!(
            recommended(StorageProfile::Hdd, 20).full_hash_workers_per_volume,
            1
        );
        assert_eq!(
            recommended(StorageProfile::SataSsd, 20).full_hash_workers_per_volume,
            2
        );
        assert_eq!(
            recommended(StorageProfile::Nvme, 20).full_hash_workers_per_volume,
            4
        );
        assert_eq!(
            recommended(StorageProfile::Network, 20).full_hash_workers_per_volume,
            1
        );
        assert_eq!(
            recommended(StorageProfile::Unknown, 20).full_hash_workers_per_volume,
            1
        );
        assert_eq!(recommended(StorageProfile::Nvme, 1).metadata_workers, 1);
    }

    #[test]
    fn pool_bounds_global_and_per_volume_parallelism() -> Result<()> {
        let active_total = Arc::new(AtomicUsize::new(0));
        let peak_total = Arc::new(AtomicUsize::new(0));
        let active_by_volume = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
        let peak_by_volume = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
        let jobs = (0..24)
            .map(|index| VolumeJob {
                volume_id: if index % 2 == 0 { "a" } else { "b" }.into(),
                value: index,
            })
            .collect();

        let results = run_volume_jobs(
            jobs,
            WorkerConfig {
                metadata_workers: 4,
                full_hash_workers_per_volume: 2,
                queue_capacity: 2,
            },
            &ControlToken::new(),
            |index, _| {
                let volume = if index % 2 == 0 { "a" } else { "b" };
                let total = active_total.fetch_add(1, Ordering::SeqCst) + 1;
                peak_total.fetch_max(total, Ordering::SeqCst);
                {
                    let mut active = active_by_volume.lock();
                    let current = active.entry(volume.into()).or_default();
                    *current += 1;
                    let mut peaks = peak_by_volume.lock();
                    peaks
                        .entry(volume.into())
                        .and_modify(|peak| *peak = (*peak).max(*current))
                        .or_insert(*current);
                }
                std::thread::sleep(Duration::from_millis(5));
                active_total.fetch_sub(1, Ordering::SeqCst);
                *active_by_volume.lock().entry(volume.into()).or_default() -= 1;
                Ok(index)
            },
        )?;

        assert_eq!(results, (0..24).collect::<Vec<_>>());
        assert!(peak_total.load(Ordering::SeqCst) <= 4);
        assert!(peak_total.load(Ordering::SeqCst) >= 3);
        assert!(peak_by_volume.lock().values().all(|peak| *peak <= 2));
        Ok(())
    }

    #[test]
    fn pause_and_cancel_are_observed_at_job_boundaries() -> Result<()> {
        let control = ControlToken::new();
        control.pause();
        let worker_control = control.clone();
        let completed = Arc::new(AtomicUsize::new(0));
        let worker_completed = Arc::clone(&completed);
        let thread = std::thread::spawn(move || {
            run_volume_jobs(
                (0..12)
                    .map(|value| VolumeJob {
                        volume_id: "volume".into(),
                        value,
                    })
                    .collect(),
                WorkerConfig {
                    metadata_workers: 3,
                    full_hash_workers_per_volume: 1,
                    queue_capacity: 1,
                },
                &worker_control,
                |value, _| {
                    worker_completed.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(3));
                    Ok(value)
                },
            )
        });
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(completed.load(Ordering::SeqCst), 0);
        control.resume();
        while completed.load(Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
        control.cancel();
        assert!(matches!(
            thread
                .join()
                .map_err(|_| DedupeError::State("Luồng kiểm thử bộ lập lịch gặp panic".into()))?,
            Err(DedupeError::Cancelled)
        ));
        assert!(completed.load(Ordering::SeqCst) < 12);
        Ok(())
    }
}
