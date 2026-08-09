//! Verified runtime process execution.

use std::path::Path;
use std::process::Command;

use thiserror::Error;

/// Runtime installer process failure.
#[derive(Debug, Error)]
pub enum InstallError {
    /// The verified executable could not be started or waited for.
    #[error("không thể chạy Runtime đã xác minh: {0}")]
    Io(#[from] std::io::Error),
    /// Runtime setup reported failure.
    #[error("Runtime installer trả mã lỗi {0}")]
    ExitCode(i32),
    /// Windows process ended without an exit code.
    #[error("Runtime installer kết thúc không có mã thoát")]
    MissingExitCode,
}

/// Execute a previously size/SHA-256-verified runtime installer using an argument vector.
pub fn run_runtime_installer(path: &Path, args: &[String]) -> Result<i32, InstallError> {
    let mut command = Command::new(path);
    command.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let status = command.status()?;
    let code = status.code().ok_or(InstallError::MissingExitCode)?;
    if is_success_exit_code(code) {
        Ok(code)
    } else {
        Err(InstallError::ExitCode(code))
    }
}

/// Windows Installer convention: success or success-with-reboot-required.
#[must_use]
pub const fn is_success_exit_code(code: i32) -> bool {
    matches!(code, 0 | 3010)
}
