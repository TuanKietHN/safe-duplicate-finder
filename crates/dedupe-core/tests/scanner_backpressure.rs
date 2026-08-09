//! Bounded scanner worker and cancellation behavior.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use dedupe_core::{
    DedupeError, Result,
    control::ControlToken,
    filters::{CompiledFilter, FilterConfig},
    metadata::snapshot_token,
    model::{AccessStatus, FileMetadataSnapshot, LinkKind, WorkerConfig},
    ports::{MetadataProvider, ScanSink},
    progress::ProgressCounters,
    scanner::scan_roots_with_config,
};

#[derive(Default)]
struct TrackingProvider {
    active: AtomicUsize,
    peak: AtomicUsize,
}

impl MetadataProvider for TrackingProvider {
    fn snapshot(&self, path: &Path) -> Result<FileMetadataSnapshot> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        let _guard = ActiveGuard(&self.active);
        std::thread::sleep(Duration::from_millis(3));
        let metadata = std::fs::metadata(path)
            .map_err(|error| DedupeError::io("test metadata", path, error))?;
        let name = path
            .file_name()
            .ok_or_else(|| DedupeError::InvalidInput("test path has no name".into()))?
            .to_string_lossy()
            .to_lowercase();
        let size_bytes = metadata.len();
        let modified_ns = 0;
        Ok(FileMetadataSnapshot {
            path: path.to_path_buf(),
            normalized_path: path.to_string_lossy().to_lowercase(),
            normalized_name: name,
            extension: path
                .extension()
                .map(|extension| extension.to_string_lossy().to_lowercase()),
            identity: None,
            size_bytes,
            created_ns: None,
            modified_ns,
            link_kind: LinkKind::Regular,
            hardlink_count: None,
            access_status: AccessStatus::Readable,
            snapshot_token: snapshot_token(None, size_bytes, modified_ns),
        })
    }
}

struct ActiveGuard<'a>(&'a AtomicUsize);

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct CountingSink {
    records: usize,
    errors: usize,
}

impl ScanSink for CountingSink {
    fn record(&mut self, _snapshot: &FileMetadataSnapshot) -> Result<()> {
        self.records += 1;
        Ok(())
    }

    fn record_error(&mut self, _path: &Path, _error: &DedupeError) -> Result<()> {
        self.errors += 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn metadata_parallelism_and_queues_stay_within_configuration()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    for index in 0..24 {
        std::fs::write(
            temporary.path().join(format!("file-{index:02}.bin")),
            [u8::try_from(index)?],
        )?;
    }
    let filters = CompiledFilter::new(FilterConfig {
        include_extensions: Vec::new(),
        skip_hidden: false,
        ..FilterConfig::default()
    })?;
    let provider = TrackingProvider::default();
    let mut sink = CountingSink::default();
    let progress = ProgressCounters::default();

    let snapshot = scan_roots_with_config(
        &[PathBuf::from(temporary.path())],
        &filters,
        &provider,
        &mut sink,
        &ControlToken::new(),
        &progress,
        WorkerConfig {
            metadata_workers: 3,
            full_hash_workers_per_volume: 1,
            queue_capacity: 2,
        },
    )?;

    assert_eq!(sink.records, 24);
    assert_eq!(sink.errors, 0);
    assert_eq!(snapshot.discovered_files, 24);
    assert_eq!(snapshot.processed_files, 24);
    let peak = provider.peak.load(Ordering::SeqCst);
    assert!((2..=3).contains(&peak));
    Ok(())
}

#[test]
fn pre_cancelled_scan_stops_before_enumeration()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    std::fs::write(temporary.path().join("file.bin"), b"content")?;
    let control = ControlToken::new();
    control.cancel();
    let mut sink = CountingSink::default();
    let result = scan_roots_with_config(
        &[temporary.path().to_path_buf()],
        &CompiledFilter::new(FilterConfig {
            include_extensions: Vec::new(),
            ..FilterConfig::default()
        })?,
        &TrackingProvider::default(),
        &mut sink,
        &control,
        &ProgressCounters::default(),
        WorkerConfig {
            metadata_workers: 2,
            full_hash_workers_per_volume: 1,
            queue_capacity: 1,
        },
    );
    assert!(matches!(result, Err(DedupeError::Cancelled)));
    assert_eq!(sink.records, 0);
    Ok(())
}

#[test]
fn paused_enumeration_waits_and_resumes_without_losing_records()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    for index in 0..32 {
        std::fs::write(temporary.path().join(format!("paused-{index}.bin")), [1])?;
    }
    let root = temporary.path().to_path_buf();
    let control = ControlToken::new();
    control.pause();
    let worker_control = control.clone();
    let worker = std::thread::spawn(move || {
        let mut sink = CountingSink::default();
        let result = scan_roots_with_config(
            &[root],
            &CompiledFilter::new(FilterConfig {
                include_extensions: Vec::new(),
                skip_hidden: false,
                ..FilterConfig::default()
            })?,
            &TrackingProvider::default(),
            &mut sink,
            &worker_control,
            &ProgressCounters::default(),
            WorkerConfig {
                metadata_workers: 2,
                full_hash_workers_per_volume: 1,
                queue_capacity: 2,
            },
        );
        result.map(|_| sink.records)
    });
    std::thread::sleep(Duration::from_millis(30));
    assert!(!worker.is_finished());
    control.resume();
    assert_eq!(worker.join().map_err(|_| "scan worker panicked")??, 32);
    Ok(())
}

#[test]
fn cancellation_during_enumeration_stops_after_a_bounded_partial_batch()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    for index in 0..120 {
        std::fs::write(temporary.path().join(format!("cancel-{index}.bin")), [1])?;
    }
    let root = temporary.path().to_path_buf();
    let control = ControlToken::new();
    let worker_control = control.clone();
    let worker = std::thread::spawn(move || {
        let mut sink = CountingSink::default();
        let result = scan_roots_with_config(
            &[root],
            &CompiledFilter::new(FilterConfig {
                include_extensions: Vec::new(),
                skip_hidden: false,
                ..FilterConfig::default()
            })?,
            &TrackingProvider::default(),
            &mut sink,
            &worker_control,
            &ProgressCounters::default(),
            WorkerConfig {
                metadata_workers: 2,
                full_hash_workers_per_volume: 1,
                queue_capacity: 2,
            },
        );
        Ok::<_, DedupeError>((result, sink.records))
    });
    std::thread::sleep(Duration::from_millis(25));
    control.cancel();
    let (result, records) = worker.join().map_err(|_| "scan worker panicked")??;
    assert!(matches!(result, Err(DedupeError::Cancelled)));
    assert!(records < 120);
    Ok(())
}
