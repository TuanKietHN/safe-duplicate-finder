//! Conservative non-Windows metadata and Linux no-replace same-filesystem rename.

use std::{fs, os::unix::fs::MetadataExt, path::Path, time::UNIX_EPOCH};

use dedupe_core::{
    DedupeError, Result,
    metadata::snapshot_token,
    model::{AccessStatus, FileIdentity, FileMetadataSnapshot, LinkKind},
    path_normalization::{normalize_name, normalize_path},
    ports::{MetadataProvider, SafeDeleter, SafeMover},
};

/// Portable read-only filesystem implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlatformFileSystem;

impl MetadataProvider for PlatformFileSystem {
    fn snapshot(&self, path: &Path) -> Result<FileMetadataSnapshot> {
        let metadata = fs::metadata(path)
            .map_err(|error| DedupeError::io("đọc siêu dữ liệu portable", path, error))?;
        let identity = Some(FileIdentity {
            volume_id: format!("{:016x}", metadata.dev()),
            file_id: format!("{:016x}", metadata.ino()),
        });
        let modified_ns = system_time_ns(
            metadata
                .modified()
                .map_err(|error| DedupeError::io("đọc thời điểm sửa đổi", path, error))?,
        );
        let created_ns = metadata.created().ok().map(system_time_ns);
        let hardlink_count = Some(metadata.nlink());
        let link_kind = if metadata.file_type().is_symlink() {
            LinkKind::Symlink
        } else if metadata.nlink() > 1 {
            LinkKind::HardLink
        } else {
            LinkKind::Regular
        };
        Ok(FileMetadataSnapshot {
            path: path.to_path_buf(),
            normalized_path: normalize_path(path)?,
            normalized_name: normalize_name(path),
            extension: path
                .extension()
                .map(|extension| extension.to_string_lossy().to_lowercase()),
            size_bytes: metadata.len(),
            created_ns,
            modified_ns,
            identity: identity.clone(),
            link_kind,
            hardlink_count,
            access_status: AccessStatus::Readable,
            snapshot_token: snapshot_token(identity.as_ref(), metadata.len(), modified_ns),
        })
    }
}

impl SafeMover for PlatformFileSystem {
    fn move_no_replace(
        &self,
        source: &Path,
        destination: &Path,
        expected: &FileMetadataSnapshot,
    ) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            move_linux_no_replace(source, destination, expected)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(DedupeError::Safety(
                "Thao tác thay đổi portable không khả dụng trên hệ điều hành này; hãy dùng Windows hoặc Linux"
                    .into(),
            ))
        }
    }
}

impl SafeDeleter for PlatformFileSystem {
    fn delete_exact(&self, _expected: &FileMetadataSnapshot) -> Result<()> {
        Err(DedupeError::Safety(
            "Xóa vĩnh viễn không khả dụng trong bản portable/container; hãy dùng quy trình desktop hoặc CLI Windows có bảo vệ"
                .into(),
        ))
    }
}

#[cfg(target_os = "linux")]
fn move_linux_no_replace(
    source: &Path,
    destination: &Path,
    expected: &FileMetadataSnapshot,
) -> Result<()> {
    ensure_destination_absent(destination)?;
    let parent = destination.parent().ok_or_else(|| {
        DedupeError::InvalidInput(format!(
            "Đích không có thư mục cha: {}",
            destination.display()
        ))
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| DedupeError::io("tạo thư mục cha cách ly portable", parent, error))?;
    let before = PlatformFileSystem.snapshot(source)?;
    if before.identity != expected.identity
        || before.size_bytes != expected.size_bytes
        || before.snapshot_token != expected.snapshot_token
        || before.link_kind != LinkKind::Regular
        || before.hardlink_count != Some(1)
    {
        return Err(DedupeError::Safety(
            "Nguồn portable không còn khớp với danh tính vật lý đã kiểm tra trước".into(),
        ));
    }
    let source_device = fs::metadata(source)
        .map_err(|error| DedupeError::io("đọc thiết bị nguồn portable", source, error))?
        .dev();
    let parent_device = fs::metadata(parent)
        .map_err(|error| DedupeError::io("đọc thiết bị đích portable", parent, error))?
        .dev();
    if source_device != parent_device {
        return Err(DedupeError::Safety(
            "Nguồn và đích cách ly không nằm trên cùng hệ thống tệp".into(),
        ));
    }
    renameat2_no_replace(source, destination)?;
    let after = PlatformFileSystem.snapshot(destination)?;
    if after.identity != expected.identity
        || after.size_bytes != expected.size_bytes
        || after.snapshot_token != expected.snapshot_token
    {
        return Err(DedupeError::Safety(
            "Danh tính đích portable không khớp với nguồn đã kiểm tra trước sau khi đổi tên".into(),
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn ensure_destination_absent(destination: &Path) -> Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(DedupeError::Safety(format!(
            "Đích đã tồn tại; từ chối ghi đè: {}",
            destination.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(DedupeError::io(
            "kiểm tra sự tồn tại của đích portable",
            destination,
            error,
        )),
    }
}

#[cfg(target_os = "linux")]
fn renameat2_no_replace(source: &Path, destination: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};

    let source_bytes = CString::new(source.as_os_str().as_bytes()).map_err(|_| {
        DedupeError::InvalidInput(format!(
            "Đường dẫn nguồn chứa ký tự NUL: {}",
            source.display()
        ))
    })?;
    let destination_bytes = CString::new(destination.as_os_str().as_bytes()).map_err(|_| {
        DedupeError::InvalidInput(format!(
            "Đường dẫn đích chứa ký tự NUL: {}",
            destination.display()
        ))
    })?;
    // SAFETY: both `CString` pointers are live and NUL terminated for the call. `AT_FDCWD` makes the
    // supplied absolute/relative paths authoritative, and `RENAME_NOREPLACE` is the only flag.
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            libc::AT_FDCWD,
            source_bytes.as_ptr(),
            libc::AT_FDCWD,
            destination_bytes.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::EEXIST) => Err(DedupeError::Safety(format!(
            "Đích xuất hiện trong lúc đổi tên không thay thế: {}",
            destination.display()
        ))),
        Some(libc::EXDEV) => Err(DedupeError::Safety(
            "Kernel từ chối đổi tên cách ly giữa hai hệ thống tệp".into(),
        )),
        _ => Err(DedupeError::io(
            "Linux renameat2 không thay thế",
            source,
            error,
        )),
    }
}

fn system_time_ns(value: std::time::SystemTime) -> i128 {
    value.duration_since(UNIX_EPOCH).map_or_else(
        |error| -i128::try_from(error.duration().as_nanos()).unwrap_or(i128::MAX),
        |duration| i128::try_from(duration.as_nanos()).unwrap_or(i128::MAX),
    )
}
