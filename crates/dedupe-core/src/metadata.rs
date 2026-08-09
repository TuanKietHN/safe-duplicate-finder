//! Metadata token construction and stability comparison.

use crate::model::{FileIdentity, FileMetadataSnapshot};

/// Compute the token used to detect identity, size, or modification changes.
#[must_use]
pub fn snapshot_token(
    identity: Option<&FileIdentity>,
    size_bytes: u64,
    modified_ns: i128,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key("safe-dedupe metadata snapshot v1");
    if let Some(identity) = identity {
        hasher.update(identity.volume_id.as_bytes());
        hasher.update(&[0]);
        hasher.update(identity.file_id.as_bytes());
    }
    hasher.update(&size_bytes.to_le_bytes());
    hasher.update(&modified_ns.to_le_bytes());
    *hasher.finalize().as_bytes()
}

/// True only if every identity-relevant field still matches.
#[must_use]
pub fn is_stable(before: &FileMetadataSnapshot, after: &FileMetadataSnapshot) -> bool {
    before.snapshot_token == after.snapshot_token
        && before.identity == after.identity
        && before.size_bytes == after.size_bytes
        && before.modified_ns == after.modified_ns
}
