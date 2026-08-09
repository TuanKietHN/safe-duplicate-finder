//! Portable/container builds fail closed for irreversible deletion.

#![cfg(not(windows))]

use dedupe_core::ports::{MetadataProvider, SafeDeleter};
use dedupe_platform::PlatformFileSystem;
use dedupe_testkit::Fixture;

#[test]
fn portable_adapter_never_permanently_deletes() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let path = fixture.write("delete/blocked.bin", b"container must preserve this")?;
    let provider = PlatformFileSystem;
    let expected = provider.snapshot(&path)?;
    assert!(provider.delete_exact(&expected).is_err());
    assert_eq!(std::fs::read(path)?, b"container must preserve this");
    Ok(())
}
