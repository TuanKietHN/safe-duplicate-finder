//! Native Win32 installer progress UI.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use crate::progress::{ItemState, ProgressBook, ProgressSnapshot};

/// Shared model consumed by the native dialog callback.
#[derive(Debug)]
pub struct UiState {
    progress: Arc<ProgressBook>,
    names: BTreeMap<String, String>,
    status: Mutex<String>,
    cancelled: AtomicBool,
    finished: AtomicBool,
    exit_code: AtomicI32,
}

impl UiState {
    /// Create UI state for one fixed manifest.
    #[must_use]
    pub fn new(
        progress: Arc<ProgressBook>,
        names: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        Self {
            progress,
            names: names.into_iter().collect(),
            status: Mutex::new("Đang kiểm tra các Runtime cần thiết...".into()),
            cancelled: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            exit_code: AtomicI32::new(20),
        }
    }

    /// Cancellation flag checked at bounded read/install boundaries.
    #[must_use]
    pub const fn cancellation(&self) -> &AtomicBool {
        &self.cancelled
    }

    /// Replace the current real operation label.
    pub fn set_status(&self, status: impl Into<String>) {
        *self
            .status
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = status.into();
    }

    /// Complete the worker and cause the dialog to close on its next timer callback.
    pub fn finish(&self, exit_code: i32, status: impl Into<String>) {
        self.set_status(status);
        self.exit_code.store(exit_code, Ordering::Release);
        self.finished.store(true, Ordering::Release);
    }

    /// Final worker exit code.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        self.exit_code.load(Ordering::Acquire)
    }

    /// Render current text from real byte snapshots; useful for UI tests and diagnostics.
    #[must_use]
    pub fn content_text(&self) -> String {
        let snapshot = self.progress.snapshot(self.progress.elapsed_ms());
        let status = self
            .status
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        format_snapshot(&snapshot, &status, &self.names)
    }
}

