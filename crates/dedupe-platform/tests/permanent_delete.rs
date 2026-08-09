//! Disposable real-filesystem tests for the handle-bound Windows delete adapter.

#![cfg(windows)]

use dedupe_core::ports::{MetadataProvider, SafeDeleter};
use dedupe_platform::PlatformFileSystem;
use dedupe_testkit::Fixture;

#[test]
fn deletes_the_opened_identity_and_rejects_a_replaced_path()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let path = fixture.write("delete/exact.bin", b"first identity")?;
    let provider = PlatformFileSystem;
    let expected = provider.snapshot(&path)?;
    std::fs::remove_file(&path)?;
    std::fs::write(&path, b"replacement")?;
    assert!(provider.delete_exact(&expected).is_err());
    assert_eq!(std::fs::read(&path)?, b"replacement");

    let replacement = provider.snapshot(&path)?;
    provider.delete_exact(&replacement)?;
    assert!(!path.exists());
    Ok(())
}
