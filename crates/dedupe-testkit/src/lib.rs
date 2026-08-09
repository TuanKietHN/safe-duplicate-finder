//! Deterministic filesystem fixtures for safety and recovery tests.

pub mod faults;

use std::{fs, io::Write, path::Path};

use tempfile::TempDir;

/// Isolated fixture tree removed automatically after a test.
#[derive(Debug)]
pub struct Fixture {
    root: TempDir,
}

impl Fixture {
    /// Create an empty fixture.
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            root: tempfile::tempdir()?,
        })
    }

    /// Fixture root.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.root.path()
    }

    /// Create a file and all parents with exact bytes.
    pub fn write(&self, relative: &str, bytes: &[u8]) -> std::io::Result<std::path::PathBuf> {
        let path = self.root.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(&path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(path)
    }
}
