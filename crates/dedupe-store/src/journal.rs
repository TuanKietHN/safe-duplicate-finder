//! `SQLite` current-state projection plus flushed append-only JSONL transaction manifest.

use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use dedupe_core::{
    DedupeError, Result,
    model::{FileTransaction, TransactionKind, TransactionState},
    path_normalization::path_key,
    ports::TransactionJournal,
};
use rusqlite::params;
use serde::Serialize;
use uuid::Uuid;

use crate::Database;

/// Durable transaction journal implementation.
#[derive(Debug, Clone)]
pub struct SqliteTransactionJournal {
    database: Database,
    manifest_path: PathBuf,
}

impl SqliteTransactionJournal {
    /// Create a journal whose JSONL manifest is outside scan/quarantine roots.
    pub fn new(database: Database, manifest_path: impl Into<PathBuf>) -> Result<Self> {
        let manifest_path = manifest_path.into();
        if let Some(parent) = manifest_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                DedupeError::io("tạo thư mục manifest giao dịch", parent, error)
            })?;
        }
        Ok(Self {
            database,
            manifest_path,
        })
    }

    /// Manifest location for manual recovery documentation.
    #[must_use]
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    fn append_manifest(&self, event: &ManifestEvent<'_>) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.manifest_path)
            .map_err(|error| {
                DedupeError::io("mở manifest giao dịch", &self.manifest_path, error)
            })?;
        serde_json::to_writer(&mut file, event)?;
        file.write_all(b"\n").map_err(|error| {
            DedupeError::io("ghi nối manifest giao dịch", &self.manifest_path, error)
        })?;
        file.flush().map_err(|error| {
            DedupeError::io("đẩy dữ liệu manifest giao dịch", &self.manifest_path, error)
        })?;
        file.sync_data().map_err(|error| {
            DedupeError::io("đồng bộ manifest giao dịch", &self.manifest_path, error)
        })?;
        Ok(())
    }
}

