//! Linux portable move contract: same filesystem, no replacement, and stale-evidence rejection.

#![cfg(target_os = "linux")]

use dedupe_core::{
    DedupeError,
    ports::{MetadataProvider, SafeMover},
};
use dedupe_platform::PlatformFileSystem;

#[test]
fn linux_move_is_no_replace_and_preserves_identity()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let source = temporary.path().join("source.bin");
    let destination = temporary.path().join("quarantine/entry.bin");
    std::fs::write(&source, b"portable-content")?;
    let expected = PlatformFileSystem.snapshot(&source)?;

    PlatformFileSystem.move_no_replace(&source, &destination, &expected)?;

    assert!(!source.exists());
    assert_eq!(std::fs::read(&destination)?, b"portable-content");
    let moved = PlatformFileSystem.snapshot(&destination)?;
    assert_eq!(moved.identity, expected.identity);
    assert_eq!(moved.snapshot_token, expected.snapshot_token);
    Ok(())
}

#[test]
fn collision_and_stale_evidence_leave_both_sources_untouched()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let source = temporary.path().join("source.bin");
    let destination = temporary.path().join("destination.bin");
    std::fs::write(&source, b"source-content")?;
    std::fs::write(&destination, b"destination-content")?;
    let expected = PlatformFileSystem.snapshot(&source)?;

    assert!(matches!(
        PlatformFileSystem.move_no_replace(&source, &destination, &expected),
        Err(DedupeError::Safety(_))
    ));
    assert_eq!(std::fs::read(&source)?, b"source-content");
    assert_eq!(std::fs::read(&destination)?, b"destination-content");

    std::fs::remove_file(&destination)?;
    std::fs::write(&source, b"changed-content")?;
    assert!(matches!(
        PlatformFileSystem.move_no_replace(&source, &destination, &expected),
        Err(DedupeError::Safety(_))
    ));
    assert_eq!(std::fs::read(&source)?, b"changed-content");
    assert!(!destination.exists());
    Ok(())
}
