//! Streaming scan sink and bounded preliminary-group reader.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use chrono::Utc;
use dedupe_core::{
    DedupeError, Result,
    control::ControlToken,
    model::{AccessStatus, ComparisonMode, FileIdentity, FileMetadataSnapshot, LinkKind},
    path_normalization::{path_identity_key, path_key},
    ports::ScanSink,
    progress::ProgressCounters,
};
use parking_lot::Mutex;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Database;

/// Scan/session repository.
#[derive(Debug, Clone)]
pub struct ScanRepository {
    database: Database,
}

/// Persisted scan state returned to CLI/desktop polling.
#[derive(Debug, Clone, Serialize)]
pub struct ScanSessionRecord {
    /// Session identifier.
    pub id: Uuid,
    /// Current durable state.
    pub state: String,
    /// Discovered files.
    pub discovered_files: u64,
    /// Processed files.
    pub processed_files: u64,
    /// Bytes read.
    pub bytes_read: u64,
    /// Isolated errors.
    pub errors: u64,
    /// Files skipped by configured filters or safety rules.
    pub skipped: u64,
    /// Files observed changing while evidence was read.
    pub unstable: u64,
    /// Proven duplicate groups.
    pub duplicate_groups: u64,
    /// Safely reclaimable bytes.
    pub reclaimable_bytes: u64,
    /// Durable scan start timestamp.
    pub started_at: Option<String>,
    /// Durable completion timestamp, when final.
    pub finished_at: Option<String>,
    /// Persisted root cause when a worker cannot continue safely.
    pub blocked_reason: Option<String>,
}

/// Durable information needed to restart a read-only scan after application termination.
#[derive(Debug, Clone, Copy)]
pub struct ScanResumeSpec {
    /// Existing session identifier reused for the retry.
    pub session_id: Uuid,
    /// Owning project.
    pub project_id: Uuid,
    /// Comparison mode captured when the session was created.
    pub mode: ComparisonMode,
    /// Whether every extension was included.
    pub all_files: bool,
}

/// Durable control request observed by an in-process scan worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanControlRequest {
    /// No request is pending.
    None,
    /// Stop at the next cooperative boundary.
    Pause,
    /// Continue a cooperatively paused scan.
    Resume,
    /// Cancel at the next cooperative boundary.
    Cancel,
}

