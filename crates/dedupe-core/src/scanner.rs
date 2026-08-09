//! Streaming read-only traversal with overlap elimination and isolated file errors.

use std::{
    path::{Path, PathBuf},
    thread,
};

use crossbeam_channel::bounded;
use walkdir::WalkDir;

use crate::{
    DedupeError, Result,
    control::ControlToken,
    filters::CompiledFilter,
    model::{FileMetadataSnapshot, WorkerConfig},
    ports::{MetadataProvider, ScanSink},
    progress::{ProgressCounters, ProgressSnapshot},
    project_manager::{effective_roots, validate_roots},
};

/// Scan selected roots without retaining the complete metadata population in memory.
pub fn scan_roots(
    roots: &[PathBuf],
    filters: &CompiledFilter,
    provider: &dyn MetadataProvider,
    sink: &mut dyn ScanSink,
    control: &ControlToken,
    progress: &ProgressCounters,
) -> Result<ProgressSnapshot> {
    scan_roots_with_config(
        roots,
        filters,
        provider,
        sink,
        control,
        progress,
        WorkerConfig::default(),
    )
}

/// Scan with a fixed metadata-worker count and bounded job/result queues.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn scan_roots_with_config(
    roots: &[PathBuf],
    filters: &CompiledFilter,
    provider: &dyn MetadataProvider,
    sink: &mut dyn ScanSink,
    control: &ControlToken,
    progress: &ProgressCounters,
    workers: WorkerConfig,
) -> Result<ProgressSnapshot> {
    let validated = validate_roots(roots)?;
    let effective = effective_roots(&validated);
    let queue_capacity = workers.queue_capacity.clamp(1, 65_536);
    let worker_count = workers.metadata_workers.clamp(1, 64);
    let (job_sender, job_receiver) = bounded::<PathBuf>(queue_capacity);
    let (result_sender, result_receiver) = bounded::<ScanObservation>(queue_capacity);

    thread::scope(|scope| -> Result<()> {
        let mut worker_handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let jobs = job_receiver.clone();
            let results = result_sender.clone();
            let worker_control = control.clone();
            worker_handles.push(scope.spawn(move || -> Result<()> {
                while let Ok(path) = jobs.recv() {
                    if let Err(error) = worker_control.checkpoint() {
                        let _ = results.send(ScanObservation::Fatal(error));
                        return Ok(());
                    }
                    let observation = match provider.snapshot(&path) {
                        Ok(snapshot) => ScanObservation::Snapshot(snapshot),
                        Err(error) => ScanObservation::FileError(path, error),
                    };
                    if results.send(observation).is_err() {
                        return Ok(());
                    }
                }
                Ok(())
            }));
        }
        drop(job_receiver);

        let enumeration_results = result_sender.clone();
        let enumeration_control = control.clone();
        let enumeration_handle = scope.spawn(move || -> Result<()> {
            for root in effective {
                enumeration_control.checkpoint()?;
                if !root.is_dir() {
                    let error = DedupeError::InvalidInput(format!(
                        "Thư mục nguồn không truy cập được: {}",
                        root.display()
                    ));
                    if enumeration_results
                        .send(ScanObservation::FileError(root, error))
                        .is_err()
                    {
                        return Ok(());
                    }
                    continue;
                }
                let walker = WalkDir::new(&root)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(|entry| {
                        let followed = entry.depth() == 0 || !is_link_or_reparse_entry(entry);
                        if !followed {
                            progress.skipped();
                        }
                        followed
                    });
                for item in walker {
                    enumeration_control.checkpoint()?;
                    let entry = match item {
                        Ok(entry) => entry,
                        Err(error) => {
                            let path = error.path().unwrap_or(&root).to_path_buf();
                            let wrapped = DedupeError::io(
                                "liệt kê thư mục",
                                &path,
                                std::io::Error::other(error.to_string()),
                            );
                            if enumeration_results
                                .send(ScanObservation::FileError(path, wrapped))
                                .is_err()
                            {
                                return Ok(());
                            }
                            continue;
                        }
                    };
                    if entry.file_type().is_symlink() {
                        progress.skipped();
                        continue;
                    }
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    if is_quarantine_path(entry.path()) {
                        progress.skipped();
                        continue;
                    }
                    let (size, hidden, system) = match entry.metadata() {
                        Ok(metadata) => {
                            let (hidden, system) = platform_attribute_flags(&metadata);
                            (metadata.len(), hidden, system)
                        }
                        Err(error) => {
                            let path = entry.path().to_path_buf();
                            let wrapped = DedupeError::io(
                                "đọc siêu dữ liệu khi liệt kê",
                                &path,
                                error.into(),
                            );
                            if enumeration_results
                                .send(ScanObservation::FileError(path, wrapped))
                                .is_err()
                            {
                                return Ok(());
                            }
                            continue;
                        }
                    };
                    if !filters.allows_with_attributes(entry.path(), size, hidden, system) {
                        progress.skipped();
                        continue;
                    }
                    progress.discovered();
                    if job_sender.send(entry.into_path()).is_err() {
                        return Ok(());
                    }
                }
            }
            Ok(())
        });
        drop(result_sender);

        let mut first_error = None;
        for observation in result_receiver {
            if first_error.is_some() {
                continue;
            }
            match observation {
                ScanObservation::Snapshot(snapshot) => match sink.record(&snapshot) {
                    Ok(()) => progress.processed(),
                    Err(error) => first_error = Some(error),
                },
                ScanObservation::FileError(path, error) => {
                    if let Err(sink_error) = sink.record_error(&path, &error) {
                        first_error = Some(sink_error);
                    } else {
                        progress.error();
                    }
                }
                ScanObservation::Fatal(error) => first_error = Some(error),
            }
        }

        let enumeration_result = enumeration_handle
            .join()
            .map_err(|_| DedupeError::State("Luồng liệt kê gặp panic".into()))?;
        if first_error.is_none()
            && let Err(error) = enumeration_result
        {
            first_error = Some(error);
        }
        for handle in worker_handles {
            let worker_result = handle
                .join()
                .map_err(|_| DedupeError::State("Luồng siêu dữ liệu gặp panic".into()))?;
            if first_error.is_none()
                && let Err(error) = worker_result
            {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    })?;
    sink.flush()?;
    Ok(progress.snapshot())
}

enum ScanObservation {
    Snapshot(FileMetadataSnapshot),
    FileError(PathBuf, DedupeError),
    Fatal(DedupeError),
}

fn is_quarantine_path(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(".safe-duplicate-finder-quarantine")
    })
}

#[cfg(windows)]
fn is_link_or_reparse_entry(entry: &walkdir::DirEntry) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    entry.file_type().is_symlink()
        || std::fs::symlink_metadata(entry.path()).map_or(true, |metadata| {
            metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        })
}

#[cfg(not(windows))]
fn is_link_or_reparse_entry(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_symlink()
}

#[cfg(windows)]
fn platform_attribute_flags(metadata: &std::fs::Metadata) -> (bool, bool) {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    let attributes = metadata.file_attributes();
    (
        attributes & FILE_ATTRIBUTE_HIDDEN != 0,
        attributes & FILE_ATTRIBUTE_SYSTEM != 0,
    )
}

#[cfg(not(windows))]
fn platform_attribute_flags(_metadata: &std::fs::Metadata) -> (bool, bool) {
    (false, false)
}
