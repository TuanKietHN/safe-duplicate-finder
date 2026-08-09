//! Windows metadata and identity through by-handle Win32 APIs.

use std::{ffi::c_void, fs, os::windows::ffi::OsStrExt, path::Path, ptr};

use dedupe_core::{
    DedupeError, Result,
    metadata::snapshot_token,
    model::{AccessStatus, FileIdentity, FileMetadataSnapshot, LinkKind},
    path_normalization::{normalize_name, normalize_path},
    ports::{MetadataProvider, SafeDeleter, SafeMover},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, DELETE, FILE_ATTRIBUTE_DIRECTORY,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileDispositionInfo, FileIdInfo,
        FileRenameInfo, GetFileInformationByHandle, GetFileInformationByHandleEx, OPEN_EXISTING,
        SetFileInformationByHandle,
    },
};

/// Native Windows filesystem implementation.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlatformFileSystem;

impl MetadataProvider for PlatformFileSystem {
    fn snapshot(&self, path: &Path) -> Result<FileMetadataSnapshot> {
        let handle = open_handle(path, FILE_READ_ATTRIBUTES)?;
        let (identity, basic) = information_by_handle(&handle, path)?;
        reject_reparse_or_directory(path, &basic)?;
        let size_bytes = file_size(&basic);
        let modified_ns = windows_ticks_to_unix_ns(filetime_ticks(basic.ftLastWriteTime));
        let created_ns = Some(windows_ticks_to_unix_ns(filetime_ticks(
            basic.ftCreationTime,
        )));
        let link_kind = if basic.nNumberOfLinks > 1 {
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
            size_bytes,
            created_ns,
            modified_ns,
            identity: Some(identity.clone()),
            link_kind,
            hardlink_count: Some(u64::from(basic.nNumberOfLinks)),
            access_status: AccessStatus::Readable,
            snapshot_token: snapshot_token(Some(&identity), size_bytes, modified_ns),
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
        ensure_destination_absent(destination)?;
        let parent = destination.parent().ok_or_else(|| {
            DedupeError::InvalidInput(format!(
                "Đích không có thư mục cha: {}",
                destination.display()
            ))
        })?;
        fs::create_dir_all(parent)
            .map_err(|error| DedupeError::io("tạo thư mục cha cách ly", parent, error))?;
        let source_handle = open_handle(source, FILE_READ_ATTRIBUTES | DELETE)?;
        let (source_identity, source_basic) = information_by_handle(&source_handle, source)?;
        reject_reparse_or_directory(source, &source_basic)?;
        let source_size = file_size(&source_basic);
        let source_modified =
            windows_ticks_to_unix_ns(filetime_ticks(source_basic.ftLastWriteTime));
        let source_token = snapshot_token(Some(&source_identity), source_size, source_modified);
        if expected.identity.as_ref() != Some(&source_identity)
            || expected.size_bytes != source_size
            || expected.snapshot_token != source_token
        {
            return Err(DedupeError::Safety(
                "Handle nguồn không còn khớp với danh tính vật lý đã kiểm tra trước".into(),
            ));
        }
        let (parent_identity, _) = identity_by_handle(parent)?;
        if source_identity.volume_id != parent_identity.volume_id {
            return Err(DedupeError::Safety(
                "Nguồn và đích cách ly không nằm trên cùng ổ đĩa".into(),
            ));
        }
        rename_handle_no_replace(&source_handle, source, destination)
    }
}

impl SafeDeleter for PlatformFileSystem {
    fn delete_exact(&self, expected: &FileMetadataSnapshot) -> Result<()> {
        let path = &expected.path;
        let handle = open_handle(path, FILE_READ_ATTRIBUTES | DELETE)?;
        let (identity, basic) = information_by_handle(&handle, path)?;
        reject_reparse_or_directory(path, &basic)?;
        let size_bytes = file_size(&basic);
        let modified_ns = windows_ticks_to_unix_ns(filetime_ticks(basic.ftLastWriteTime));
        let token = snapshot_token(Some(&identity), size_bytes, modified_ns);
        if expected.identity.as_ref() != Some(&identity)
            || expected.size_bytes != size_bytes
            || expected.snapshot_token != token
        {
            return Err(DedupeError::Safety(
                "Handle xóa không còn khớp với danh tính vật lý đã kiểm tra trước".into(),
            ));
        }
        let mut disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        let buffer_length = u32::try_from(std::mem::size_of::<FILE_DISPOSITION_INFO>())
            .map_err(|_| DedupeError::State("FILE_DISPOSITION_INFO size overflow".into()))?;
        // SAFETY: `disposition` is writable storage of the exact Win32 structure size and `handle`
        // remains open for the call. The opened handle has DELETE access and was identity-checked.
        let deleted = unsafe {
            SetFileInformationByHandle(
                handle.0,
                FileDispositionInfo,
                ptr::from_mut(&mut disposition).cast::<c_void>(),
                buffer_length,
            )
        };
        if deleted == 0 {
            return Err(DedupeError::io(
                "xóa vĩnh viễn gắn với handle",
                path,
                std::io::Error::last_os_error(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` is a valid owned handle returned by `CreateFileW`, and Drop runs once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn identity_by_handle(path: &Path) -> Result<(FileIdentity, u32)> {
    let handle = open_handle(path, FILE_READ_ATTRIBUTES)?;
    let (identity, basic) = information_by_handle(&handle, path)?;
    if basic.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(DedupeError::Safety(format!(
            "Thư mục reparse-point không phải đích thay đổi hợp lệ: {}",
            path.display()
        )));
    }
    Ok((identity, basic.nNumberOfLinks))
}

fn open_handle(path: &Path, access: u32) -> Result<OwnedHandle> {
    let wide_path = long_path_wide(path)?;
    // SAFETY: the UTF-16 buffer is NUL terminated and lives for the duration of the call. All optional
    // pointer parameters are null as permitted by `CreateFileW`.
    let raw = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(DedupeError::io(
            "mở handle danh tính Windows",
            path,
            std::io::Error::last_os_error(),
        ));
    }
    Ok(OwnedHandle(raw))
}

fn information_by_handle(
    handle: &OwnedHandle,
    path: &Path,
) -> Result<(FileIdentity, BY_HANDLE_FILE_INFORMATION)> {
    let mut id_info = FILE_ID_INFO::default();
    // SAFETY: `id_info` points to writable storage of exactly the declared buffer size, and the handle
    // remains owned/open for this call.
    let id_result = unsafe {
        GetFileInformationByHandleEx(
            handle.0,
            FileIdInfo,
            ptr::from_mut(&mut id_info).cast::<c_void>(),
            u32::try_from(std::mem::size_of::<FILE_ID_INFO>()).map_err(|_| {
                DedupeError::State("Kích thước FILE_ID_INFO vượt quá độ dài bộ đệm Win32".into())
            })?,
        )
    };
    if id_result == 0 {
        return Err(DedupeError::io(
            "đọc FILE_ID_INFO của Windows",
            path,
            std::io::Error::last_os_error(),
        ));
    }
    let mut basic = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `basic` is valid writable storage and `handle` remains open.
    let basic_result = unsafe { GetFileInformationByHandle(handle.0, &raw mut basic) };
    if basic_result == 0 {
        return Err(DedupeError::io(
            "đọc số liên kết Windows",
            path,
            std::io::Error::last_os_error(),
        ));
    }
    Ok((
        FileIdentity {
            volume_id: format!("{:016x}", id_info.VolumeSerialNumber),
            file_id: hex_bytes(&id_info.FileId.Identifier),
        },
        basic,
    ))
}

fn reject_reparse_or_directory(path: &Path, basic: &BY_HANDLE_FILE_INFORMATION) -> Result<()> {
    if basic.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(DedupeError::Safety(format!(
            "Bỏ qua symlink, junction và các tệp reparse-point khác: {}",
            path.display()
        )));
    }
    if basic.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return Err(DedupeError::InvalidInput(format!(
            "Cần một tệp thông thường nhưng lại gặp thư mục: {}",
            path.display()
        )));
    }
    Ok(())
}

fn ensure_destination_absent(destination: &Path) -> Result<()> {
    match fs::symlink_metadata(destination) {
        Ok(_) => Err(DedupeError::Safety(format!(
            "Đích đã tồn tại; từ chối ghi đè: {}",
            destination.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(DedupeError::io(
            "kiểm tra sự tồn tại của đích",
            destination,
            error,
        )),
    }
}

fn rename_handle_no_replace(handle: &OwnedHandle, source: &Path, destination: &Path) -> Result<()> {
    // `FILE_RENAME_INFO` still applies the legacy MAX_PATH limit when it receives an ordinary DOS
    // path. Use the verbatim Win32 namespace just as `CreateFileW` does above, then omit the helper's
    // trailing NUL because the structure carries its own byte length and zero-filled terminator.
    let mut destination_wide = long_path_wide(destination)?;
    destination_wide.pop();
    if destination_wide.is_empty() {
        return Err(DedupeError::InvalidInput(format!(
            "Đường dẫn đích không hợp lệ: {}",
            destination.display()
        )));
    }
    let name_bytes = destination_wide
        .len()
        .checked_mul(std::mem::size_of::<u16>())
        .ok_or_else(|| DedupeError::InvalidInput("Đường dẫn đích quá dài".into()))?;
    let header_bytes = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
    let buffer_bytes = header_bytes
        .checked_add(name_bytes)
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<u16>()))
        .ok_or_else(|| DedupeError::InvalidInput("Bộ đệm đổi tên quá lớn".into()))?;
    let word_bytes = std::mem::size_of::<usize>();
    let mut storage = vec![0_usize; buffer_bytes.div_ceil(word_bytes)];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    let name_length = u32::try_from(name_bytes)
        .map_err(|_| DedupeError::InvalidInput("Đường dẫn đích vượt giới hạn Win32".into()))?;
    let buffer_length = u32::try_from(buffer_bytes)
        .map_err(|_| DedupeError::InvalidInput("Bộ đệm đổi tên vượt giới hạn Win32".into()))?;
    // SAFETY: `storage` is usize-aligned and sized for the fixed FILE_RENAME_INFO prefix plus the
    // complete UTF-16 filename plus a NUL terminator. `FileNameLength` excludes that terminator as
    // required by FILE_RENAME_INFO. The handle stays open and owns the validated physical file.
    let renamed = unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).RootDirectory = ptr::null_mut();
        (*info).FileNameLength = name_length;
        ptr::copy_nonoverlapping(
            destination_wide.as_ptr(),
            ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            destination_wide.len(),
        );
        SetFileInformationByHandle(
            handle.0,
            FileRenameInfo,
            info.cast::<c_void>(),
            buffer_length,
        )
    };
    if renamed == 0 {
        return Err(DedupeError::io(
            "đổi tên cùng ổ đĩa, không thay thế và gắn với handle",
            source,
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

fn file_size(basic: &BY_HANDLE_FILE_INFORMATION) -> u64 {
    (u64::from(basic.nFileSizeHigh) << 32) | u64::from(basic.nFileSizeLow)
}

fn filetime_ticks(value: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

fn long_path_wide(path: &Path) -> Result<Vec<u16>> {
    let absolute = std::path::absolute(path)
        .map_err(|error| DedupeError::io("xác định đường dẫn dài Windows", path, error))?;
    let display = absolute.as_os_str().to_string_lossy();
    let verbatim = if display.starts_with(r"\\?\") {
        display.into_owned()
    } else if let Some(unc) = display.strip_prefix(r"\\") {
        format!(r"\\?\UNC\{unc}")
    } else {
        format!(r"\\?\{display}")
    };
    Ok(std::ffi::OsStr::new(&verbatim)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect())
}

fn windows_ticks_to_unix_ns(ticks: u64) -> i128 {
    const TICKS_PER_SECOND: u64 = 10_000_000;
    const WINDOWS_TO_UNIX_SECONDS: u64 = 11_644_473_600;
    let seconds = ticks / TICKS_PER_SECOND;
    let subsecond_ticks = ticks % TICKS_PER_SECOND;
    (i128::from(seconds) - i128::from(WINDOWS_TO_UNIX_SECONDS)) * 1_000_000_000
        + i128::from(subsecond_ticks) * 100
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}
