//! Layered duplicate confirmation for one bounded preliminary candidate group.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use uuid::Uuid;

use crate::{
    Result,
    control::ControlToken,
    full_hash,
    model::{
        ComparisonMode, DuplicateGroup, DuplicateMember, FileMetadataSnapshot, MemberAction,
        ProvenFile, WorkerConfig,
    },
    ports::MetadataProvider,
    quick_hash,
    scheduler::{VolumeJob, run_volume_jobs},
};

type StableHashGroups = HashMap<Vec<u8>, Vec<(FileMetadataSnapshot, crate::model::HashResult)>>;

/// One file-scoped read failure isolated from the rest of its candidate group.
#[derive(Debug)]
pub struct DetectionFileError {
    /// Path that could not complete the evidence stage.
    pub path: PathBuf,
    /// Stage at which the failure occurred.
    pub stage: &'static str,
    /// Path-aware error suitable for the durable scan error table.
    pub error: crate::DedupeError,
}

/// Proven groups plus progress/error telemetry for every attempted hash, including rejects.
#[derive(Debug, Default)]
pub struct DetectionOutcome {
    /// Fully proven duplicate groups.
    pub groups: Vec<DuplicateGroup>,
    /// Bytes actually read by quick and full hash passes.
    pub bytes_read: u64,
    /// Files that changed while one evidence stage was reading them.
    pub unstable_files: u64,
    /// File-scoped failures that did not invalidate other candidates.
    pub errors: Vec<DetectionFileError>,
}

/// Confirm duplicates inside one preliminary `(name,size)` or `size` group.
///
/// The caller streams preliminary groups from durable storage, so the entire file population is never
/// retained in memory. Quick evidence only partitions/rejects; full digests create positive groups.
pub fn confirm_preliminary_group(
    mode: ComparisonMode,
    candidates: &[FileMetadataSnapshot],
    provider: &dyn MetadataProvider,
    control: &ControlToken,
) -> Result<Vec<DuplicateGroup>> {
    confirm_preliminary_group_detailed(mode, candidates, provider, control)
        .map(|outcome| outcome.groups)
}

/// Confirm a group while returning complete read/error telemetry for durable adapters.
///
/// Cancellation remains fatal for the requested scan. Every other per-file hash failure is isolated,
/// allowing the remaining readable candidates to continue through independent proof.
pub fn confirm_preliminary_group_detailed(
    mode: ComparisonMode,
    candidates: &[FileMetadataSnapshot],
    provider: &dyn MetadataProvider,
    control: &ControlToken,
) -> Result<DetectionOutcome> {
    confirm_preliminary_group_detailed_with_config(
        mode,
        candidates,
        provider,
        control,
        WorkerConfig {
            metadata_workers: 1,
            full_hash_workers_per_volume: 1,
            queue_capacity: 1,
        },
    )
}

/// Confirm a group using a bounded global pool and an independent full-read limit per volume.
///
/// This is the production entry point for adapters. The sequential function remains available for
/// deterministic callers and compatibility tests, but applies the identical evidence rules.
pub fn confirm_preliminary_group_detailed_with_config(
    mode: ComparisonMode,
    candidates: &[FileMetadataSnapshot],
    provider: &dyn MetadataProvider,
    control: &ControlToken,
    workers: WorkerConfig,
) -> Result<DetectionOutcome> {
    if candidates.len() < 2 {
        return Ok(DetectionOutcome::default());
    }
    let mut outcome = DetectionOutcome::default();
    let expected_size = candidates[0].size_bytes;
    let expected_name = candidates[0].normalized_name.clone();
    let eligible = candidates
        .iter()
        .filter(|candidate| {
            candidate.size_bytes == expected_size
                && (mode == ComparisonMode::Content || candidate.normalized_name == expected_name)
                && candidate.link_kind != crate::model::LinkKind::HardLink
                && candidate.hardlink_count.unwrap_or(1) == 1
        })
        .cloned()
        .collect::<Vec<_>>();
    let quick_groups = group_stable_hashes(
        eligible,
        "quick_hash",
        |candidate, worker_control| {
            quick_hash::hash_file(&candidate.path, provider, worker_control)
        },
        &mut outcome,
        workers,
        control,
    )?;
    for quick_group in quick_groups.into_values().filter(|group| group.len() >= 2) {
        let quick_survivors = quick_group
            .into_iter()
            .map(|(metadata, _quick_evidence)| metadata)
            .collect();
        let blake_groups = group_stable_hashes(
            quick_survivors,
            "blake3",
            |candidate, worker_control| {
                full_hash::blake3_file(&candidate.path, provider, worker_control)
            },
            &mut outcome,
            workers,
            control,
        )?;
        for (blake3, blake_group) in blake_groups
            .into_iter()
            .filter(|(_, group)| group.len() >= 2)
        {
            let sha_groups = group_sha256(blake_group, provider, control, &mut outcome, workers)?;
            for (sha256, files) in sha_groups.into_iter().filter(|(_, files)| files.len() >= 2) {
                let mut seen_identity = HashSet::new();
                let independent = files
                    .into_iter()
                    .filter(|file| {
                        file.metadata
                            .identity
                            .as_ref()
                            .is_none_or(|identity| seen_identity.insert(identity.clone()))
                    })
                    .collect::<Vec<_>>();
                if independent.len() < 2 {
                    continue;
                }
                let members = independent
                    .into_iter()
                    .map(|file| DuplicateMember {
                        file,
                        action: MemberAction::Manual,
                        reason: "Đang chờ chính sách chọn tệp giữ lại".into(),
                    })
                    .collect();
                outcome.groups.push(DuplicateGroup {
                    id: Uuid::new_v4(),
                    mode,
                    size_bytes: expected_size,
                    normalized_name: (mode == ComparisonMode::Strict)
                        .then(|| expected_name.clone()),
                    blake3: blake3.clone(),
                    sha256,
                    members,
                });
            }
        }
    }
    Ok(outcome)
}

