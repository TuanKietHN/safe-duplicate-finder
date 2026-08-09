use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use safe_dedupe_runtime_installer::download::{DownloadError, DownloadOutcome, download_artifact};
use safe_dedupe_runtime_installer::install::{InstallError, run_runtime_installer};
use safe_dedupe_runtime_installer::manifest::{RuntimeArtifact, parse_manifest};
use safe_dedupe_runtime_installer::preflight::{
    PreflightStatus, preflight_artifact, runtime_is_installed,
};
use safe_dedupe_runtime_installer::progress::{ItemState, ProgressBook};
use safe_dedupe_runtime_installer::scheduler::run_bounded;
use safe_dedupe_runtime_installer::ui::{UiState, show_progress_dialog};
use thiserror::Error;

const EMBEDDED_MANIFEST: &str = include_str!("../../../installer/runtime-manifest.json");

fn main() {
    let manifest = match parse_manifest(EMBEDDED_MANIFEST) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("Manifest Runtime không hợp lệ: {error}");
            std::process::exit(2);
        }
    };
    let cache_dir = match product_cache_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Không xác định được thư mục cache: {error}");
            std::process::exit(20);
        }
    };
    let progress = match ProgressBook::new(
        manifest
            .artifacts
            .iter()
            .map(|artifact| (artifact.id.as_str(), artifact.size_bytes)),
    ) {
        Ok(progress) => Arc::new(progress),
        Err(error) => {
            eprintln!("Không thể khởi tạo bộ đếm Runtime: {error}");
            std::process::exit(2);
        }
    };
    let names = manifest
        .artifacts
        .iter()
        .map(|artifact| (artifact.id.clone(), artifact.display_name.clone()));
    let ui = Arc::new(UiState::new(Arc::clone(&progress), names));
    let worker_ui = Arc::clone(&ui);
    let log_path = cache_dir
        .parent()
        .unwrap_or(&cache_dir)
        .join("installer.log");
    let worker = std::thread::spawn(move || {
        let result = run_session(
            manifest.artifacts,
            &cache_dir,
            &log_path,
            &progress,
            &worker_ui,
        );
        match result {
            Ok(()) => {
                worker_ui.finish(0, "Đã chuẩn bị xong Runtime. Đang tiếp tục cài ứng dụng...")
            }
            Err(error) => {
                append_log(
                    &log_path,
                    &format!("failed code={} error={error}", error.exit_code()),
                );
                worker_ui.finish(
                    error.exit_code(),
                    format!("Không thể chuẩn bị Runtime: {error}"),
                );
            }
        }
    });

    if let Err(error) = show_progress_dialog(&ui) {
        eprintln!("Không thể mở giao diện tiến độ native: {error}");
    }
    if worker.join().is_err() {
        eprintln!("Worker Runtime kết thúc bất thường");
        std::process::exit(20);
    }
    std::process::exit(ui.exit_code());
}