/// Polls durable cross-process control requests while one scan worker owns the session.
pub struct ScanControlMonitor {
    stop: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ScanControlMonitor {
    /// Start a bounded polling loop. It never performs source-file I/O.
    #[must_use]
    pub fn start(
        database: Database,
        session_id: Uuid,
        control: ControlToken,
        progress: Arc<ProgressCounters>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let worker_stop = Arc::clone(&stop);
        let worker_error = Arc::clone(&error);
        let worker = thread::spawn(move || {
            let repository = ScanRepository::new(database);
            while !worker_stop.load(Ordering::Acquire) {
                if let Err(failure) = repository.update_progress(session_id, progress.snapshot()) {
                    *worker_error.lock() = Some(failure.to_string());
                    control.cancel();
                    break;
                }
                let request = match repository.pending_control(session_id) {
                    Ok(request) => request,
                    Err(failure) => {
                        *worker_error.lock() = Some(failure.to_string());
                        control.cancel();
                        break;
                    }
                };
                let result = match request {
                    ScanControlRequest::None => Ok(()),
                    ScanControlRequest::Pause => {
                        control.pause();
                        repository.acknowledge_control(session_id, ScanControlRequest::Pause)
                    }
                    ScanControlRequest::Resume => {
                        control.resume();
                        repository.acknowledge_control(session_id, ScanControlRequest::Resume)
                    }
                    ScanControlRequest::Cancel => {
                        control.cancel();
                        break;
                    }
                };
                if let Err(failure) = result {
                    *worker_error.lock() = Some(failure.to_string());
                    control.cancel();
                    break;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        Self {
            stop,
            error,
            worker: Some(worker),
        }
    }

    /// Stop polling, persist one final snapshot in the caller, and surface monitor failures.
    pub fn finish(mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| DedupeError::State("Bộ giám sát điều khiển quét gặp panic".into()))?;
        }
        if let Some(error) = self.error.lock().take() {
            return Err(DedupeError::Durability(format!(
                "Bộ giám sát điều khiển quét thất bại: {error}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct StoredScanConfig {
    #[serde(default)]
    all_files: bool,
}

impl ScanRepository {
    /// Bind repository.
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// Create a durable scan session before enumeration.
    pub fn create_session(&self, project_id: Uuid, mode: ComparisonMode) -> Result<Uuid> {
        self.create_session_with_config(project_id, mode, false)
    }

    /// Create a durable session and retain the options required for an explicit restart.
    pub fn create_session_with_config(
        &self,
        project_id: Uuid,
        mode: ComparisonMode,
        all_files: bool,
    ) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        let config = serde_json::to_string(&StoredScanConfig { all_files })?;
        self.database
            .connection()
            .execute(
                "INSERT INTO scan_sessions (
                    id,project_id,mode,state,started_at,config_json,created_at,updated_at
                ) VALUES (?1,?2,?3,'enumerating',?4,?5,?4,?4)",
                params![
                    id.to_string(),
                    project_id.to_string(),
                    mode_name(mode),
                    now,
                    config
                ],
            )
            .map_err(store_error)?;
        Ok(id)
    }

    /// Convert active sessions left without an in-process worker into an explicit recoverable state.
    pub fn mark_incomplete_interrupted(&self) -> Result<u64> {
        let changed = self
            .database
            .connection()
            .execute(
                "UPDATE scan_sessions SET state='interrupted',control_request='none',
                    resume_state=NULL,updated_at=?1
                 WHERE state IN (
                    'enumerating','quick_hashing','blake3_hashing','sha256_hashing','grouping',
                    'pausing','paused','cancelling','recovering'
                 )",
                [Utc::now().to_rfc3339()],
            )
            .map_err(store_error)?;
        u64::try_from(changed)
            .map_err(|_| DedupeError::State("Số phiên bị gián đoạn bị tràn".into()))
    }

    /// Load the immutable scan options for an interrupted or blocked session.
    pub fn resume_spec(&self, session_id: Uuid) -> Result<ScanResumeSpec> {
        let (project, mode, state, config): (String, String, String, String) = self
            .database
            .connection()
            .query_row(
                "SELECT project_id,mode,state,config_json FROM scan_sessions WHERE id=?1",
                [session_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(store_error)?;
        if !matches!(state.as_str(), "interrupted" | "blocked") {
            return Err(DedupeError::Safety(format!(
                "Phiên quét {session_id} không thể tiếp tục từ trạng thái {state}"
            )));
        }
        let config: StoredScanConfig = serde_json::from_str(&config)?;
        Ok(ScanResumeSpec {
            session_id,
            project_id: Uuid::parse_str(&project)
                .map_err(|error| DedupeError::State(format!("UUID dự án không hợp lệ: {error}")))?,
            mode: parse_mode(&mode)?,
            all_files: config.all_files,
        })
    }

    /// Reset only read-only evidence before retrying the same session from a safe stage boundary.
    /// Source files and quarantine inventory are never touched.
    pub fn prepare_resume(&self, session_id: Uuid) -> Result<()> {
        let mut connection = self.database.connection();
        let transaction = connection.transaction().map_err(store_error)?;
        let plans: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM operation_plans WHERE session_id=?1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        if plans != 0 {
            return Err(DedupeError::Safety(
                "Không thể đặt lại để tiếp tục một phiên đã có kế hoạch thao tác".into(),
            ));
        }
        transaction
            .execute(
                "DELETE FROM duplicate_groups WHERE session_id=?1",
                [session_id.to_string()],
            )
            .map_err(store_error)?;
        transaction
            .execute(
                "DELETE FROM file_snapshots WHERE session_id=?1",
                [session_id.to_string()],
            )
            .map_err(store_error)?;
        transaction
            .execute(
                "DELETE FROM error_records WHERE session_id=?1",
                [session_id.to_string()],
            )
            .map_err(store_error)?;
        transaction
            .execute(
                "DELETE FROM scan_checkpoints WHERE session_id=?1",
                [session_id.to_string()],
            )
            .map_err(store_error)?;
        let changed = transaction
            .execute(
                "UPDATE scan_sessions SET state='enumerating',finished_at=NULL,updated_at=?1,
                 control_request='none',resume_state=NULL,blocked_reason=NULL,
                 discovered_files=0,processed_files=0,bytes_read=0,duplicate_groups=0,
                 reclaimable_bytes=0,error_count=0,skipped_count=0,unstable_count=0
                 WHERE id=?2 AND state IN ('interrupted','blocked')",
                params![Utc::now().to_rfc3339(), session_id.to_string()],
            )
            .map_err(store_error)?;
        if changed != 1 {
            return Err(DedupeError::Safety(
                "Trạng thái quét đã thay đổi trước khi chuẩn bị tiếp tục".into(),
            ));
        }
        transaction.commit().map_err(store_error)?;
        Ok(())
    }

    /// Persist the latest restart boundary for diagnostics and recovery decisions.
    pub fn checkpoint(&self, session_id: Uuid, stage: &str, committed_items: u64) -> Result<()> {
        self.database
            .connection()
            .execute(
                "INSERT INTO scan_checkpoints(session_id,stage,cursor_json,committed_items,updated_at)
                 VALUES (?1,?2,'{}',?3,?4)
                 ON CONFLICT(session_id) DO UPDATE SET stage=excluded.stage,
                    cursor_json=excluded.cursor_json,committed_items=excluded.committed_items,
                    updated_at=excluded.updated_at",
                params![
                    session_id.to_string(),
                    stage,
                    as_i64(committed_items)?,
                    Utc::now().to_rfc3339()
                ],
            )
            .map_err(store_error)?;
        Ok(())
    }

    /// Update counters and final read-only state.
    pub fn complete_session(
        &self,
        session_id: Uuid,
        progress: dedupe_core::progress::ProgressSnapshot,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let mut connection = self.database.connection();
        let transaction = connection.transaction().map_err(store_error)?;
        transaction
            .execute(
                "UPDATE scan_sessions SET state='completed', finished_at=?1, updated_at=?1,
                 control_request='none',resume_state=NULL,blocked_reason=NULL,
                 discovered_files=?2, processed_files=?3, bytes_read=?4,
                 error_count=?5, skipped_count=?6, unstable_count=?7 WHERE id=?8",
                params![
                    now,
                    as_i64(progress.discovered_files)?,
                    as_i64(progress.processed_files)?,
                    as_i64(progress.bytes_read)?,
                    as_i64(progress.errors)?,
                    as_i64(progress.skipped)?,
                    as_i64(progress.unstable)?,
                    session_id.to_string(),
                ],
            )
            .map_err(store_error)?;
        transaction
            .execute(
                "UPDATE projects SET last_scan_at=?1,updated_at=?1
                 WHERE id=(SELECT project_id FROM scan_sessions WHERE id=?2)",
                params![now, session_id.to_string()],
            )
            .map_err(store_error)?;
        transaction.commit().map_err(store_error)?;
        Ok(())
    }

    /// Change only the workflow state at a safe boundary.
    pub fn set_state(&self, session_id: Uuid, state: &str) -> Result<()> {
        if !matches!(
            state,
            "enumerating"
                | "quick_hashing"
                | "blake3_hashing"
                | "sha256_hashing"
                | "grouping"
                | "pausing"
                | "paused"
                | "cancelling"
                | "cancelled"
                | "interrupted"
                | "recovering"
                | "completed"
                | "blocked"
        ) {
            return Err(DedupeError::InvalidInput(format!(
                "Không hỗ trợ trạng thái quét: {state}"
            )));
        }
        let active_stage = matches!(
            state,
            "enumerating"
                | "quick_hashing"
                | "blake3_hashing"
                | "sha256_hashing"
                | "grouping"
                | "recovering"
        );
        if active_stage {
            self.database
                .connection()
                .execute(
                    "UPDATE scan_sessions SET
                        state=CASE WHEN state IN ('pausing','paused','cancelling')
                            THEN state ELSE ?1 END,
                        resume_state=CASE WHEN state IN ('pausing','paused')
                            THEN ?1 ELSE resume_state END,
                        blocked_reason=NULL,updated_at=?2 WHERE id=?3",
                    params![state, Utc::now().to_rfc3339(), session_id.to_string()],
                )
                .map_err(store_error)?;
        } else {
            self.database
                .connection()
                .execute(
                    "UPDATE scan_sessions SET state=?1,
                        control_request=CASE WHEN ?1 IN ('cancelled','interrupted','completed','blocked')
                            THEN 'none' ELSE control_request END,
                        resume_state=CASE WHEN ?1 IN ('cancelled','interrupted','completed','blocked')
                            THEN NULL ELSE resume_state END,
                        updated_at=?2 WHERE id=?3",
                    params![state, Utc::now().to_rfc3339(), session_id.to_string()],
                )
                .map_err(store_error)?;
        }
        Ok(())
    }

    /// Persist the exact root cause when a scan worker cannot continue safely.
    pub fn block_session(&self, session_id: Uuid, reason: &str) -> Result<()> {
        let changed = self
            .database
            .connection()
            .execute(
                "UPDATE scan_sessions SET state='blocked',control_request='none',
                    resume_state=NULL,blocked_reason=?1,updated_at=?2 WHERE id=?3",
                params![reason, Utc::now().to_rfc3339(), session_id.to_string()],
            )
            .map_err(store_error)?;
        if changed != 1 {
            return Err(DedupeError::State(format!(
                "Không tìm thấy phiên quét để lưu nguyên nhân bị chặn: {session_id}"
            )));
        }
        Ok(())
    }

    /// Persist a cross-process pause/resume/cancel request after validating the current state.
    pub fn request_control(&self, session_id: Uuid, action: ScanControlRequest) -> Result<()> {
        if action == ScanControlRequest::None {
            return Err(DedupeError::InvalidInput(
                "none không phải yêu cầu điều khiển quét có thể thực thi".into(),
            ));
        }
        let mut connection = self.database.connection();
        let transaction = connection.transaction().map_err(store_error)?;
        let (state, resume_state): (String, Option<String>) = transaction
            .query_row(
                "SELECT state,resume_state FROM scan_sessions WHERE id=?1",
                [session_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(store_error)?;
        let now = Utc::now().to_rfc3339();
        match action {
            ScanControlRequest::Pause => {
                if !matches!(
                    state.as_str(),
                    "enumerating"
                        | "quick_hashing"
                        | "blake3_hashing"
                        | "sha256_hashing"
                        | "grouping"
                        | "recovering"
                ) {
                    return Err(DedupeError::Safety(format!(
                        "Phiên quét {session_id} không thể tạm dừng từ trạng thái {state}"
                    )));
                }
                transaction
                    .execute(
                        "UPDATE scan_sessions SET state='pausing',control_request='pause',
                            resume_state=?1,updated_at=?2 WHERE id=?3",
                        params![state, now, session_id.to_string()],
                    )
                    .map_err(store_error)?;
            }
            ScanControlRequest::Resume => {
                if !matches!(state.as_str(), "pausing" | "paused") || resume_state.is_none() {
                    return Err(DedupeError::Safety(format!(
                        "Phiên quét {session_id} không thể tiếp tục từ trạng thái {state}"
                    )));
                }
                transaction
                    .execute(
                        "UPDATE scan_sessions SET control_request='resume',updated_at=?1
                         WHERE id=?2",
                        params![now, session_id.to_string()],
                    )
                    .map_err(store_error)?;
            }
            ScanControlRequest::Cancel => {
                if matches!(
                    state.as_str(),
                    "cancelled" | "completed" | "blocked" | "interrupted"
                ) {
                    return Err(DedupeError::Safety(format!(
                        "Phiên quét {session_id} không thể hủy từ trạng thái {state}"
                    )));
                }
                transaction
                    .execute(
                        "UPDATE scan_sessions SET state='cancelling',control_request='cancel',
                            resume_state=NULL,updated_at=?1 WHERE id=?2",
                        params![now, session_id.to_string()],
                    )
                    .map_err(store_error)?;
            }
            ScanControlRequest::None => {
                return Err(DedupeError::InvalidInput(
                    "none không phải yêu cầu điều khiển quét có thể thực thi".into(),
                ));
            }
        }
        transaction.commit().map_err(store_error)?;
        Ok(())
    }

    /// Read the latest durable request without changing it.
    pub fn pending_control(&self, session_id: Uuid) -> Result<ScanControlRequest> {
        let request: String = self
            .database
            .connection()
            .query_row(
                "SELECT control_request FROM scan_sessions WHERE id=?1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        match request.as_str() {
            "none" => Ok(ScanControlRequest::None),
            "pause" => Ok(ScanControlRequest::Pause),
            "resume" => Ok(ScanControlRequest::Resume),
            "cancel" => Ok(ScanControlRequest::Cancel),
            _ => Err(DedupeError::State(format!(
                "Không nhận diện được yêu cầu điều khiển quét đã lưu: {request}"
            ))),
        }
    }

    /// Acknowledge a pause/resume request after updating the in-process control token.
    pub fn acknowledge_control(&self, session_id: Uuid, action: ScanControlRequest) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let changed = match action {
            ScanControlRequest::Pause => self
                .database
                .connection()
                .execute(
                    "UPDATE scan_sessions SET state='paused',control_request='none',updated_at=?1
                     WHERE id=?2 AND control_request='pause'",
                    params![now, session_id.to_string()],
                )
                .map_err(store_error)?,
            ScanControlRequest::Resume => self
                .database
                .connection()
                .execute(
                    "UPDATE scan_sessions SET state=resume_state,control_request='none',
                        resume_state=NULL,updated_at=?1
                     WHERE id=?2 AND control_request='resume' AND resume_state IS NOT NULL",
                    params![now, session_id.to_string()],
                )
                .map_err(store_error)?,
            ScanControlRequest::None | ScanControlRequest::Cancel => {
                return Err(DedupeError::InvalidInput(
                    "Chỉ có thể xác nhận yêu cầu pause hoặc resume".into(),
                ));
            }
        };
        if changed != 1 {
            return Err(DedupeError::Safety(format!(
                "Yêu cầu điều khiển phiên quét {session_id} đã thay đổi trước khi xác nhận"
            )));
        }
        Ok(())
    }

    /// Persist the latest counters without declaring the session complete.
    pub fn update_progress(
        &self,
        session_id: Uuid,
        progress: dedupe_core::progress::ProgressSnapshot,
    ) -> Result<()> {
        self.database
            .connection()
            .execute(
                "UPDATE scan_sessions SET updated_at=?1,discovered_files=?2,processed_files=?3,
                 bytes_read=?4,error_count=?5,skipped_count=?6,unstable_count=?7 WHERE id=?8",
                params![
                    Utc::now().to_rfc3339(),
                    as_i64(progress.discovered_files)?,
                    as_i64(progress.processed_files)?,
                    as_i64(progress.bytes_read)?,
                    as_i64(progress.errors)?,
                    as_i64(progress.skipped)?,
                    as_i64(progress.unstable)?,
                    session_id.to_string(),
                ],
            )
            .map_err(store_error)?;
        Ok(())
    }

    /// Poll one session without loading file rows.
    pub fn status(&self, session_id: Uuid) -> Result<ScanSessionRecord> {
        let connection = self.database.connection();
        let row = connection
            .query_row(
                "SELECT state,discovered_files,processed_files,bytes_read,error_count,
                        skipped_count,unstable_count,duplicate_groups,reclaimable_bytes,
                        started_at,finished_at,blocked_reason FROM scan_sessions WHERE id=?1",
                [session_id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                    ))
                },
            )
            .map_err(store_error)?;
        Ok(ScanSessionRecord {
            id: session_id,
            state: row.0,
            discovered_files: to_u64(row.1, "số tệp đã phát hiện")?,
            processed_files: to_u64(row.2, "số tệp đã xử lý")?,
            bytes_read: to_u64(row.3, "số byte đã đọc")?,
            errors: to_u64(row.4, "số lỗi")?,
            skipped: to_u64(row.5, "số mục đã bỏ qua")?,
            unstable: to_u64(row.6, "số tệp không ổn định")?,
            duplicate_groups: to_u64(row.7, "số nhóm trùng lặp")?,
            reclaimable_bytes: to_u64(row.8, "số byte có thể thu hồi")?,
            started_at: row.9,
            finished_at: row.10,
            blocked_reason: row.11,
        })
    }

    /// Resolve the project owning a scan session.
    pub fn project_id(&self, session_id: Uuid) -> Result<Uuid> {
        let value: String = self
            .database
            .connection()
            .query_row(
                "SELECT project_id FROM scan_sessions WHERE id=?1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        Uuid::parse_str(&value)
            .map_err(|error| DedupeError::State(format!("UUID dự án không hợp lệ: {error}")))
    }

    /// Stream one preliminary group at a time from a read snapshot.
    pub fn for_each_candidate_group<F>(
        &self,
        session_id: Uuid,
        mode: ComparisonMode,
        mut callback: F,
    ) -> Result<()>
    where
        F: FnMut(Vec<FileMetadataSnapshot>) -> Result<()>,
    {
        let connection = self
            .database
            .read_connection()
            .map_err(|error| DedupeError::Durability(error.to_string()))?;
        let order = if mode == ComparisonMode::Strict {
            "s.size_bytes, e.normalized_name, e.normalized_path"
        } else {
            "s.size_bytes, e.normalized_path"
        };
        let sql = format!(
            "SELECT e.original_path,e.normalized_path,e.normalized_name,e.extension,
                    s.volume_id,s.file_id,s.size_bytes,s.created_time_ns,s.modified_time_ns,
                    s.link_kind,s.hardlink_count,s.access_status,s.snapshot_token
             FROM file_snapshots s JOIN file_entries e ON e.id=s.file_entry_id
             WHERE s.session_id=?1 AND s.state='metadata_ready'
             ORDER BY {order}"
        );
        let mut statement = connection.prepare(&sql).map_err(store_error)?;
        let mut rows = statement
            .query([session_id.to_string()])
            .map_err(store_error)?;
        let mut current_key: Option<(u64, Option<String>)> = None;
        let mut current_group = Vec::new();
        while let Some(row) = rows.next().map_err(store_error)? {
            let snapshot = snapshot_from_row(row)?;
            let key = (
                snapshot.size_bytes,
                (mode == ComparisonMode::Strict).then(|| snapshot.normalized_name.clone()),
            );
            if current_key
                .as_ref()
                .is_some_and(|existing| existing != &key)
            {
                if current_group.len() >= 2 {
                    callback(std::mem::take(&mut current_group))?;
                } else {
                    current_group.clear();
                }
            }
            current_key = Some(key);
            current_group.push(snapshot);
        }
        if current_group.len() >= 2 {
            callback(current_group)?;
        }
        Ok(())
    }
}

/// Batch-buffered streaming sink.
pub struct SqliteScanSink {
    database: Database,
    project_id: Uuid,
    session_id: Uuid,
    roots: Vec<(Uuid, PathBuf)>,
    pending: Vec<FileMetadataSnapshot>,
    pending_errors: Vec<(PathBuf, String, bool)>,
    batch_size: usize,
}

impl SqliteScanSink {
    /// Construct a sink. Roots are sorted longest-first for deterministic ownership.
    #[must_use]
    pub fn new(
        database: Database,
        project_id: Uuid,
        session_id: Uuid,
        roots: Vec<(Uuid, PathBuf)>,
    ) -> Self {
        let mut roots = roots;
        roots.sort_by_key(|(_, path)| std::cmp::Reverse(path.as_os_str().len()));
        Self {
            database,
            project_id,
            session_id,
            roots,
            pending: Vec::with_capacity(256),
            pending_errors: Vec::new(),
            batch_size: 256,
        }
    }
}

impl ScanSink for SqliteScanSink {
    fn record(&mut self, snapshot: &FileMetadataSnapshot) -> Result<()> {
        self.pending.push(snapshot.clone());
        if self.pending.len() >= self.batch_size {
            self.flush()?;
        }
        Ok(())
    }

    fn record_error(&mut self, path: &Path, error: &DedupeError) -> Result<()> {
        self.pending_errors
            .push((path.to_path_buf(), error.to_string(), error.retryable()));
        if self.pending_errors.len() >= self.batch_size {
            self.flush()?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() && self.pending_errors.is_empty() {
            return Ok(());
        }
        let mut connection = self.database.connection();
        let transaction = connection.transaction().map_err(store_error)?;
        for snapshot in self.pending.drain(..) {
            let root_id = self
                .roots
                .iter()
                .find(|(_, root)| snapshot.path.starts_with(root))
                .map(|(id, _)| *id)
                .ok_or_else(|| {
                    DedupeError::State(format!(
                        "Đường dẫn đã quét không thuộc thư mục gốc nào: {}",
                        snapshot.path.display()
                    ))
                })?;
            let entry_id = Uuid::new_v4();
            let now = Utc::now().to_rfc3339();
            let normalized_key = path_key(&snapshot.path)?;
            let identity_key = path_identity_key(&snapshot.path)?;
            let existing: Option<(String, String)> = transaction
                .query_row(
                    "SELECT id,original_path FROM file_entries
                     WHERE project_id=?1 AND path_key=?2",
                    params![self.project_id.to_string(), normalized_key.as_slice()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(store_error)?;
            let (existing, insert_key) = if let Some((id, original_path)) = existing {
                if path_identity_key(Path::new(&original_path))? == identity_key {
                    (Some(id), normalized_key)
                } else {
                    let collision: Option<(String, String)> = transaction
                        .query_row(
                            "SELECT id,original_path FROM file_entries
                             WHERE project_id=?1 AND path_key=?2",
                            params![self.project_id.to_string(), identity_key.as_slice()],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        )
                        .optional()
                        .map_err(store_error)?;
                    if let Some((collision_id, collision_path)) = collision {
                        if path_identity_key(Path::new(&collision_path))? != identity_key {
                            return Err(DedupeError::State(format!(
                                "Xung đột khóa đường dẫn chính xác tại {}",
                                snapshot.path.display()
                            )));
                        }
                        (Some(collision_id), identity_key)
                    } else {
                        (None, identity_key)
                    }
                }
            } else {
                (None, normalized_key)
            };
            let entry_id = if let Some(existing) = existing {
                transaction
                    .execute(
                        "UPDATE file_entries SET root_id=?1,original_path=?2,normalized_path=?3,
                         normalized_name=?4,extension=?5,last_seen_at=?6 WHERE id=?7",
                        params![
                            root_id.to_string(),
                            snapshot.path.to_string_lossy(),
                            snapshot.normalized_path,
                            snapshot.normalized_name,
                            snapshot.extension,
                            now,
                            existing,
                        ],
                    )
                    .map_err(store_error)?;
                existing
            } else {
                transaction
                    .execute(
                        "INSERT INTO file_entries (
                            id,project_id,root_id,original_path,normalized_path,path_key,
                            normalized_name,extension,created_at,last_seen_at
                         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?9)",
                        params![
                            entry_id.to_string(),
                            self.project_id.to_string(),
                            root_id.to_string(),
                            snapshot.path.to_string_lossy(),
                            snapshot.normalized_path,
                            insert_key.as_slice(),
                            snapshot.normalized_name,
                            snapshot.extension,
                            now,
                        ],
                    )
                    .map_err(store_error)?;
                entry_id.to_string()
            };
            transaction
                .execute(
                    "INSERT INTO file_snapshots (
                        id,session_id,file_entry_id,volume_id,file_id,size_bytes,
                        created_time_ns,modified_time_ns,link_kind,hardlink_count,
                        access_status,state,snapshot_token,observed_at
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'metadata_ready',?12,?13)
                     ON CONFLICT(session_id,file_entry_id) DO UPDATE SET
                        volume_id=excluded.volume_id,file_id=excluded.file_id,
                        size_bytes=excluded.size_bytes,created_time_ns=excluded.created_time_ns,
                        modified_time_ns=excluded.modified_time_ns,link_kind=excluded.link_kind,
                        hardlink_count=excluded.hardlink_count,
                        access_status=excluded.access_status,state='metadata_ready',
                        snapshot_token=excluded.snapshot_token,observed_at=excluded.observed_at",
                    params![
                        Uuid::new_v4().to_string(),
                        self.session_id.to_string(),
                        entry_id,
                        snapshot
                            .identity
                            .as_ref()
                            .map(|identity| identity.volume_id.as_str()),
                        snapshot
                            .identity
                            .as_ref()
                            .map(|identity| identity.file_id.as_str()),
                        as_i64(snapshot.size_bytes)?,
                        snapshot.created_ns.map(as_i128_i64).transpose()?,
                        as_i128_i64(snapshot.modified_ns)?,
                        link_name(snapshot.link_kind),
                        snapshot.hardlink_count.map(as_i64).transpose()?,
                        access_name(snapshot.access_status),
                        snapshot.snapshot_token.as_slice(),
                        now,
                    ],
                )
                .map_err(store_error)?;
        }
        for (path, message, retryable) in self.pending_errors.drain(..) {
            transaction
                .execute(
                    "INSERT INTO error_records (
                        id,project_id,session_id,operation,category,message,retryable,occurred_at
                     ) VALUES (?1,?2,?3,'scan','file_error',?4,?5,?6)",
                    params![
                        Uuid::new_v4().to_string(),
                        self.project_id.to_string(),
                        self.session_id.to_string(),
                        format!("{}: {message}", path.display()),
                        i64::from(retryable),
                        Utc::now().to_rfc3339(),
                    ],
                )
                .map_err(store_error)?;
        }
        transaction.commit().map_err(store_error)?;
        Ok(())
    }
}

use rusqlite::OptionalExtension;

fn snapshot_from_row(row: &rusqlite::Row<'_>) -> Result<FileMetadataSnapshot> {
    let volume: Option<String> = row.get(4).map_err(store_error)?;
    let file_id: Option<String> = row.get(5).map_err(store_error)?;
    let size: i64 = row.get(6).map_err(store_error)?;
    let created: Option<i64> = row.get(7).map_err(store_error)?;
    let modified: i64 = row.get(8).map_err(store_error)?;
    let link: String = row.get(9).map_err(store_error)?;
    let access: String = row.get(11).map_err(store_error)?;
    let token: Vec<u8> = row.get(12).map_err(store_error)?;
    let snapshot_token: [u8; 32] = token
        .try_into()
        .map_err(|_| DedupeError::State("Token ảnh chụp đã lưu có độ dài sai".into()))?;
    Ok(FileMetadataSnapshot {
        path: PathBuf::from(row.get::<_, String>(0).map_err(store_error)?),
        normalized_path: row.get(1).map_err(store_error)?,
        normalized_name: row.get(2).map_err(store_error)?,
        extension: row.get(3).map_err(store_error)?,
        identity: volume
            .zip(file_id)
            .map(|(volume_id, file_id)| FileIdentity { volume_id, file_id }),
        size_bytes: u64::try_from(size)
            .map_err(|_| DedupeError::State("Kích thước tệp đã lưu là số âm".into()))?,
        created_ns: created.map(i128::from),
        modified_ns: i128::from(modified),
        link_kind: parse_link(&link)?,
        hardlink_count: row
            .get::<_, Option<i64>>(10)
            .map_err(store_error)?
            .map(u64::try_from)
            .transpose()
            .map_err(|_| DedupeError::State("Số hard link đã lưu là số âm".into()))?,
        access_status: parse_access(&access)?,
        snapshot_token,
    })
}

fn as_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        DedupeError::Safety("Giá trị vượt quá phạm vi số nguyên 64-bit có dấu của SQLite".into())
    })
}

fn to_u64(value: i64, field: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| DedupeError::State(format!("Giá trị {field} đã lưu là số âm")))
}

fn as_i128_i64(value: i128) -> Result<i64> {
    i64::try_from(value).map_err(|_| {
        DedupeError::Safety(
            "Dấu thời gian vượt quá phạm vi số nguyên 64-bit có dấu của SQLite".into(),
        )
    })
}

fn mode_name(mode: ComparisonMode) -> &'static str {
    match mode {
        ComparisonMode::Strict => "strict",
        ComparisonMode::Content => "content",
    }
}

fn parse_mode(mode: &str) -> Result<ComparisonMode> {
    match mode {
        "strict" => Ok(ComparisonMode::Strict),
        "content" => Ok(ComparisonMode::Content),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được chế độ so sánh đã lưu: {mode}"
        ))),
    }
}

