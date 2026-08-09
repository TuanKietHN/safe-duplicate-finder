//! Lexical, non-destructive path normalization for comparison and overlap checks.

use std::path::{Component, Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use crate::{DedupeError, Result};

/// Normalize a path without resolving symlinks or requiring it to exist.
pub fn normalize_path(path: &Path) -> Result<String> {
    Ok(lexical_path_text(path)?
        .nfc()
        .collect::<String>()
        .to_lowercase())
}

fn lexical_path_text(path: &Path) -> Result<String> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| DedupeError::io("xác định thư mục hiện tại", path, error))?
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(DedupeError::InvalidInput(format!(
                        "Đường dẫn vượt ra ngoài thư mục gốc: {}",
                        path.display()
                    )));
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    let text = normalized.to_string_lossy().replace('/', "\\");
    let without_verbatim = text.strip_prefix(r"\\?\").unwrap_or(&text);
    Ok(without_verbatim.to_owned())
}

/// Normalize only the leaf filename using the same Unicode/case policy.
#[must_use]
pub fn normalize_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().nfc().collect())
        .to_lowercase()
}

/// Stable opaque key for a normalized path.
pub fn path_key(path: &Path) -> Result<[u8; 32]> {
    Ok(*blake3::hash(normalize_path(path)?.as_bytes()).as_bytes())
}

/// Secondary, spelling-preserving key used only when two distinct filesystem names collapse to
/// the same Unicode-normalized comparison path. The domain prefix keeps this key separate from
/// the long-lived normalized-path key format.
pub fn path_identity_key(path: &Path) -> Result<[u8; 32]> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"exact-path-v1\0");
    hasher.update(lexical_path_text(path)?.as_bytes());
    Ok(*hasher.finalize().as_bytes())
}

/// Whether `candidate` is equal to or lexically nested under `root`.
#[must_use]
pub fn is_same_or_child(root: &str, candidate: &str) -> bool {
    candidate == root
        || candidate
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{is_same_or_child, normalize_name, path_identity_key, path_key};

    #[test]
    fn name_is_case_and_unicode_normalized() {
        assert_eq!(normalize_name(Path::new("C:/Books/CAFÉ.PDF")), "café.pdf");
    }

    #[test]
    fn child_requires_separator_boundary() {
        assert!(is_same_or_child(r"c:\books", r"c:\books\pdf"));
        assert!(!is_same_or_child(r"c:\books", r"c:\books-old"));
    }

    #[test]
    fn identity_key_distinguishes_unicode_spellings_that_comparison_collapses() -> crate::Result<()>
    {
        let composed = Path::new("C:/Books/café.pdf");
        let decomposed = Path::new("C:/Books/cafe\u{301}.pdf");
        assert_eq!(path_key(composed)?, path_key(decomposed)?);
        assert_ne!(path_identity_key(composed)?, path_identity_key(decomposed)?);
        Ok(())
    }
}
