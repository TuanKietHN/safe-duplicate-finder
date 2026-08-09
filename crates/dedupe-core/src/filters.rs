//! Compiled include/exclude filters applied before any content read.

use std::path::Path;

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::{DedupeError, Result};

/// Scan filter configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterConfig {
    /// Empty means all extensions; otherwise only these lowercase extensions are included.
    pub include_extensions: Vec<String>,
    /// Lowercase extensions to exclude.
    pub exclude_extensions: Vec<String>,
    /// Path globs to exclude.
    pub exclude_globs: Vec<String>,
    /// Ignore files smaller than this exact byte count.
    pub minimum_size: u64,
    /// Skip dot-prefixed files on platforms without hidden attributes.
    pub skip_hidden: bool,
    /// Skip files carrying the Windows system attribute.
    pub skip_system: bool,
}

impl Default for FilterConfig {
    fn default() -> Self {
        Self {
            include_extensions: vec!["pdf".into(), "epub".into(), "mobi".into()],
            exclude_extensions: Vec::new(),
            exclude_globs: Vec::new(),
            minimum_size: 0,
            skip_hidden: true,
            skip_system: true,
        }
    }
}

/// Validated filter ready for high-volume checks.
#[derive(Debug)]
pub struct CompiledFilter {
    config: FilterConfig,
    excluded: GlobSet,
}

impl CompiledFilter {
    /// Compile all globs up front.
    pub fn new(mut config: FilterConfig) -> Result<Self> {
        config.include_extensions = normalize_extensions(config.include_extensions);
        config.exclude_extensions = normalize_extensions(config.exclude_extensions);
        let mut builder = GlobSetBuilder::new();
        for pattern in &config.exclude_globs {
            builder.add(
                Glob::new(pattern).map_err(|error| DedupeError::InvalidInput(error.to_string()))?,
            );
        }
        let excluded = builder
            .build()
            .map_err(|error| DedupeError::InvalidInput(error.to_string()))?;
        Ok(Self { config, excluded })
    }

    /// Decide from path and size only; no file content is opened.
    #[must_use]
    pub fn allows(&self, path: &Path, size_bytes: u64) -> bool {
        self.allows_with_attributes(path, size_bytes, false, false)
    }

    /// Decide with platform hidden/system attributes supplied by read-only enumeration metadata.
    #[must_use]
    pub fn allows_with_attributes(
        &self,
        path: &Path,
        size_bytes: u64,
        hidden_attribute: bool,
        system_attribute: bool,
    ) -> bool {
        if size_bytes < self.config.minimum_size || self.excluded.is_match(path) {
            return false;
        }
        if self.config.skip_system && system_attribute {
            return false;
        }
        if self.config.skip_hidden
            && (hidden_attribute
                || path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.')))
        {
            return false;
        }
        let extension = path
            .extension()
            .map(|value| value.to_string_lossy().to_lowercase());
        if extension
            .as_ref()
            .is_some_and(|value| self.config.exclude_extensions.contains(value))
        {
            return false;
        }
        self.config.include_extensions.is_empty()
            || extension
                .as_ref()
                .is_some_and(|value| self.config.include_extensions.contains(value))
    }
}

fn normalize_extensions(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim_start_matches('.').to_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_size_hidden_and_glob_rules_compose() -> Result<()> {
        let filters = CompiledFilter::new(FilterConfig {
            include_extensions: vec![".PDF".into(), "epub".into()],
            exclude_extensions: vec!["EPUB".into()],
            exclude_globs: vec!["**/cache/**".into()],
            minimum_size: 100,
            skip_hidden: true,
            skip_system: true,
        })?;

        assert!(filters.allows(Path::new("library/book.PDF"), 100));
        assert!(!filters.allows(Path::new("library/book.epub"), 100));
        assert!(!filters.allows(Path::new("library/book.pdf"), 99));
        assert!(!filters.allows(Path::new("library/.hidden.pdf"), 100));
        assert!(!filters.allows(Path::new("library/cache/book.pdf"), 100));
        assert!(!filters.allows(Path::new("library/book.txt"), 100));
        assert!(!filters.allows_with_attributes(Path::new("library/system.pdf"), 100, false, true));
        assert!(!filters.allows_with_attributes(Path::new("library/hidden.pdf"), 100, true, false));
        Ok(())
    }

    #[test]
    fn empty_include_list_means_all_extensions() -> Result<()> {
        let filters = CompiledFilter::new(FilterConfig {
            include_extensions: Vec::new(),
            skip_hidden: false,
            ..FilterConfig::default()
        })?;
        assert!(filters.allows(Path::new("library/file.with-anything"), 0));
        assert!(filters.allows(Path::new("library/no-extension"), 0));
        Ok(())
    }
}
