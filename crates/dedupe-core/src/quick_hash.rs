//! Sampled head/middle/tail digest used only to reject unequal files.

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

use crate::{
    DedupeError, Result,
    control::ControlToken,
    metadata::is_stable,
    model::{HashAlgorithm, HashResult},
    ports::MetadataProvider,
};

/// Default number of bytes sampled from each region.
pub const SAMPLE_BYTES: u64 = 64 * 1024;

/// Hash distinct head, middle, and tail regions with explicit offsets and file size.
pub fn hash_file(
    path: &Path,
    provider: &dyn MetadataProvider,
    control: &ControlToken,
) -> Result<HashResult> {
    control.checkpoint()?;
    let before = provider.snapshot(path)?;
    let mut file =
        File::open(path).map_err(|error| DedupeError::io("mở để băm nhanh", path, error))?;
    let offsets = sample_offsets(before.size_bytes);
    let mut hasher = blake3::Hasher::new_derive_key("safe-dedupe sampled quick hash v1");
    hasher.update(&before.size_bytes.to_le_bytes());
    let mut bytes_read = 0_u64;
    for offset in offsets {
        control.checkpoint()?;
        file.seek(SeekFrom::Start(offset))
            .map_err(|error| DedupeError::io("di chuyển vị trí để băm nhanh", path, error))?;
        let wanted = SAMPLE_BYTES.min(before.size_bytes.saturating_sub(offset));
        let mut buffer = vec![
            0_u8;
            usize::try_from(wanted).map_err(|_| {
                DedupeError::State("Độ dài mẫu không vừa chỉ mục bộ nhớ".into())
            })?
        ];
        file.read_exact(&mut buffer)
            .map_err(|error| DedupeError::io("đọc mẫu băm nhanh", path, error))?;
        hasher.update(&offset.to_le_bytes());
        hasher.update(&wanted.to_le_bytes());
        hasher.update(&buffer);
        bytes_read = bytes_read.saturating_add(wanted);
    }
    let after = provider.snapshot(path)?;
    Ok(HashResult {
        algorithm: HashAlgorithm::QuickBlake3V1,
        digest: hasher.finalize().as_bytes().to_vec(),
        bytes_read,
        snapshot_before: before.snapshot_token,
        snapshot_after: after.snapshot_token,
        stable: is_stable(&before, &after),
    })
}

fn sample_offsets(size: u64) -> Vec<u64> {
    if size <= SAMPLE_BYTES {
        return vec![0];
    }
    let middle = size
        .saturating_div(2)
        .saturating_sub(SAMPLE_BYTES.saturating_div(2));
    let tail = size.saturating_sub(SAMPLE_BYTES);
    let mut offsets = vec![0, middle, tail];
    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

#[cfg(test)]
mod tests {
    use super::{SAMPLE_BYTES, sample_offsets};

    #[test]
    fn small_file_is_sampled_once() {
        assert_eq!(sample_offsets(SAMPLE_BYTES), vec![0]);
    }

    #[test]
    fn large_file_has_three_ordered_regions() {
        let offsets = sample_offsets(SAMPLE_BYTES * 10);
        assert_eq!(offsets.len(), 3);
        assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
