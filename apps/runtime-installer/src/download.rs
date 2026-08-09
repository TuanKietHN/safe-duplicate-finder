//! Resumable WinHTTP downloader.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::manifest::RuntimeArtifact;
use crate::progress::{ItemState, ProgressBook, ProgressError};

/// Fixed streaming buffer used for both downloads and verification.
pub const READ_BUFFER_SIZE: usize = 64 * 1_024;

/// Successful verified cache result.
#[derive(Debug, Clone)]
pub struct DownloadOutcome {
    /// Content-addressed completed artifact.
    pub complete_path: PathBuf,
    /// True when no network request was needed.
    pub reused_cache: bool,
    /// Bytes received from the network during this invocation.
    pub network_bytes: u64,
}

/// Fail-closed download/cache verification error.
#[derive(Debug, Error)]
pub enum DownloadError {
    /// User cancellation was observed at a read boundary.
    #[error("đã hủy tải runtime")]
    Cancelled,
    /// Local cache I/O failed.
    #[error("I/O cache runtime: {0}")]
    Io(#[from] std::io::Error),
    /// Progress accounting refused an invalid byte update.
    #[error(transparent)]
    Progress(#[from] ProgressError),
    /// URL could not be safely decomposed.
    #[error("URL runtime không hợp lệ: {0}")]
    InvalidUrl(String),
    /// WinHTTP call failed.
    #[error("WinHTTP thất bại tại {operation}: {source}")]
    WinHttp {
        /// Operation label.
        operation: &'static str,
        /// Windows error.
        source: std::io::Error,
    },
    /// Server returned an unsupported status code.
    #[error("HTTP trả trạng thái không hợp lệ: {0}")]
    HttpStatus(u32),
    /// Resume response did not begin at the local `.part` length.
    #[error("Content-Range không khớp offset resume")]
    InvalidContentRange,
    /// Received length did not equal the embedded manifest length.
    #[error("sai kích thước runtime: mong đợi {expected}, nhận {actual}")]
    LengthMismatch {
        /// Embedded size.
        expected: u64,
        /// Actual file/response size.
        actual: u64,
    },
    /// Full file SHA-256 did not equal the embedded digest.
    #[error("SHA-256 runtime không khớp: mong đợi {expected}, nhận {actual}")]
    DigestMismatch {
        /// Embedded digest.
        expected: String,
        /// Actual digest.
        actual: String,
    },
    /// The response redirected from HTTPS to a non-HTTPS URL.
    #[error("redirect runtime hạ cấp khỏi HTTPS bị từ chối")]
    InsecureRedirect,
    /// Platform-specific implementation is unavailable.
    #[error("downloader WinHTTP chỉ hỗ trợ Windows")]
    UnsupportedPlatform,
}

/// Reuse, resume, download, and verify one artifact.
pub fn download_artifact(
    artifact: &RuntimeArtifact,
    cache_dir: &Path,
    progress: &ProgressBook,
    cancelled: &AtomicBool,
) -> Result<DownloadOutcome, DownloadError> {
    fs::create_dir_all(cache_dir)?;
    let complete_path = cache_dir.join(&artifact.cache_file_name);
    let part_path = cache_dir.join(format!("{}.part", artifact.cache_file_name));

    progress.set_state(&artifact.id, ItemState::Prechecking)?;
    if verify_file(&complete_path, artifact)? {
        progress.set_existing_bytes(&artifact.id, artifact.size_bytes)?;
        progress.set_state(&artifact.id, ItemState::CacheValid)?;
        return Ok(DownloadOutcome {
            complete_path,
            reused_cache: true,
            network_bytes: 0,
        });
    }
    remove_cache_file_if_present(&complete_path)?;

    let mut existing = fs::metadata(&part_path).map_or(0, |metadata| metadata.len());
    if existing > artifact.size_bytes {
        remove_cache_file_if_present(&part_path)?;
        existing = 0;
    }
    progress.set_existing_bytes(&artifact.id, existing)?;

    if existing == artifact.size_bytes {
        progress.set_state(&artifact.id, ItemState::Verifying)?;
        if verify_file(&part_path, artifact)? {
            promote_verified(&part_path, &complete_path)?;
            progress.set_state(&artifact.id, ItemState::CacheValid)?;
            return Ok(DownloadOutcome {
                complete_path,
                reused_cache: true,
                network_bytes: 0,
            });
        }
        remove_cache_file_if_present(&part_path)?;
        progress.set_existing_bytes(&artifact.id, 0)?;
    }

    let mut network_bytes = 0_u64;
    let mut last_error = None;
    for attempt in 1..=artifact.max_retries {
        if cancelled.load(Ordering::Acquire) {
            progress.set_state(&artifact.id, ItemState::Cancelled)?;
            return Err(DownloadError::Cancelled);
        }
        progress.set_state(&artifact.id, ItemState::Downloading)?;
        match download_once(
            artifact,
            &part_path,
            progress,
            cancelled,
            &mut network_bytes,
        ) {
            Ok(()) => {
                progress.set_state(&artifact.id, ItemState::Verifying)?;
                match verify_file(&part_path, artifact) {
                    Ok(true) => {
                        promote_verified(&part_path, &complete_path)?;
                        progress.set_state(&artifact.id, ItemState::CacheValid)?;
                        return Ok(DownloadOutcome {
                            complete_path,
                            reused_cache: false,
                            network_bytes,
                        });
                    }
                    Ok(false) => {
                        let actual = hash_file(&part_path)?;
                        let error = DownloadError::DigestMismatch {
                            expected: artifact.sha256.clone(),
                            actual,
                        };
                        last_error = Some(error);
                        remove_cache_file_if_present(&part_path)?;
                        progress.set_existing_bytes(&artifact.id, 0)?;
                    }
                    Err(error) => last_error = Some(error),
                }
            }
            Err(DownloadError::Cancelled) => {
                progress.set_state(&artifact.id, ItemState::Cancelled)?;
                return Err(DownloadError::Cancelled);
            }
            Err(error) => last_error = Some(error),
        }

        if attempt < artifact.max_retries {
            std::thread::sleep(Duration::from_millis(250_u64 << (attempt - 1).min(3)));
        }
    }
    progress.set_state(&artifact.id, ItemState::Failed)?;
    Err(last_error.unwrap_or(DownloadError::UnsupportedPlatform))
}

/// Verify exact size and full streaming SHA-256.
pub fn verify_file(path: &Path, artifact: &RuntimeArtifact) -> Result<bool, DownloadError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() != artifact.size_bytes {
        return Ok(false);
    }
    Ok(hash_file(path)?.eq_ignore_ascii_case(&artifact.sha256))
}

/// Reject a redirect that downgrades an originally HTTPS artifact to another scheme.
pub fn validate_final_redirect_scheme(
    original_url: &str,
    final_url: &str,
) -> Result<(), DownloadError> {
    let original = ParsedUrl::parse(original_url)?;
    if original.secure && !final_url.starts_with("https://") {
        return Err(DownloadError::InsecureRedirect);
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, DownloadError> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; READ_BUFFER_SIZE];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:X}", hasher.finalize()))
}