impl TransactionJournal for SqliteTransactionJournal {
    fn create(&self, record: &FileTransaction) -> Result<()> {
        if record.state != TransactionState::Planned {
            return Err(DedupeError::State(
                "Tạo nhật ký yêu cầu trạng thái đã lên kế hoạch".into(),
            ));
        }
        let size_bytes = i64::try_from(record.size_bytes).map_err(|_| {
            DedupeError::Safety(
                "Kích thước tệp vượt quá phạm vi số nguyên 64-bit có dấu của SQLite".into(),
            )
        })?;
        let source_key = path_key(&record.source)?;
        let destination_key = path_key(&record.destination)?;
        let timestamp = record.started_at.to_rfc3339();
        self.append_manifest(&ManifestEvent {
            transaction: record,
            from: None,
            to: TransactionState::Planned,
            verification: None,
            error: None,
            occurred_at: timestamp.clone(),
        })?;
        {
            let mut connection = self.database.connection();
            let transaction = connection.transaction().map_err(store_error)?;
            transaction
                .execute(
                    "INSERT INTO file_transactions (
                        id, project_id, session_id, plan_item_id, kind, status,
                        source_path, destination_path, source_path_key, destination_path_key,
                        volume_id, file_id, size_bytes, blake3_digest, sha256_digest,
                        source_snapshot_token, started_at, updated_at
                    ) VALUES (?1,?2,?3,?4,?5,'planned',?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?16)",
                    params![
                        record.id.to_string(),
                        record.project_id.to_string(),
                        record.session_id.map(|value| value.to_string()),
                        record.plan_item_id.map(|value| value.to_string()),
                        kind_name(record),
                        record.source.to_string_lossy(),
                        record.destination.to_string_lossy(),
                        source_key.as_slice(),
                        destination_key.as_slice(),
                        record.identity.volume_id,
                        record.identity.file_id,
                        size_bytes,
                        record.blake3,
                        record.sha256,
                        record.snapshot_token.as_slice(),
                        timestamp,
                    ],
                )
                .map_err(store_error)?;
            transaction
                .execute(
                    "INSERT INTO transaction_events (
                        transaction_id, from_status, to_status, occurred_at
                    ) VALUES (?1,NULL,'planned',?2)",
                    params![record.id.to_string(), record.started_at.to_rfc3339()],
                )
                .map_err(store_error)?;
            insert_audit_event(
                &transaction,
                record,
                None,
                TransactionState::Planned,
                &timestamp,
            )?;
            transaction.commit().map_err(store_error)?;
        }
        Ok(())
    }

    fn transition(
        &self,
        record: &FileTransaction,
        next: TransactionState,
        verification: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        if !record.state.can_transition_to(next) {
            return Err(DedupeError::State(format!(
                "Nhật ký từ chối chuyển trạng thái {:?} -> {:?}",
                record.state, next
            )));
        }
        let occurred_at = Utc::now().to_rfc3339();
        self.append_manifest(&ManifestEvent {
            transaction: record,
            from: Some(record.state),
            to: next,
            verification,
            error,
            occurred_at: occurred_at.clone(),
        })?;
        {
            let mut connection = self.database.connection();
            let transaction = connection.transaction().map_err(store_error)?;
            let changed = transaction
                .execute(
                    "UPDATE file_transactions
                     SET status=?1, updated_at=?2, verified_at=CASE WHEN ?1='verified' THEN ?2 ELSE verified_at END,
                         error_message=COALESCE(?3,error_message)
                     WHERE id=?4 AND status=?5",
                    params![
                        state_name(next),
                        occurred_at,
                        error,
                        record.id.to_string(),
                        state_name(record.state),
                    ],
                )
                .map_err(store_error)?;
            if changed != 1 {
                return Err(DedupeError::State(format!(
                    "Trạng thái hiện tại của giao dịch {} đã thay đổi đồng thời",
                    record.id
                )));
            }
            transaction
                .execute(
                    "INSERT INTO transaction_events (
                        transaction_id, from_status, to_status, verification_result,
                        error_message, occurred_at
                    ) VALUES (?1,?2,?3,?4,?5,?6)",
                    params![
                        record.id.to_string(),
                        state_name(record.state),
                        state_name(next),
                        verification,
                        error,
                        occurred_at,
                    ],
                )
                .map_err(store_error)?;
            update_quarantine_projection(&transaction, record, next, &occurred_at)?;
            insert_audit_event(&transaction, record, Some(record.state), next, &occurred_at)?;
            transaction.commit().map_err(store_error)?;
        }
        Ok(())
    }
}

fn insert_audit_event(
    transaction: &rusqlite::Transaction<'_>,
    record: &FileTransaction,
    from: Option<TransactionState>,
    to: TransactionState,
    occurred_at: &str,
) -> Result<()> {
    let payload = serde_json::to_string(&serde_json::json!({
        "kind": kind_name(record),
        "from": from.map(state_name),
        "to": state_name(to),
        "size_bytes": record.size_bytes,
    }))?;
    transaction
        .execute(
            "INSERT INTO audit_events (
                event_id,project_id,session_id,transaction_id,actor,event_type,
                payload_json,occurred_at
             ) VALUES (?1,?2,?3,?4,'system',?5,?6,?7)",
            params![
                Uuid::new_v4().to_string(),
                record.project_id.to_string(),
                record.session_id.map(|value| value.to_string()),
                record.id.to_string(),
                format!("transaction.{}", state_name(to)),
                payload,
                occurred_at,
            ],
        )
        .map_err(store_error)?;
    Ok(())
}