/// Show the modal native setup progress dialog until worker completion/cancellation.
#[cfg(windows)]
pub fn show_progress_dialog(state: &Arc<UiState>) -> Result<(), String> {
    use std::ptr;

    use windows_sys::Win32::UI::Controls::{
        TASKDIALOGCONFIG, TDCBF_CANCEL_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_CALLBACK_TIMER,
        TDF_CAN_BE_MINIMIZED, TDF_SHOW_PROGRESS_BAR, TDF_SIZE_TO_CONTENT, TaskDialogIndirect,
    };

    let title = wide("Cài đặt Trình tìm tệp trùng lặp an toàn");
    let instruction = wide("Đang chuẩn bị các thành phần cần thiết");
    let content = wide(&state.content_text());
    let config = TASKDIALOGCONFIG {
        cbSize: u32::try_from(std::mem::size_of::<TASKDIALOGCONFIG>()).unwrap_or(u32::MAX),
        hwndParent: ptr::null_mut(),
        hInstance: ptr::null_mut(),
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION
            | TDF_CALLBACK_TIMER
            | TDF_CAN_BE_MINIMIZED
            | TDF_SHOW_PROGRESS_BAR
            | TDF_SIZE_TO_CONTENT,
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: title.as_ptr(),
        pszMainInstruction: instruction.as_ptr(),
        pszContent: content.as_ptr(),
        pfCallback: Some(task_dialog_callback),
        lpCallbackData: Arc::as_ptr(state) as isize,
        cxWidth: 520,
        ..TASKDIALOGCONFIG::default()
    };
    let mut button = 0_i32;
    // SAFETY: config and its UTF-16 buffers remain live until this synchronous modal call returns.
    let result = unsafe {
        TaskDialogIndirect(
            &raw const config,
            &raw mut button,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if result < 0 {
        Err(format!(
            "TaskDialogIndirect HRESULT=0x{:08X}",
            result as u32
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
pub fn show_progress_dialog(state: &Arc<UiState>) -> Result<(), String> {
    while !state.finished.load(Ordering::Acquire) {
        eprintln!("{}", state.content_text());
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Ok(())
}

#[cfg(windows)]
unsafe extern "system" fn task_dialog_callback(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: windows_sys::Win32::Foundation::WPARAM,
    _lparam: windows_sys::Win32::Foundation::LPARAM,
    reference_data: isize,
) -> windows_sys::core::HRESULT {
    use windows_sys::Win32::UI::Controls::{
        TDE_CONTENT, TDM_CLICK_BUTTON, TDM_SET_ELEMENT_TEXT, TDM_SET_PROGRESS_BAR_POS,
        TDN_BUTTON_CLICKED, TDN_CREATED, TDN_TIMER,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDCANCEL, SendMessageW};

    // SAFETY: reference_data is Arc::as_ptr(state), and the Arc outlives TaskDialogIndirect.
    let state = unsafe { &*(reference_data as *const UiState) };
    if message == TDN_CREATED as u32 || message == TDN_TIMER as u32 {
        let snapshot = state.progress.snapshot(state.progress.elapsed_ms());
        let status = state
            .status
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let content = wide(&format_snapshot(&snapshot, &status, &state.names));
        // SAFETY: SendMessageW is synchronous; content remains live through the call.
        unsafe {
            SendMessageW(
                hwnd,
                TDM_SET_PROGRESS_BAR_POS as u32,
                usize::from(snapshot.overall_basis_points / 100),
                0,
            );
            SendMessageW(
                hwnd,
                TDM_SET_ELEMENT_TEXT as u32,
                usize::try_from(TDE_CONTENT).unwrap_or_default(),
                content.as_ptr() as isize,
            );
        }
        if state.finished.load(Ordering::Acquire) {
            // SAFETY: Clicking the existing common Cancel button closes the modal dialog.
            unsafe {
                SendMessageW(
                    hwnd,
                    TDM_CLICK_BUTTON as u32,
                    usize::try_from(IDCANCEL).unwrap_or(2),
                    0,
                );
            }
        }
        return 0;
    }
    if message == TDN_BUTTON_CLICKED as u32 && i32::try_from(wparam).unwrap_or_default() == IDCANCEL
    {
        if state.finished.load(Ordering::Acquire) {
            return 0;
        }
        state.cancelled.store(true, Ordering::Release);
        state.set_status("Đang dừng an toàn sau khối dữ liệu hiện tại...");
        return 1;
    }
    0
}

fn format_snapshot(
    snapshot: &ProgressSnapshot,
    status: &str,
    names: &BTreeMap<String, String>,
) -> String {
    let percent = f64::from(snapshot.overall_basis_points) / 100.0;
    let speed = if snapshot.bytes_per_second == 0 {
        "đang đo".to_owned()
    } else {
        format!("{}/giây", format_bytes(snapshot.bytes_per_second))
    };
    let eta = snapshot
        .eta_seconds
        .map_or_else(|| "đang tính".to_owned(), format_eta);
    let current = snapshot
        .items
        .iter()
        .find(|item| {
            matches!(
                item.state,
                ItemState::Downloading | ItemState::Verifying | ItemState::Installing
            )
        })
        .map_or_else(
            || "Không có".to_owned(),
            |item| {
                names
                    .get(&item.id)
                    .cloned()
                    .unwrap_or_else(|| item.id.clone())
            },
        );
    let mut lines = vec![
        status.to_owned(),
        String::new(),
        format!(
            "Tổng đã tải: {} / {}",
            format_bytes(snapshot.received_bytes),
            format_bytes(snapshot.required_download_bytes)
        ),
        format!("Tốc độ: {speed}    •    Còn lại: {eta}"),
        format!("Tiến độ tổng: {percent:.1}%"),
        format!("Tệp hiện tại: {current}"),
        String::new(),
        "Trạng thái toàn bộ Runtime:".to_owned(),
    ];
    for item in &snapshot.items {
        let name = names.get(&item.id).unwrap_or(&item.id);
        let state = state_label(item.state);
        let suffix = if item.message.is_empty() {
            String::new()
        } else {
            format!(" — {}", item.message)
        };
        lines.push(format!(
            "• {name}: {state} ({} / {}){suffix}",
            format_bytes(item.received_bytes),
            format_bytes(item.size_bytes)
        ));
    }
    lines.join("\r\n")
}

fn state_label(state: ItemState) -> &'static str {
    match state {
        ItemState::Pending => "Chờ kiểm tra",
        ItemState::Prechecking => "Đang kiểm tra",
        ItemState::InstalledValid => "Đã cài hợp lệ — bỏ qua tải",
        ItemState::CacheValid => "Tệp tải đã xác minh",
        ItemState::Downloading => "Đang tải",
        ItemState::Verifying => "Đang xác minh SHA-256",
        ItemState::Installing => "Đang cài Runtime",
        ItemState::Completed => "Hoàn tất",
        ItemState::Cancelled => "Đã hủy",
        ItemState::Failed => "Lỗi",
    }
}

fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1_048_576.0;
    if bytes < 1_048_576 {
        format!("{:.1} KiB", bytes as f64 / 1_024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB)
    }
}

fn format_eta(seconds: u64) -> String {
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    if minutes == 0 {
        format!("{seconds} giây")
    } else {
        format!("{minutes} phút {seconds} giây")
    }
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
