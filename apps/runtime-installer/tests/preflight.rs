use std::fs;

use safe_dedupe_runtime_installer::manifest::{DetectionRule, RuntimeArtifact};
use safe_dedupe_runtime_installer::preflight::{PreflightStatus, preflight_with_detector};
use sha2::{Digest, Sha256};

fn artifact(bytes: &[u8]) -> RuntimeArtifact {
    RuntimeArtifact {
        id: "fixture-runtime".into(),
        display_name: "Fixture".into(),
        architecture: "x64".into(),
        url: "https://example.invalid/runtime.exe".into(),
        size_bytes: u64::try_from(bytes.len()).expect("fixture size"),
        sha256: format!("{:X}", Sha256::digest(bytes)),
        cache_file_name: "fixture.exe".into(),
        install_args: vec!["/silent".into()],
        detection: DetectionRule::Webview2Registry {
            app_guid: "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}".into(),
        },
        max_retries: 1,
    }
}

#[test]
fn installed_runtime_wins_without_touching_cache() {
    let bytes = b"runtime";
    let artifact = artifact(bytes);
    let cache = tempfile::tempdir().expect("cache");
    fs::write(cache.path().join(&artifact.cache_file_name), b"corrupt").expect("seed cache");

    let status =
        preflight_with_detector(&artifact, cache.path(), || Ok(true)).expect("preflight succeeds");
    assert_eq!(status, PreflightStatus::InstalledValid);
    assert!(cache.path().join(&artifact.cache_file_name).exists());
}

#[test]
fn valid_completed_cache_is_reused() {
    let bytes = b"verified runtime cache";
    let artifact = artifact(bytes);
    let cache = tempfile::tempdir().expect("cache");
    fs::write(cache.path().join(&artifact.cache_file_name), bytes).expect("seed cache");

    let status =
        preflight_with_detector(&artifact, cache.path(), || Ok(false)).expect("preflight succeeds");
    assert_eq!(status, PreflightStatus::CacheValid);
}

#[test]
fn corrupt_completed_cache_is_removed_and_partial_offset_is_preserved() {
    let bytes = b"expected runtime bytes";
    let artifact = artifact(bytes);
    let cache = tempfile::tempdir().expect("cache");
    let complete = cache.path().join(&artifact.cache_file_name);
    let partial = cache
        .path()
        .join(format!("{}.part", artifact.cache_file_name));
    fs::write(&complete, b"same length but altered!").expect("seed corrupt complete");
    fs::write(&partial, &bytes[..7]).expect("seed partial");

    let status =
        preflight_with_detector(&artifact, cache.path(), || Ok(false)).expect("preflight succeeds");
    assert_eq!(status, PreflightStatus::NeedsDownload { resume_offset: 7 });
    assert!(!complete.exists());
    assert!(partial.exists());
}

#[test]
fn oversized_partial_is_discarded() {
    let bytes = b"small";
    let artifact = artifact(bytes);
    let cache = tempfile::tempdir().expect("cache");
    let partial = cache
        .path()
        .join(format!("{}.part", artifact.cache_file_name));
    fs::write(&partial, b"far too large").expect("seed oversized partial");

    let status =
        preflight_with_detector(&artifact, cache.path(), || Ok(false)).expect("preflight succeeds");
    assert_eq!(status, PreflightStatus::NeedsDownload { resume_offset: 0 });
    assert!(!partial.exists());
}