fn group_sha256(
    candidates: Vec<(FileMetadataSnapshot, crate::model::HashResult)>,
    provider: &dyn MetadataProvider,
    control: &ControlToken,
    outcome: &mut DetectionOutcome,
    workers: WorkerConfig,
) -> Result<HashMap<Vec<u8>, Vec<ProvenFile>>> {
    let mut groups: HashMap<Vec<u8>, Vec<ProvenFile>> = HashMap::new();
    let jobs = candidates
        .into_iter()
        .map(|candidate| VolumeJob {
            volume_id: volume_key(&candidate.0),
            value: candidate,
        })
        .collect();
    let results = run_volume_jobs(
        jobs,
        workers,
        control,
        |(metadata, blake3), worker_control| {
            let result = full_hash::sha256_file(&metadata.path, provider, worker_control);
            Ok((metadata, blake3, result))
        },
    )?;
    for (metadata, blake3, result) in results {
        let sha256 = match result {
            Ok(result) => result,
            Err(crate::DedupeError::Cancelled) => return Err(crate::DedupeError::Cancelled),
            Err(error) => {
                outcome.errors.push(DetectionFileError {
                    path: metadata.path,
                    stage: "sha256",
                    error,
                });
                continue;
            }
        };
        outcome.bytes_read = outcome.bytes_read.saturating_add(sha256.bytes_read);
        if !sha256.stable {
            outcome.unstable_files = outcome.unstable_files.saturating_add(1);
            continue;
        }
        groups
            .entry(sha256.digest.clone())
            .or_default()
            .push(ProvenFile {
                metadata,
                blake3,
                sha256,
            });
    }
    Ok(groups)
}

fn group_stable_hashes<F>(
    candidates: Vec<FileMetadataSnapshot>,
    stage: &'static str,
    hash: F,
    outcome: &mut DetectionOutcome,
    workers: WorkerConfig,
    control: &ControlToken,
) -> Result<StableHashGroups>
where
    F: Fn(&FileMetadataSnapshot, &ControlToken) -> Result<crate::model::HashResult> + Sync,
{
    let mut groups = StableHashGroups::new();
    let jobs = candidates
        .into_iter()
        .map(|candidate| VolumeJob {
            volume_id: volume_key(&candidate),
            value: candidate,
        })
        .collect();
    let results = run_volume_jobs(jobs, workers, control, |candidate, worker_control| {
        let result = hash(&candidate, worker_control);
        Ok((candidate, result))
    })?;
    for (candidate, result) in results {
        let result = match result {
            Ok(result) => result,
            Err(crate::DedupeError::Cancelled) => return Err(crate::DedupeError::Cancelled),
            Err(error) => {
                outcome.errors.push(DetectionFileError {
                    path: candidate.path,
                    stage,
                    error,
                });
                continue;
            }
        };
        outcome.bytes_read = outcome.bytes_read.saturating_add(result.bytes_read);
        if result.stable {
            groups
                .entry(result.digest.clone())
                .or_default()
                .push((candidate, result));
        } else {
            outcome.unstable_files = outcome.unstable_files.saturating_add(1);
        }
    }
    Ok(groups)
}

fn volume_key(candidate: &FileMetadataSnapshot) -> String {
    candidate.identity.as_ref().map_or_else(
        || "__unknown_volume__".into(),
        |identity| identity.volume_id.clone(),
    )
}
