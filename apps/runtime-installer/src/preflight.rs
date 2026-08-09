//! Installed-runtime and cache preflight.

use std::fs;
use std::path::Path;

use thiserror::Error;

use crate::download::{DownloadError, verify_file};
use crate::manifest::{DetectionRule, RuntimeArtifact};

/// Result of independently checking one required runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreflightStatus {
    /// A valid installed runtime was detected; no cache/network is required.
    InstalledValid,
    /// Completed cache passed exact length and SHA-256.
    CacheValid,
    /// Network transfer is required, optionally from a retained offset.
    NeedsDownload {
        /// Existing `.part` byte length.
        resume_offset: u64,
    },
}

/// Preflight failure.
#[derive(Debug, Error)]
pub enum PreflightError {
    /// Cache I/O or content verification failed.
    #[error(transparent)]
    Download(#[from] DownloadError),
    /// Installed-runtime detection failed unexpectedly.
    #[error("không thể kiểm tra Runtime đã cài: {0}")]
    Detection(String),
}

/// Check the actual Windows runtime registration and local verified cache.
pub fn preflight_artifact(
    artifact: &RuntimeArtifact,
    cache_dir: &Path,
) -> Result<PreflightStatus, PreflightError> {
    preflight_with_detector(artifact, cache_dir, || detect_installed(artifact))
}

/// Query only installed-runtime evidence without consulting cache.
pub fn runtime_is_installed(artifact: &RuntimeArtifact) -> Result<bool, PreflightError> {
    detect_installed(artifact)
}

/// Check with an injectable installed-runtime detector for deterministic tests.
pub fn preflight_with_detector<F>(
    artifact: &RuntimeArtifact,
    cache_dir: &Path,
    detector: F,
) -> Result<PreflightStatus, PreflightError>
where
    F: FnOnce() -> Result<bool, PreflightError>,
{
    if detector()? {
        return Ok(PreflightStatus::InstalledValid);
    }

    fs::create_dir_all(cache_dir).map_err(DownloadError::from)?;
    let complete_path = cache_dir.join(&artifact.cache_file_name);
    if verify_file(&complete_path, artifact)? {
        return Ok(PreflightStatus::CacheValid);
    }
    remove_if_present(&complete_path)?;

    let part_path = cache_dir.join(format!("{}.part", artifact.cache_file_name));
    let mut resume_offset = fs::metadata(&part_path).map_or(0, |metadata| metadata.len());
    if resume_offset > artifact.size_bytes {
        remove_if_present(&part_path)?;
        resume_offset = 0;
    }
    Ok(PreflightStatus::NeedsDownload { resume_offset })
}

fn remove_if_present(path: &Path) -> Result<(), PreflightError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(DownloadError::Io(error).into()),
    }
}

#[cfg(not(windows))]
fn detect_installed(_artifact: &RuntimeArtifact) -> Result<bool, PreflightError> {
    Ok(false)
}

#[cfg(windows)]
fn detect_installed(artifact: &RuntimeArtifact) -> Result<bool, PreflightError> {
    match &artifact.detection {
        DetectionRule::Webview2Registry { app_guid } => Ok(webview2_registered(app_guid)),
    }
}

#[cfg(windows)]
fn webview2_registered(app_guid: &str) -> bool {
    use std::ffi::c_void;

    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
    };

    fn read_version(root: HKEY, key: &str) -> Option<String> {
        let key = wide(key);
        let value_name = wide("pv");
        let mut buffer = [0_u16; 128];
        let mut byte_count = u32::try_from(std::mem::size_of_val(&buffer)).ok()?;
        // SAFETY: Registry root is predefined; key/value are null-terminated and output buffer is live.
        let result = unsafe {
            RegGetValueW(
                root,
                key.as_ptr(),
                value_name.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast::<c_void>(),
                &raw mut byte_count,
            )
        };
        if result != ERROR_SUCCESS {
            return None;
        }
        let units = usize::try_from(byte_count)
            .ok()?
            .div_ceil(std::mem::size_of::<u16>());
        let end = buffer
            .iter()
            .take(units.min(buffer.len()))
            .position(|unit| *unit == 0)
            .unwrap_or(units.min(buffer.len()));
        Some(String::from_utf16_lossy(&buffer[..end]))
    }

    let keys = [
        (
            HKEY_LOCAL_MACHINE,
            format!("SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\{app_guid}"),
        ),
        (
            HKEY_LOCAL_MACHINE,
            format!("SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{app_guid}"),
        ),
        (
            HKEY_CURRENT_USER,
            format!("SOFTWARE\\Microsoft\\EdgeUpdate\\Clients\\{app_guid}"),
        ),
    ];
    keys.into_iter().any(|(root, key)| {
        read_version(root, &key).is_some_and(|version| {
            let version = version.trim();
            !version.is_empty() && version != "0.0.0.0"
        })
    })
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