fn remove_cache_file_if_present(path: &Path) -> Result<(), DownloadError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn promote_verified(part_path: &Path, complete_path: &Path) -> Result<(), DownloadError> {
    remove_cache_file_if_present(complete_path)?;
    fs::rename(part_path, complete_path)?;
    Ok(())
}

#[cfg(not(windows))]
fn download_once(
    _artifact: &RuntimeArtifact,
    _part_path: &Path,
    _progress: &ProgressBook,
    _cancelled: &AtomicBool,
    _network_bytes: &mut u64,
) -> Result<(), DownloadError> {
    Err(DownloadError::UnsupportedPlatform)
}

#[cfg(windows)]
fn download_once(
    artifact: &RuntimeArtifact,
    part_path: &Path,
    progress: &ProgressBook,
    cancelled: &AtomicBool,
    network_bytes: &mut u64,
) -> Result<(), DownloadError> {
    use std::ffi::c_void;
    use std::ptr;

    use windows_sys::Win32::Networking::WinHttp::{
        HTTP_STATUS_OK, HTTP_STATUS_PARTIAL_CONTENT, WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
        WINHTTP_ADDREQ_FLAG_ADD, WINHTTP_ADDREQ_FLAG_REPLACE, WINHTTP_FLAG_SECURE,
        WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP,
        WINHTTP_OPTION_URL, WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_QUERY_CONTENT_RANGE,
        WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_FLAG_NUMBER64, WINHTTP_QUERY_STATUS_CODE,
        WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen,
        WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpQueryOption, WinHttpReadData,
        WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
    };

    struct InternetHandle(*mut c_void);
    impl Drop for InternetHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: The handle was returned by WinHTTP and is owned exactly once here.
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    let parsed = ParsedUrl::parse(&artifact.url)?;
    let agent = wide("SafeDuplicateFinderSetup/0.2");
    // SAFETY: All pointers are valid null-terminated UTF-16 for the duration of each call.
    let session = unsafe {
        InternetHandle(WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        ))
    };
    ensure_handle(&session, "WinHttpOpen")?;
    // SAFETY: Valid session handle and scalar timeout values.
    if unsafe { WinHttpSetTimeouts(session.0, 20_000, 20_000, 30_000, 30_000) } == 0 {
        return Err(winhttp_error("WinHttpSetTimeouts"));
    }

    let host = wide(&parsed.host);
    // SAFETY: Valid session and host buffer.
    let connect =
        unsafe { InternetHandle(WinHttpConnect(session.0, host.as_ptr(), parsed.port, 0)) };
    ensure_handle(&connect, "WinHttpConnect")?;

    let verb = wide("GET");
    let object = wide(&parsed.object);
    let flags = if parsed.secure {
        WINHTTP_FLAG_SECURE
    } else {
        0
    };
    // SAFETY: Valid handles/buffers; null optional strings and accept list are documented.
    let request = unsafe {
        InternetHandle(WinHttpOpenRequest(
            connect.0,
            verb.as_ptr(),
            object.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            flags,
        ))
    };
    ensure_handle(&request, "WinHttpOpenRequest")?;
    let redirect_policy = WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP;
    // SAFETY: request is valid; option points to a live u32 with the documented size.
    if unsafe {
        WinHttpSetOption(
            request.0,
            WINHTTP_OPTION_REDIRECT_POLICY,
            (&raw const redirect_policy).cast(),
            u32::try_from(std::mem::size_of::<u32>()).unwrap_or(4),
        )
    } == 0
    {
        return Err(winhttp_error("WinHttpSetOption(redirect policy)"));
    }

    let resume_offset = fs::metadata(part_path).map_or(0, |metadata| metadata.len());
    if resume_offset > 0 {
        let range = wide(&format!("Range: bytes={resume_offset}-\r\n"));
        // SAFETY: request and null-terminated header buffer are valid.
        if unsafe {
            WinHttpAddRequestHeaders(
                request.0,
                range.as_ptr(),
                u32::MAX,
                WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
            )
        } == 0
        {
            return Err(winhttp_error("WinHttpAddRequestHeaders(Range)"));
        }
    }

    // SAFETY: request is valid and this GET has no optional body.
    if unsafe { WinHttpSendRequest(request.0, ptr::null(), 0, ptr::null(), 0, 0, 0) } == 0 {
        return Err(winhttp_error("WinHttpSendRequest"));
    }
    // SAFETY: request is valid and reserved pointer is null.
    if unsafe { WinHttpReceiveResponse(request.0, ptr::null_mut()) } == 0 {
        return Err(winhttp_error("WinHttpReceiveResponse"));
    }

    if parsed.secure {
        let final_url = query_option_string(request.0, WINHTTP_OPTION_URL)?;
        validate_final_redirect_scheme(&artifact.url, &final_url)?;
    }

    let status = query_header_u32(
        request.0,
        WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
    )?;
    let mut append = false;
    let expected_response_bytes;
    match (resume_offset, status) {
        (0, HTTP_STATUS_OK) => {
            expected_response_bytes = artifact.size_bytes;
        }
        (offset, HTTP_STATUS_PARTIAL_CONTENT) => {
            let range = query_header_string(request.0, WINHTTP_QUERY_CONTENT_RANGE)?
                .ok_or(DownloadError::InvalidContentRange)?;
            validate_content_range(&range, offset, artifact.size_bytes)?;
            expected_response_bytes = artifact.size_bytes.saturating_sub(offset);
            append = offset > 0;
        }
        (offset, HTTP_STATUS_OK) if offset > 0 => {
            expected_response_bytes = artifact.size_bytes;
            progress.set_existing_bytes(&artifact.id, 0)?;
        }
        (_, other) => return Err(DownloadError::HttpStatus(other)),
    }
    let response_length = query_header_u64(
        request.0,
        WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER64,
    )?;
    if response_length != expected_response_bytes {
        return Err(DownloadError::LengthMismatch {
            expected: expected_response_bytes,
            actual: response_length,
        });
    }

    let mut options = OpenOptions::new();
    options.create(true).write(true);
    if append {
        options.append(true);
    } else {
        options.truncate(true);
    }
    let mut output = options.open(part_path)?;
    let mut buffer = [0_u8; READ_BUFFER_SIZE];
    loop {
        if cancelled.load(Ordering::Acquire) {
            output.flush()?;
            return Err(DownloadError::Cancelled);
        }
        let mut read = 0_u32;
        // SAFETY: request is valid and buffer/read pointers remain live for this synchronous call.
        if unsafe {
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                u32::try_from(buffer.len()).unwrap_or(65_536),
                &raw mut read,
            )
        } == 0
        {
            return Err(winhttp_error("WinHttpReadData"));
        }
        if read == 0 {
            break;
        }
        let count = usize::try_from(read).unwrap_or(buffer.len());
        output.write_all(&buffer[..count])?;
        let delta = u64::from(read);
        *network_bytes = network_bytes
            .checked_add(delta)
            .ok_or(ProgressError::CounterOverflow)?;
        progress.record_network_bytes_now(&artifact.id, delta)?;
    }
    output.sync_all()?;

    let actual = fs::metadata(part_path)?.len();
    if actual != artifact.size_bytes {
        return Err(DownloadError::LengthMismatch {
            expected: artifact.size_bytes,
            actual,
        });
    }
    return Ok(());

    fn ensure_handle(
        handle: &InternetHandle,
        operation: &'static str,
    ) -> Result<(), DownloadError> {
        if handle.0.is_null() {
            Err(winhttp_error(operation))
        } else {
            Ok(())
        }
    }

    fn query_header_u32(handle: *mut c_void, query: u32) -> Result<u32, DownloadError> {
        let mut value = 0_u32;
        let mut length = u32::try_from(std::mem::size_of::<u32>()).unwrap_or(4);
        // SAFETY: output points to a live u32 and the supplied size is exact.
        if unsafe {
            WinHttpQueryHeaders(
                handle,
                query,
                ptr::null(),
                (&raw mut value).cast(),
                &raw mut length,
                ptr::null_mut(),
            )
        } == 0
        {
            Err(winhttp_error("WinHttpQueryHeaders(u32)"))
        } else {
            Ok(value)
        }
    }

    fn query_header_u64(handle: *mut c_void, query: u32) -> Result<u64, DownloadError> {
        let mut value = 0_u64;
        let mut length = u32::try_from(std::mem::size_of::<u64>()).unwrap_or(8);
        // SAFETY: output points to a live u64 and the supplied size is exact.
        if unsafe {
            WinHttpQueryHeaders(
                handle,
                query,
                ptr::null(),
                (&raw mut value).cast(),
                &raw mut length,
                ptr::null_mut(),
            )
        } == 0
        {
            Err(winhttp_error("WinHttpQueryHeaders(u64)"))
        } else {
            Ok(value)
        }
    }

    fn query_header_string(
        handle: *mut c_void,
        query: u32,
    ) -> Result<Option<String>, DownloadError> {
        query_wide_string(|buffer, length| {
            // SAFETY: WinHTTP receives either a null sizing pointer or a live UTF-16 buffer.
            unsafe {
                WinHttpQueryHeaders(handle, query, ptr::null(), buffer, length, ptr::null_mut())
            }
        })
    }

    fn query_option_string(handle: *mut c_void, option: u32) -> Result<String, DownloadError> {
        query_wide_string(|buffer, length| {
            // SAFETY: WinHTTP receives either a null sizing pointer or a live UTF-16 buffer.
            unsafe { WinHttpQueryOption(handle, option, buffer, length) }
        })?
        .ok_or_else(|| winhttp_error("WinHttpQueryOption(URL)"))
    }

    fn query_wide_string<F>(mut query: F) -> Result<Option<String>, DownloadError>
    where
        F: FnMut(*mut c_void, *mut u32) -> i32,
    {
        const ERROR_INSUFFICIENT_BUFFER: i32 = 122;
        const ERROR_WINHTTP_HEADER_NOT_FOUND: i32 = 12_150;
        let mut length = 0_u32;
        if query(ptr::null_mut(), &raw mut length) == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_WINHTTP_HEADER_NOT_FOUND) {
                return Ok(None);
            }
            if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER) {
                return Err(DownloadError::WinHttp {
                    operation: "query string size",
                    source: error,
                });
            }
        }
        let units = usize::try_from(length)
            .unwrap_or(0)
            .div_ceil(std::mem::size_of::<u16>())
            .max(1);
        let mut buffer = vec![0_u16; units];
        if query(buffer.as_mut_ptr().cast(), &raw mut length) == 0 {
            return Err(winhttp_error("query string"));
        }
        let end = buffer
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(buffer.len());
        Ok(Some(String::from_utf16_lossy(&buffer[..end])))
    }

    fn winhttp_error(operation: &'static str) -> DownloadError {
        DownloadError::WinHttp {
            operation,
            source: std::io::Error::last_os_error(),
        }
    }
}

