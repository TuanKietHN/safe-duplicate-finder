//! Cancellable streaming full-content BLAKE3 and SHA-256 passes.

use std::{fs::File, io::Read, path::Path};

use sha2::{Digest, Sha256};

use crate::{
    DedupeError, Result,
    control::ControlToken,
    metadata::is_stable,
    model::{HashAlgorithm, HashResult},
    ports::MetadataProvider,
};

const BUFFER_BYTES: usize = 1024 * 1024;

/// Compute full BLAKE3 using a fixed-size reusable buffer.
pub fn blake3_file(
    path: &Path,
    provider: &dyn MetadataProvider,
    control: &ControlToken,
) -> Result<HashResult> {
    hash_stream(path, provider, control, HashAlgorithm::Blake3)
}

/// Compute full SHA-256 in a separate pass after equal BLAKE3 evidence exists.
pub fn sha256_file(
    path: &Path,
    provider: &dyn MetadataProvider,
    control: &ControlToken,
) -> Result<HashResult> {
    hash_stream(path, provider, control, HashAlgorithm::Sha256)
}

fn hash_stream(
    path: &Path,
    provider: &dyn MetadataProvider,
    control: &ControlToken,
    algorithm: HashAlgorithm,
) -> Result<HashResult> {
    control.checkpoint()?;
    let before = provider.snapshot(path)?;
    let mut file =
        File::open(path).map_err(|error| DedupeError::io("mở để băm đầy đủ", path, error))?;
    let mut buffer = vec![0_u8; BUFFER_BYTES];
    let mut bytes_read = 0_u64;
    let digest = match algorithm {
        HashAlgorithm::Blake3 => {
            let mut hasher = blake3::Hasher::new();
            loop {
                control.checkpoint()?;
                let read = file
                    .read(&mut buffer)
                    .map_err(|error| DedupeError::io("đọc luồng BLAKE3", path, error))?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                bytes_read = bytes_read.saturating_add(read as u64);
            }
            hasher.finalize().as_bytes().to_vec()
        }
        HashAlgorithm::Sha256 => {
            let mut hasher = Sha256::new();
            loop {
                control.checkpoint()?;
                let read = file
                    .read(&mut buffer)
                    .map_err(|error| DedupeError::io("đọc luồng SHA-256", path, error))?;
                if read == 0 {
                    break;
                }
                hasher.update(&buffer[..read]);
                bytes_read = bytes_read.saturating_add(read as u64);
            }
            hasher.finalize().to_vec()
        }
        HashAlgorithm::QuickBlake3V1 => {
            return Err(DedupeError::State(
                "Băm nhanh không thể dùng luồng băm đầy đủ".into(),
            ));
        }
    };
    let after = provider.snapshot(path)?;
    Ok(HashResult {
        algorithm,
        digest,
        bytes_read,
        snapshot_before: before.snapshot_token,
        snapshot_after: after.snapshot_token,
        stable: is_stable(&before, &after),
    })
}
