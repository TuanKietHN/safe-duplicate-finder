//! Real Windows reparse fixtures proving the scanner never follows links or junction loops.

#![cfg(windows)]

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use dedupe_core::{
    DedupeError, Result,
    control::ControlToken,
    filters::{CompiledFilter, FilterConfig},
    model::FileMetadataSnapshot,
    ports::ScanSink,
    progress::ProgressCounters,
    scanner::scan_roots,
};
use dedupe_platform::PlatformFileSystem;

#[derive(Default)]
struct RecordingSink {
    paths: Vec<PathBuf>,
    errors: Vec<PathBuf>,
}

impl ScanSink for RecordingSink {
    fn record(&mut self, snapshot: &FileMetadataSnapshot) -> Result<()> {
        self.paths.push(snapshot.path.clone());
        Ok(())
    }

    fn record_error(&mut self, path: &Path, _error: &DedupeError) -> Result<()> {
        self.errors.push(path.to_path_buf());
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn file_symlink_external_junction_and_junction_loop_are_not_followed()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path().join("source");
    let outside = temporary.path().join("outside");
    std::fs::create_dir(&root)?;
    std::fs::create_dir(&outside)?;
    let source = root.join("source.pdf");
    let secret = outside.join("must-not-be-scanned.pdf");
    std::fs::write(&source, b"source")?;
    std::fs::write(&secret, b"outside")?;

    let file_link_path = root.join("file-link.pdf");
    std::os::windows::fs::symlink_file(&secret, &file_link_path)?;
    let file_link = FileLinkGuard(file_link_path.clone());
    let external_junction = JunctionGuard::create(&root.join("external-junction"), &outside)?;
    let loop_junction = JunctionGuard::create(&root.join("loop-junction"), &root)?;

    let mut sink = RecordingSink::default();
    let progress = scan_roots(
        &[root],
        &CompiledFilter::new(FilterConfig {
            include_extensions: Vec::new(),
            skip_hidden: false,
            skip_system: false,
            ..FilterConfig::default()
        })?,
        &PlatformFileSystem,
        &mut sink,
        &ControlToken::new(),
        &ProgressCounters::default(),
    )?;

    assert_eq!(sink.paths, vec![source]);
    assert!(sink.errors.is_empty());
    assert!(progress.skipped >= 3);
    assert_eq!(std::fs::read(&secret)?, b"outside");

    drop(loop_junction);
    drop(external_junction);
    drop(file_link);
    Ok(())
}

struct FileLinkGuard(PathBuf);

impl Drop for FileLinkGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct JunctionGuard(PathBuf);

impl JunctionGuard {
    fn create(path: &Path, target: &Path) -> Result<Self> {
        let output = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(path)
            .arg(target)
            .output()
            .map_err(|error| DedupeError::io("launch junction fixture command", path, error))?;
        if !output.status.success() {
            return Err(DedupeError::State(format!(
                "could not create junction fixture {} -> {}: {}",
                path.display(),
                target.display(),
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(Self(path.to_path_buf()))
    }
}

impl Drop for JunctionGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}
