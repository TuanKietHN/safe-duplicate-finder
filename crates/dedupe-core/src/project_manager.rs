//! Project root validation and overlap elimination.

use std::path::{Path, PathBuf};

use crate::{DedupeError, Result, path_normalization};

/// Validation result for one selected root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootStatus {
    /// Root is eligible and non-overlapping.
    Valid,
    /// Same normalized root was selected more than once.
    Duplicate,
    /// Root is nested under another selected root.
    CoveredBy(PathBuf),
    /// Root is the application quarantine area.
    QuarantineForbidden,
}

/// Selected root plus its validation result.
#[derive(Debug, Clone)]
pub struct ValidatedRoot {
    /// Original path.
    pub path: PathBuf,
    /// Validation state.
    pub status: RootStatus,
}

/// Validate selections and return a minimal set that cannot enumerate one path twice.
pub fn validate_roots(roots: &[PathBuf]) -> Result<Vec<ValidatedRoot>> {
    if roots.is_empty() {
        return Err(DedupeError::InvalidInput(
            "Cần ít nhất một thư mục nguồn".into(),
        ));
    }
    let mut normalized = roots
        .iter()
        .map(|path| Ok((path.clone(), path_normalization::normalize_path(path)?)))
        .collect::<Result<Vec<_>>>()?;
    normalized.sort_by(|left, right| left.1.len().cmp(&right.1.len()).then(left.1.cmp(&right.1)));
    let mut accepted: Vec<(PathBuf, String)> = Vec::new();
    let mut result = Vec::with_capacity(normalized.len());
    for (path, key) in normalized {
        if is_quarantine_path(&path) {
            result.push(ValidatedRoot {
                path,
                status: RootStatus::QuarantineForbidden,
            });
            continue;
        }
        if let Some((existing_path, existing_key)) = accepted
            .iter()
            .find(|(_, existing_key)| path_normalization::is_same_or_child(existing_key, &key))
        {
            let status = if existing_key == &key {
                RootStatus::Duplicate
            } else {
                RootStatus::CoveredBy(existing_path.clone())
            };
            result.push(ValidatedRoot { path, status });
        } else {
            accepted.push((path.clone(), key));
            result.push(ValidatedRoot {
                path,
                status: RootStatus::Valid,
            });
        }
    }
    Ok(result)
}

/// Extract only validated minimal roots.
#[must_use]
pub fn effective_roots(validated: &[ValidatedRoot]) -> Vec<PathBuf> {
    validated
        .iter()
        .filter(|root| root.status == RootStatus::Valid)
        .map(|root| root.path.clone())
        .collect()
}

fn is_quarantine_path(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(".safe-duplicate-finder-quarantine")
    })
}
