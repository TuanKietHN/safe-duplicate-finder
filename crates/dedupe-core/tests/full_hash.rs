//! Streaming full-hash control, boundary, and large-file behavior.

use std::{
    path::Path,
    thread,
    time::{Duration, UNIX_EPOCH},
};

use dedupe_core::{
    DedupeError, Result,
    control::ControlToken,
    full_hash,
    metadata::snapshot_token,
    model::{AccessStatus, FileMetadataSnapshot, LinkKind},
    ports::MetadataProvider,
};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy)]
struct HostMetadata;

impl MetadataProvider for HostMetadata {
    fn snapshot(&self, path: &Path) -> Result<FileMetadataSnapshot> {
        let metadata = std::fs::metadata(path)
            .map_err(|error| DedupeError::io("test full-hash metadata", path, error))?;
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| {
                i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX)
            });
        let size_bytes = metadata.len();
        let name = path
            .file_name()
            .ok_or_else(|| DedupeError::InvalidInput("test path has no leaf name".into()))?
            .to_string_lossy()
            .to_lowercase();
        Ok(FileMetadataSnapshot {
            path: path.to_path_buf(),
            normalized_path: path.to_string_lossy().to_lowercase(),
            normalized_name: name,
            extension: path
                .extension()
                .map(|extension| extension.to_string_lossy().to_lowercase()),
            size_bytes,
            created_ns: None,
            modified_ns,
            identity: None,
            link_kind: LinkKind::Regular,
            hardlink_count: Some(1),
            access_status: AccessStatus::Readable,
            snapshot_token: snapshot_token(None, size_bytes, modified_ns),
        })
    }
}

#[test]
fn empty_small_and_multi_chunk_files_match_reference_digests()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    for (name, contents) in [
        ("empty.bin", Vec::new()),
        ("small.bin", b"small deterministic payload".to_vec()),
        ("multi.bin", vec![0xA7; 2 * 1024 * 1024 + 333]),
    ] {
        let path = temporary.path().join(name);
        std::fs::write(&path, &contents)?;
        let blake3 = full_hash::blake3_file(&path, &HostMetadata, &ControlToken::new())?;
        let sha256 = full_hash::sha256_file(&path, &HostMetadata, &ControlToken::new())?;
        assert!(blake3.stable && sha256.stable);
        assert_eq!(blake3.bytes_read, contents.len() as u64);
        assert_eq!(sha256.bytes_read, contents.len() as u64);
        assert_eq!(blake3.digest.as_slice(), blake3::hash(&contents).as_bytes());
        assert_eq!(
            sha256.digest.as_slice(),
            Sha256::digest(&contents).as_slice()
        );
    }
    Ok(())
}

#[test]
fn hashing_waits_while_paused_and_resumes_at_a_chunk_boundary()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("paused.bin");
    std::fs::write(&path, vec![0x5C; 4 * 1024 * 1024])?;
    let control = ControlToken::new();
    control.pause();
    let worker_control = control.clone();
    let worker_path = path.clone();
    let worker =
        thread::spawn(move || full_hash::blake3_file(&worker_path, &HostMetadata, &worker_control));
    thread::sleep(Duration::from_millis(30));
    assert!(!worker.is_finished());
    control.resume();
    let result = worker.join().map_err(|_| "hash worker panicked")??;
    assert!(result.stable);
    assert_eq!(result.bytes_read, 4 * 1024 * 1024);
    Ok(())
}

#[test]
fn cancelled_sparse_file_larger_than_four_gib_stops_before_content_read()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("sparse-over-four-gib.bin");
    let file = std::fs::File::create(&path)?;
    let logical_size = u64::from(u32::MAX) + 8193;
    file.set_len(logical_size)?;
    drop(file);
    assert_eq!(std::fs::metadata(&path)?.len(), logical_size);

    let control = ControlToken::new();
    control.cancel();
    assert!(matches!(
        full_hash::sha256_file(&path, &HostMetadata, &control),
        Err(DedupeError::Cancelled)
    ));
    Ok(())
}
