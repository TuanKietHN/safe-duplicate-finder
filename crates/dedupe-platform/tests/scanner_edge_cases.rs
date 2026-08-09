//! Windows scanner fixtures for difficult paths and isolated access failures.

#![cfg(windows)]

use std::{
    fs::OpenOptions,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
};

use dedupe_core::{
    DedupeError, Result,
    control::ControlToken,
    filters::{CompiledFilter, FilterConfig},
    model::FileMetadataSnapshot,
    ports::{MetadataProvider, ScanSink},
    progress::ProgressCounters,
    quick_hash,
    scanner::scan_roots,
};
use dedupe_platform::PlatformFileSystem;
use windows_sys::Win32::{
    Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING},
};

struct FaultProvider;

impl MetadataProvider for FaultProvider {
    fn snapshot(&self, path: &Path) -> Result<FileMetadataSnapshot> {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let failure = match name {
            "vanished.pdf" => Some(std::io::ErrorKind::NotFound),
            "permission-denied.pdf" => Some(std::io::ErrorKind::PermissionDenied),
            "cloud-placeholder.pdf" => Some(std::io::ErrorKind::NotConnected),
            _ => None,
        };
        if let Some(kind) = failure {
            return Err(DedupeError::io(
                "injected scanner edge condition",
                path,
                std::io::Error::new(kind, name),
            ));
        }
        PlatformFileSystem.snapshot(path)
    }
}

#[derive(Default)]
struct RecordingSink {
    snapshots: Vec<FileMetadataSnapshot>,
    errors: Vec<(PathBuf, String)>,
}

impl ScanSink for RecordingSink {
    fn record(&mut self, snapshot: &FileMetadataSnapshot) -> Result<()> {
        self.snapshots.push(snapshot.clone());
        Ok(())
    }

    fn record_error(&mut self, path: &Path, error: &DedupeError) -> Result<()> {
        self.errors.push((path.to_path_buf(), error.to_string()));
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn unicode_long_sparse_locked_and_faulted_files_are_isolated_without_mutation()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path().join("source");
    std::fs::create_dir(&root)?;
    let unicode = root.join("Tài liệu_日本語_📚.pdf");
    std::fs::write(&unicode, b"unicode-source")?;

    let mut long_parent = root.clone();
    for index in 0..7 {
        long_parent.push(format!("long-directory-segment-{index:02}-abcdefghijklmno"));
    }
    std::fs::create_dir_all(&long_parent)?;
    let long_file = long_parent.join("long-path-document.pdf");
    std::fs::write(&long_file, b"long-path-source")?;
    assert!(long_file.as_os_str().encode_wide().count() > 260);

    let sparse = root.join("large-sparse.pdf");
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&sparse)?
        .set_len((4_u64 << 30) + 17)?;
    let fault_paths = [
        root.join("vanished.pdf"),
        root.join("permission-denied.pdf"),
        root.join("cloud-placeholder.pdf"),
    ];
    for path in &fault_paths {
        std::fs::write(path, b"fault-source-remains")?;
    }
    let locked = root.join("exclusively-locked.pdf");
    std::fs::write(&locked, b"locked-source-remains")?;
    let lock = ExclusiveLock::open(&locked)?;

    let mut sink = RecordingSink::default();
    let progress = scan_roots(
        &[root],
        &all_files_filter()?,
        &FaultProvider,
        &mut sink,
        &ControlToken::new(),
        &ProgressCounters::default(),
    )?;

    let recorded = sink
        .snapshots
        .iter()
        .map(|snapshot| snapshot.path.clone())
        .collect::<Vec<_>>();
    assert!(recorded.contains(&unicode));
    assert!(recorded.contains(&long_file));
    assert!(recorded.contains(&sparse));
    assert!(recorded.contains(&locked));
    assert_eq!(
        sink.snapshots
            .iter()
            .find(|snapshot| snapshot.path == sparse)
            .ok_or("sparse file was not inventoried")?
            .size_bytes,
        (4_u64 << 30) + 17
    );
    assert_eq!(sink.errors.len(), 3);
    assert_eq!(progress.processed_files, 4);
    assert_eq!(progress.errors, 3);
    for expected in [
        "vanished.pdf",
        "permission-denied.pdf",
        "cloud-placeholder.pdf",
    ] {
        assert!(sink.errors.iter().any(|(path, _)| path.ends_with(expected)));
    }
    assert!(matches!(
        quick_hash::hash_file(&locked, &FaultProvider, &ControlToken::new()),
        Err(DedupeError::Io { .. })
    ));

    drop(lock);
    assert_eq!(std::fs::read(&unicode)?, b"unicode-source");
    assert_eq!(std::fs::read(&long_file)?, b"long-path-source");
    assert_eq!(std::fs::metadata(&sparse)?.len(), (4_u64 << 30) + 17);
    for path in &fault_paths {
        assert_eq!(std::fs::read(path)?, b"fault-source-remains");
    }
    assert_eq!(std::fs::read(&locked)?, b"locked-source-remains");
    Ok(())
}

fn all_files_filter() -> Result<CompiledFilter> {
    CompiledFilter::new(FilterConfig {
        include_extensions: Vec::new(),
        skip_hidden: false,
        skip_system: false,
        ..FilterConfig::default()
    })
}

struct ExclusiveLock(HANDLE);

impl ExclusiveLock {
    fn open(path: &Path) -> Result<Self> {
        let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        wide.push(0);
        // SAFETY: `wide` is a live NUL-terminated UTF-16 path; optional security/template pointers are
        // null as allowed by `CreateFileW`. The returned handle is owned by `ExclusiveLock`.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(DedupeError::io(
                "open exclusive test lock",
                path,
                std::io::Error::last_os_error(),
            ));
        }
        Ok(Self(handle))
    }
}

impl Drop for ExclusiveLock {
    fn drop(&mut self) {
        // SAFETY: this handle was returned by `CreateFileW` and is closed exactly once here.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