fn run_session(
    artifacts: Vec<RuntimeArtifact>,
    cache_dir: &Path,
    log_path: &Path,
    progress: &Arc<ProgressBook>,
    ui: &Arc<UiState>,
) -> Result<(), SessionError> {
    fs::create_dir_all(cache_dir)?;
    append_log(log_path, "session_start schema=1");
    let mut installed = BTreeMap::new();
    let mut to_download = Vec::new();

    for artifact in &artifacts {
        progress.set_state(&artifact.id, ItemState::Prechecking)?;
        ui.set_status(format!("Đang kiểm tra {}...", artifact.display_name));
        let status = preflight_artifact(artifact, cache_dir)?;
        append_log(
            log_path,
            &format!("preflight id={} status={status:?}", artifact.id),
        );
        match status {
            PreflightStatus::InstalledValid => {
                progress.set_required(&artifact.id, false)?;
                progress.set_existing_bytes(&artifact.id, artifact.size_bytes)?;
                progress.set_state(&artifact.id, ItemState::InstalledValid)?;
                progress.set_message(&artifact.id, "đã có trên máy; không tải lại")?;
                installed.insert(artifact.id.clone(), true);
            }
            PreflightStatus::CacheValid => {
                progress.set_existing_bytes(&artifact.id, artifact.size_bytes)?;
                progress.set_state(&artifact.id, ItemState::CacheValid)?;
                progress.set_message(&artifact.id, "cache đúng kích thước và SHA-256")?;
                installed.insert(artifact.id.clone(), false);
                to_download.push(artifact.clone());
            }
            PreflightStatus::NeedsDownload { resume_offset } => {
                progress.set_existing_bytes(&artifact.id, resume_offset)?;
                progress.set_message(
                    &artifact.id,
                    if resume_offset == 0 {
                        "cần tải mới".to_owned()
                    } else {
                        format!("tiếp tục từ byte {resume_offset}")
                    },
                )?;
                installed.insert(artifact.id.clone(), false);
                to_download.push(artifact.clone());
            }
        }
    }

    if ui.cancellation().load(Ordering::Acquire) {
        return Err(SessionError::Cancelled);
    }
    let mut outcomes = BTreeMap::<String, DownloadOutcome>::new();
    if !to_download.is_empty() {
        ui.set_status("Đang tải các Runtime còn thiếu bằng dữ liệu thực nhận...");
        let cache_dir = cache_dir.to_path_buf();
        let progress_for_work = Arc::clone(progress);
        let ui_for_work = Arc::clone(ui);
        let results = run_bounded(to_download, 2, move |artifact| {
            let result = download_artifact(
                &artifact,
                &cache_dir,
                &progress_for_work,
                ui_for_work.cancellation(),
            );
            (artifact.id, result)
        })?;
        for (id, result) in results {
            let outcome = result?;
            append_log(
                log_path,
                &format!(
                    "download id={id} network_bytes={} reused_cache={} digest=ok",
                    outcome.network_bytes, outcome.reused_cache
                ),
            );
            outcomes.insert(id, outcome);
        }
    }

    for artifact in &artifacts {
        if installed.get(&artifact.id).copied().unwrap_or(false) {
            continue;
        }
        if ui.cancellation().load(Ordering::Acquire) {
            return Err(SessionError::Cancelled);
        }
        let outcome = outcomes
            .get(&artifact.id)
            .ok_or_else(|| SessionError::MissingOutcome(artifact.id.clone()))?;
        progress.set_state(&artifact.id, ItemState::Installing)?;
        progress.set_message(&artifact.id, "đang chạy trình cài đặt đã xác minh")?;
        ui.set_status(format!("Đang cài {}...", artifact.display_name));
        let code = run_runtime_installer(&outcome.complete_path, &artifact.install_args)?;
        append_log(
            log_path,
            &format!("install id={} exit_code={code}", artifact.id),
        );
        if !runtime_is_installed(artifact)? {
            return Err(SessionError::RuntimeNotDetected(artifact.id.clone()));
        }
        progress.set_state(&artifact.id, ItemState::Completed)?;
        progress.set_message(&artifact.id, "đã cài và kiểm tra lại")?;
    }
    append_log(log_path, "session_complete result=success");
    Ok(())
}

#[derive(Debug, Error)]
enum SessionError {
    #[error("người dùng đã hủy")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Progress(#[from] safe_dedupe_runtime_installer::progress::ProgressError),
    #[error(transparent)]
    Preflight(#[from] safe_dedupe_runtime_installer::preflight::PreflightError),
    #[error(transparent)]
    Download(#[from] DownloadError),
    #[error(transparent)]
    Install(#[from] InstallError),
    #[error(transparent)]
    Scheduler(#[from] safe_dedupe_runtime_installer::scheduler::SchedulerError),
    #[error("thiếu kết quả tải đã xác minh cho {0}")]
    MissingOutcome(String),
    #[error("Runtime cài xong nhưng không phát hiện lại được: {0}")]
    RuntimeNotDetected(String),
}

impl SessionError {
    const fn exit_code(&self) -> i32 {
        match self {
            Self::Cancelled | Self::Download(DownloadError::Cancelled) => 4,
            Self::Download(
                DownloadError::LengthMismatch { .. } | DownloadError::DigestMismatch { .. },
            ) => 6,
            Self::Download(_) => 5,
            Self::Install(_) | Self::RuntimeNotDetected(_) => 7,
            Self::Preflight(_) => 3,
            Self::MissingOutcome(_) | Self::Io(_) | Self::Progress(_) | Self::Scheduler(_) => 20,
        }
    }
}

fn product_cache_dir() -> Result<PathBuf, std::io::Error> {
    let local = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "LOCALAPPDATA chưa được đặt")
    })?;
    Ok(PathBuf::from(local)
        .join("io.github.safeduplicate.finder")
        .join("installer-cache"))
}

fn append_log(path: &Path, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let _ = writeln!(file, "{now} {message}");
        let _ = file.flush();
    }
}