#[derive(Debug)]
struct ParsedUrl {
    secure: bool,
    host: String,
    port: u16,
    object: String,
}

impl ParsedUrl {
    fn parse(url: &str) -> Result<Self, DownloadError> {
        let (secure, rest, default_port) = if let Some(rest) = url.strip_prefix("https://") {
            (true, rest, 443)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (false, rest, 80)
        } else {
            return Err(DownloadError::InvalidUrl(url.to_owned()));
        };
        let (authority, object) = rest
            .split_once('/')
            .map_or((rest, "/".to_owned()), |(authority, path)| {
                (authority, format!("/{path}"))
            });
        if authority.is_empty() || authority.contains('@') || authority.contains('[') {
            return Err(DownloadError::InvalidUrl(url.to_owned()));
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() => {
                let port = port
                    .parse::<u16>()
                    .map_err(|_| DownloadError::InvalidUrl(url.to_owned()))?;
                (host, port)
            }
            _ => (authority, default_port),
        };
        if host.is_empty() || object.contains('#') {
            return Err(DownloadError::InvalidUrl(url.to_owned()));
        }
        Ok(Self {
            secure,
            host: host.to_owned(),
            port,
            object,
        })
    }
}

fn validate_content_range(
    value: &str,
    expected_start: u64,
    expected_total: u64,
) -> Result<(), DownloadError> {
    let range = value
        .strip_prefix("bytes ")
        .ok_or(DownloadError::InvalidContentRange)?;
    let (bounds, total) = range
        .split_once('/')
        .ok_or(DownloadError::InvalidContentRange)?;
    let (start, end) = bounds
        .split_once('-')
        .ok_or(DownloadError::InvalidContentRange)?;
    let start = start
        .parse::<u64>()
        .map_err(|_| DownloadError::InvalidContentRange)?;
    let end = end
        .parse::<u64>()
        .map_err(|_| DownloadError::InvalidContentRange)?;
    let total = total
        .parse::<u64>()
        .map_err(|_| DownloadError::InvalidContentRange)?;
    if start != expected_start
        || total != expected_total
        || end.checked_add(1) != Some(expected_total)
        || end < start
    {
        return Err(DownloadError::InvalidContentRange);
    }
    Ok(())
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