fn link_name(value: LinkKind) -> &'static str {
    match value {
        LinkKind::Regular => "regular",
        LinkKind::HardLink => "hardlink",
        LinkKind::Symlink => "symlink",
        LinkKind::Junction => "junction",
        LinkKind::Other => "other",
    }
}

fn parse_link(value: &str) -> Result<LinkKind> {
    match value {
        "regular" => Ok(LinkKind::Regular),
        "hardlink" => Ok(LinkKind::HardLink),
        "symlink" => Ok(LinkKind::Symlink),
        "junction" => Ok(LinkKind::Junction),
        "other" => Ok(LinkKind::Other),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được loại liên kết đã lưu: {value}"
        ))),
    }
}

fn access_name(value: AccessStatus) -> &'static str {
    match value {
        AccessStatus::Readable => "readable",
        AccessStatus::Locked => "locked",
        AccessStatus::Denied => "denied",
        AccessStatus::Offline => "offline",
        AccessStatus::Missing => "missing",
        AccessStatus::Error => "error",
    }
}

fn parse_access(value: &str) -> Result<AccessStatus> {
    match value {
        "readable" => Ok(AccessStatus::Readable),
        "locked" => Ok(AccessStatus::Locked),
        "denied" => Ok(AccessStatus::Denied),
        "offline" => Ok(AccessStatus::Offline),
        "missing" => Ok(AccessStatus::Missing),
        "error" => Ok(AccessStatus::Error),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được trạng thái truy cập đã lưu: {value}"
        ))),
    }
}

fn store_error(error: rusqlite::Error) -> DedupeError {
    DedupeError::Durability(error.to_string())
}
