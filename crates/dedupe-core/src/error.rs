//! Error types that preserve safety context without exposing document contents.

use std::path::PathBuf;

/// Project-wide result type.
pub type Result<T> = std::result::Result<T, DedupeError>;

/// Failures are categorized so one bad file can be isolated while durability failures stop mutation.
#[derive(Debug, thiserror::Error)]
pub enum DedupeError {
    /// A requested operation violated a non-negotiable safety precondition.
    #[error("Điều kiện an toàn không đạt: {0}")]
    Safety(String),
    /// The requested operation was cancelled at a safe boundary.
    #[error("Thao tác đã bị hủy")]
    Cancelled,
    /// A path could not be processed.
    #[error("Lỗi I/O khi {operation} đối với {path}: {source}")]
    Io {
        /// Operation being performed.
        operation: &'static str,
        /// Affected path.
        path: PathBuf,
        /// Operating-system error.
        #[source]
        source: std::io::Error,
    },
    /// Durable storage could not commit required evidence.
    #[error("Lỗi lưu trữ bền vững: {0}")]
    Durability(String),
    /// Stored state was invalid or inconsistent.
    #[error("Xung đột trạng thái: {0}")]
    State(String),
    /// User or configuration input was invalid.
    #[error("Dữ liệu đầu vào không hợp lệ: {0}")]
    InvalidInput(String),
    /// Serialization failed.
    #[error("Lỗi tuần tự hóa: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl DedupeError {
    /// Construct a path-aware I/O error without reading or logging file contents.
    #[must_use]
    pub fn io(operation: &'static str, path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            operation,
            path: path.into(),
            source,
        }
    }

    /// Whether retrying later could reasonably succeed.
    #[must_use]
    pub fn retryable(&self) -> bool {
        match self {
            Self::Io { source, .. } => matches!(
                source.kind(),
                std::io::ErrorKind::Interrupted
                    | std::io::ErrorKind::WouldBlock
                    | std::io::ErrorKind::TimedOut
            ),
            Self::Durability(_) | Self::State(_) => true,
            Self::Safety(_) | Self::Cancelled | Self::InvalidInput(_) | Self::Serialization(_) => {
                false
            }
        }
    }
}
