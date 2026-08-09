//! Runtime manifest parsing and fail-closed validation.

use std::collections::HashSet;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Immutable manifest embedded in one setup release.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeManifest {
    /// Manifest schema version.
    pub schema_version: u32,
    /// Product release version.
    pub release_version: String,
    /// Rust/Windows target architecture.
    pub architecture: String,
    /// Required non-system runtime artifacts.
    pub artifacts: Vec<RuntimeArtifact>,
}

/// One content-pinned runtime artifact.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeArtifact {
    /// Stable identifier used by cache and progress records.
    pub id: String,
    /// User-facing name.
    pub display_name: String,
    /// Runtime architecture.
    pub architecture: String,
    /// Immutable HTTPS download URL.
    pub url: String,
    /// Exact expected byte length.
    pub size_bytes: u64,
    /// Exact expected SHA-256 in hexadecimal.
    pub sha256: String,
    /// Single safe filename inside the product cache root.
    pub cache_file_name: String,
    /// Argument vector passed to the verified installer.
    pub install_args: Vec<String>,
    /// Installed-runtime preflight rule.
    pub detection: DetectionRule,
    /// Bounded network retry count.
    pub max_retries: u8,
}

/// Supported installed-runtime detection rules.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DetectionRule {
    /// Detect WebView2 Evergreen using its Edge Update client registration.
    Webview2Registry {
        /// Microsoft WebView2 application GUID.
        app_guid: String,
    },
}

/// Manifest validation failure.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    /// Unsupported schema.
    #[error("phiên bản schema manifest không được hỗ trợ: {0}")]
    UnsupportedSchema(u32),
    /// Invalid manifest-level field.
    #[error("trường manifest không hợp lệ: {0}")]
    InvalidManifest(&'static str),
    /// Invalid artifact field.
    #[error("artifact {id} không hợp lệ: {field}")]
    InvalidArtifact {
        /// Artifact identifier, or a placeholder when the ID itself is invalid.
        id: String,
        /// Field/reason.
        field: &'static str,
    },
    /// Artifact IDs must be unique.
    #[error("artifact bị trùng ID: {0}")]
    DuplicateId(String),
}

/// Parse and validate one manifest JSON document.
pub fn parse_manifest(json: &str) -> Result<RuntimeManifest, ManifestLoadError> {
    let manifest: RuntimeManifest = serde_json::from_str(json)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Parse/validation error.
#[derive(Debug, Error)]
pub enum ManifestLoadError {
    /// JSON shape or value type was invalid.
    #[error("JSON manifest không hợp lệ: {0}")]
    Json(#[from] serde_json::Error),
    /// Semantic contract was invalid.
    #[error(transparent)]
    Validation(#[from] ManifestError),
}

/// Validate all security-sensitive manifest invariants.
pub fn validate_manifest(manifest: &RuntimeManifest) -> Result<(), ManifestError> {
    if manifest.schema_version != 1 {
        return Err(ManifestError::UnsupportedSchema(manifest.schema_version));
    }
    if manifest.release_version.trim().is_empty() {
        return Err(ManifestError::InvalidManifest("release_version"));
    }
    if manifest.architecture != "x86_64-pc-windows-msvc" {
        return Err(ManifestError::InvalidManifest("architecture"));
    }
    if manifest.artifacts.is_empty() {
        return Err(ManifestError::InvalidManifest("artifacts"));
    }

    let mut ids = HashSet::with_capacity(manifest.artifacts.len());
    for artifact in &manifest.artifacts {
        validate_artifact(artifact)?;
        if !ids.insert(artifact.id.as_str()) {
            return Err(ManifestError::DuplicateId(artifact.id.clone()));
        }
    }
    Ok(())
}

fn validate_artifact(artifact: &RuntimeArtifact) -> Result<(), ManifestError> {
    let shown_id = if artifact.id.is_empty() {
        "<trống>".to_owned()
    } else {
        artifact.id.clone()
    };
    let invalid = |field| ManifestError::InvalidArtifact {
        id: shown_id.clone(),
        field,
    };

    if artifact.id.is_empty()
        || !artifact
            .id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(invalid("id"));
    }
    if artifact.display_name.trim().is_empty() {
        return Err(invalid("display_name"));
    }
    if artifact.architecture != "x64" {
        return Err(invalid("architecture"));
    }
    if !artifact.url.starts_with("https://")
        || artifact.url.bytes().any(|byte| byte.is_ascii_whitespace())
        || artifact.url.contains('#')
    {
        return Err(invalid("url HTTPS bất biến"));
    }
    if artifact.size_bytes == 0 {
        return Err(invalid("size_bytes"));
    }
    if artifact.sha256.len() != 64 || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(invalid("sha256"));
    }
    let cache_path = Path::new(&artifact.cache_file_name);
    let mut components = cache_path.components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || !artifact.cache_file_name.ends_with(".exe")
    {
        return Err(invalid("cache_file_name"));
    }
    if artifact.install_args.is_empty()
        || artifact
            .install_args
            .iter()
            .any(|arg| arg.is_empty() || arg.contains('\0'))
    {
        return Err(invalid("install_args"));
    }
    if !(1..=10).contains(&artifact.max_retries) {
        return Err(invalid("max_retries"));
    }
    match &artifact.detection {
        DetectionRule::Webview2Registry { app_guid }
            if app_guid.len() == 38 && app_guid.starts_with('{') && app_guid.ends_with('}') => {}
        DetectionRule::Webview2Registry { .. } => return Err(invalid("detection.app_guid")),
    }
    Ok(())
}
