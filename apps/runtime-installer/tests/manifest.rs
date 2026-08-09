use safe_dedupe_runtime_installer::manifest::{RuntimeManifest, validate_manifest};

fn embedded() -> RuntimeManifest {
    serde_json::from_str(include_str!("../../../installer/runtime-manifest.json"))
        .expect("embedded fixture must parse")
}

#[test]
fn release_manifest_is_strict_and_valid() {
    let manifest = embedded();
    validate_manifest(&manifest).expect("release manifest must validate");
    assert_eq!(manifest.artifacts.len(), 1);
    assert_eq!(manifest.artifacts[0].size_bytes, 209_605_840);
    assert_eq!(manifest.artifacts[0].sha256.len(), 64);
}

#[test]
fn rejects_non_https_zero_length_and_bad_sha() {
    let mut manifest = embedded();
    manifest.artifacts[0].url = "http://example.invalid/runtime.exe".into();
    assert!(validate_manifest(&manifest).is_err());

    let mut manifest = embedded();
    manifest.artifacts[0].size_bytes = 0;
    assert!(validate_manifest(&manifest).is_err());

    let mut manifest = embedded();
    manifest.artifacts[0].sha256 = "not-a-digest".into();
    assert!(validate_manifest(&manifest).is_err());
}

#[test]
fn rejects_duplicate_ids_and_unsafe_cache_names() {
    let mut manifest = embedded();
    manifest.artifacts.push(manifest.artifacts[0].clone());
    assert!(validate_manifest(&manifest).is_err());

    let mut manifest = embedded();
    manifest.artifacts[0].cache_file_name = "..\\outside.exe".into();
    assert!(validate_manifest(&manifest).is_err());
}