fn update_quarantine_projection(
    transaction: &rusqlite::Transaction<'_>,
    record: &FileTransaction,
    next: TransactionState,
    occurred_at: &str,
) -> Result<()> {
    match (record.kind, next) {
        (TransactionKind::Quarantine, TransactionState::Verified) => {
            let retention_days: i64 = transaction
                .query_row(
                    "SELECT retention_days FROM projects WHERE id=?1",
                    [record.project_id.to_string()],
                    |row| row.get(0),
                )
                .map_err(store_error)?;
            let retain_until = Utc::now()
                .checked_add_signed(chrono::Duration::days(retention_days))
                .ok_or_else(|| DedupeError::State("Ngày lưu giữ cách ly bị tràn".into()))?
                .to_rfc3339();
            transaction
                .execute(
                    "INSERT INTO quarantine_entries (
                        id,project_id,origin_transaction_id,original_path,quarantine_path,
                        volume_id,file_id,size_bytes,blake3_digest,sha256_digest,state,
                        quarantined_at,retain_until,last_verified_at
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'verified',?11,?12,?11)
                     ON CONFLICT(origin_transaction_id) DO UPDATE SET
                        state='verified',last_verified_at=excluded.last_verified_at",
                    params![
                        Uuid::new_v4().to_string(),
                        record.project_id.to_string(),
                        record.id.to_string(),
                        record.source.to_string_lossy(),
                        record.destination.to_string_lossy(),
                        record.identity.volume_id,
                        record.identity.file_id,
                        i64::try_from(record.size_bytes).map_err(|_| {
                            DedupeError::Safety(
                                "Kích thước tệp vượt quá phạm vi có dấu của SQLite".into(),
                            )
                        })?,
                        record.blake3,
                        record.sha256,
                        occurred_at,
                        retain_until,
                    ],
                )
                .map_err(store_error)?;
        }
        (TransactionKind::Restore, TransactionState::PreflightValidated) => {
            transaction
                .execute(
                    "UPDATE quarantine_entries SET state='restoring'
                     WHERE project_id=?1 AND quarantine_path=?2 AND state='verified'",
                    params![
                        record.project_id.to_string(),
                        record.source.to_string_lossy()
                    ],
                )
                .map_err(store_error)?;
        }
        (TransactionKind::Restore, TransactionState::Verified) => {
            transaction
                .execute(
                    "UPDATE quarantine_entries SET state='restored',restored_at=?1,
                     last_verified_at=?1 WHERE project_id=?2 AND quarantine_path=?3",
                    params![
                        occurred_at,
                        record.project_id.to_string(),
                        record.source.to_string_lossy(),
                    ],
                )
                .map_err(store_error)?;
        }
        (TransactionKind::Restore, TransactionState::RecoveryRequired) => {
            transaction
                .execute(
                    "UPDATE quarantine_entries SET state='recovery_required'
                     WHERE project_id=?1 AND quarantine_path=?2",
                    params![
                        record.project_id.to_string(),
                        record.source.to_string_lossy()
                    ],
                )
                .map_err(store_error)?;
        }
        _ => {}
    }
    Ok(())
}

#[derive(Serialize)]
struct ManifestEvent<'a> {
    transaction: &'a FileTransaction,
    from: Option<TransactionState>,
    to: TransactionState,
    verification: Option<&'a str>,
    error: Option<&'a str>,
    occurred_at: String,
}

fn store_error(error: rusqlite::Error) -> DedupeError {
    DedupeError::Durability(error.to_string())
}

fn kind_name(record: &FileTransaction) -> &'static str {
    match record.kind {
        dedupe_core::model::TransactionKind::Quarantine => "quarantine",
        dedupe_core::model::TransactionKind::Restore => "restore",
    }
}

/// Stable `SQLite` state spelling.
#[must_use]
pub fn state_name(state: TransactionState) -> &'static str {
    match state {
        TransactionState::Planned => "planned",
        TransactionState::PreflightValidated => "preflight_validated",
        TransactionState::Moving => "moving",
        TransactionState::MovedUnverified => "moved_unverified",
        TransactionState::Verified => "verified",
        TransactionState::PreflightFailed => "preflight_failed",
        TransactionState::MoveFailed => "move_failed",
        TransactionState::VerifyFailed => "verify_failed",
        TransactionState::RecoveryRequired => "recovery_required",
        TransactionState::Cancelled => "cancelled",
        TransactionState::ReconciledSourceOnly => "reconciled_source_only",
        TransactionState::ReconciledBoth => "reconciled_both",
        TransactionState::ReconciledMissing => "reconciled_missing",
    }
}
