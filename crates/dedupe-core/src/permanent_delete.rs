//! High-friction, quarantine-only permanent deletion with durable recovery boundaries.

use std::{collections::HashSet, path::PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    DedupeError, Result,
    control::ControlToken,
    full_hash,
    model::{FileIdentity, FileMetadataSnapshot, LinkKind},
    ports::{MetadataProvider, PermanentDeleteJournal, SafeDeleter},
};

/// Release-specific phrase suffix. Changing this intentionally invalidates memorized confirmations.
pub const CONFIRMATION_RELEASE: &str = "TRÌNH TÌM TỆP TRÙNG LẶP AN TOÀN 0.2.1";

/// Default lifetime of a newly prepared authorization challenge.
pub const DEFAULT_TOKEN_TTL_MINUTES: i64 = 10;

/// User-selected timing gate for one permanent-delete authorization batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermanentDeleteMode {
    /// Require every selected entry's configured retention deadline to have passed.
    RetentionExpired,
    /// Explicitly bypass the retention deadline while preserving all other deletion gates.
    Immediate,
}

/// Verified quarantine evidence. There is deliberately no original/source-path field in this type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletionEntry {
    /// Quarantine registry identifier selected individually by the user.
    pub id: Uuid,
    /// Owning project; one batch cannot cross project boundaries.
    pub project_id: Uuid,
    /// Registry-controlled quarantine path.
    pub quarantine_path: PathBuf,
    /// Stable physical identity captured and verified by quarantine.
    pub identity: FileIdentity,
    /// Exact byte count.
    pub size_bytes: u64,
    /// Full BLAKE3 content digest.
    pub blake3: Vec<u8>,
    /// Full SHA-256 content digest.
    pub sha256: Vec<u8>,
    /// Earliest time at which explicit permanent deletion is allowed.
    pub retain_until: DateTime<Utc>,
}

/// Durable batch state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermanentDeleteBatchState {
    /// Challenge exists but has not authorized mutation.
    Prepared,
    /// Authorization succeeded and deletion/reconciliation may be in progress.
    Executing,
    /// Every selected item is durably recorded deleted.
    Completed,
    /// A failed or interrupted item requires an explicit retry.
    RecoveryRequired,
    /// An unused challenge passed its deadline.
    Expired,
}

impl PermanentDeleteBatchState {
    /// Validate a forward durable transition.
    #[must_use]
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Prepared, Self::Executing | Self::Expired)
                | (Self::Executing, Self::Completed | Self::RecoveryRequired)
                | (Self::RecoveryRequired, Self::Executing | Self::Completed)
        )
    }
}

/// Durable state of one individually selected quarantine entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermanentDeleteItemState {
    /// Immutable evidence was prepared; no deletion intent exists yet.
    Planned,
    /// Durable intent was fsynced before the deletion system call.
    Deleting,
    /// The selected quarantine object is absent after an authorized delete intent.
    Deleted,
    /// The exact selected path failed; the batch stopped before trying another item.
    Failed,
}

impl PermanentDeleteItemState {
    /// Validate a forward or explicit same-item retry transition.
    #[must_use]
    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Planned | Self::Failed, Self::Deleting)
                | (Self::Deleting, Self::Deleted | Self::Failed)
                | (Self::Failed, Self::Deleted)
        )
    }
}

/// One immutable entry and its durable deletion state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermanentDeleteItem {
    /// Quarantine evidence.
    pub entry: DeletionEntry,
    /// Current journaled state.
    pub state: PermanentDeleteItemState,
}

/// Prepared authorization batch. Only a digest of the short-lived raw token is stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermanentDeleteBatch {
    /// Batch identifier.
    pub id: Uuid,
    /// Owning project.
    pub project_id: Uuid,
    /// Current durable state.
    pub state: PermanentDeleteBatchState,
    /// Whether this batch waits for retention or explicitly deletes immediately.
    pub mode: PermanentDeleteMode,
    /// Domain-separated digest of the raw authorization token.
    pub token_digest: [u8; 32],
    /// Digest binding the exact sorted entry identities and content evidence.
    pub selection_digest: [u8; 32],
    /// Exact release-specific phrase required at execution.
    pub confirmation_phrase: String,
    /// Exact selected item count.
    pub entry_count: u64,
    /// Exact selected byte count.
    pub total_bytes: u64,
    /// Challenge creation time.
    pub created_at: DateTime<Utc>,
    /// Deadline for starting an unused challenge.
    pub expires_at: DateTime<Utc>,
    /// Individually selected immutable entries.
    pub items: Vec<PermanentDeleteItem>,
}

/// User-visible first step. The raw token is returned once and never persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermanentDeleteChallenge {
    /// Batch used by the execute step.
    pub batch_id: Uuid,
    /// Short-lived raw token that must be supplied exactly.
    pub token: String,
    /// Timing gate durably bound to this challenge.
    pub mode: PermanentDeleteMode,
    /// Exact phrase that must be typed, including count and bytes.
    pub confirmation_phrase: String,
    /// Exact selected item count.
    pub entry_count: u64,
    /// Exact selected byte count.
    pub total_bytes: u64,
    /// Token start deadline.
    pub expires_at: DateTime<Utc>,
}

/// Result of an execute or recovery attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermanentDeleteOutcome {
    /// Total entries now durably recorded deleted.
    pub deleted_entries: u64,
    /// Total bytes represented by those entries.
    pub deleted_bytes: u64,
}

/// Prepare a short-lived authorization bound to explicit verified quarantine entries.
pub fn prepare(
    entries: Vec<DeletionEntry>,
    journal: &dyn PermanentDeleteJournal,
    now: DateTime<Utc>,
) -> Result<PermanentDeleteChallenge> {
    prepare_with_ttl(
        entries,
        journal,
        now,
        Duration::minutes(DEFAULT_TOKEN_TTL_MINUTES),
    )
}

/// Prepare an explicit immediate-delete authorization that bypasses only the retention deadline.
pub fn prepare_immediate(
    entries: Vec<DeletionEntry>,
    journal: &dyn PermanentDeleteJournal,
    now: DateTime<Utc>,
) -> Result<PermanentDeleteChallenge> {
    prepare_with_ttl_and_mode(
        entries,
        journal,
        now,
        Duration::minutes(DEFAULT_TOKEN_TTL_MINUTES),
        PermanentDeleteMode::Immediate,
    )
}

/// Prepare with an explicit TTL, primarily for deterministic expiry tests.
pub fn prepare_with_ttl(
    entries: Vec<DeletionEntry>,
    journal: &dyn PermanentDeleteJournal,
    now: DateTime<Utc>,
    ttl: Duration,
) -> Result<PermanentDeleteChallenge> {
    prepare_with_ttl_and_mode(
        entries,
        journal,
        now,
        ttl,
        PermanentDeleteMode::RetentionExpired,
    )
}

fn prepare_with_ttl_and_mode(
    mut entries: Vec<DeletionEntry>,
    journal: &dyn PermanentDeleteJournal,
    now: DateTime<Utc>,
    ttl: Duration,
    mode: PermanentDeleteMode,
) -> Result<PermanentDeleteChallenge> {
    if entries.is_empty() {
        return Err(DedupeError::InvalidInput(
            "Xóa vĩnh viễn yêu cầu ít nhất một mục cách ly được chọn riêng lẻ".into(),
        ));
    }
    if ttl <= Duration::zero() {
        return Err(DedupeError::InvalidInput(
            "Thời hạn token xóa vĩnh viễn phải lớn hơn 0".into(),
        ));
    }
    entries.sort_by_key(|entry| entry.id);
    let project_id = entries[0].project_id;
    let mut unique = HashSet::with_capacity(entries.len());
    let mut total_bytes = 0_u64;
    for entry in &entries {
        if !unique.insert(entry.id) {
            return Err(DedupeError::InvalidInput(format!(
                "Mục cách ly {} được chọn nhiều lần",
                entry.id
            )));
        }
        if entry.project_id != project_id {
            return Err(DedupeError::Safety(
                "Một lô xóa vĩnh viễn không thể trải qua nhiều dự án".into(),
            ));
        }
        if mode == PermanentDeleteMode::RetentionExpired && entry.retain_until > now {
            return Err(DedupeError::Safety(format!(
                "Thời hạn lưu giữ cách ly chưa hết đối với mục {} (giữ đến {})",
                entry.id, entry.retain_until
            )));
        }
        if entry.blake3.len() != 32 || entry.sha256.len() != 32 {
            return Err(DedupeError::State(format!(
                "Mục cách ly {} có bằng chứng nội dung đầy đủ không hợp lệ",
                entry.id
            )));
        }
        total_bytes = total_bytes.checked_add(entry.size_bytes).ok_or_else(|| {
            DedupeError::Safety("Tổng số byte được chọn để xóa vĩnh viễn vượt quá u64".into())
        })?;
    }
    let entry_count = u64::try_from(entries.len())
        .map_err(|_| DedupeError::Safety("Số mục đã chọn vượt quá u64".into()))?;
    let raw_token = Uuid::new_v4().to_string();
    let token_digest = digest_token(&raw_token);
    let selection_digest = digest_selection(&entries);
    let confirmation_phrase = confirmation_phrase(entry_count, total_bytes, mode);
    let expires_at = now.checked_add_signed(ttl).ok_or_else(|| {
        DedupeError::State("Thời điểm hết hạn token xóa vĩnh viễn bị tràn".into())
    })?;
    let batch = PermanentDeleteBatch {
        id: Uuid::new_v4(),
        project_id,
        state: PermanentDeleteBatchState::Prepared,
        mode,
        token_digest,
        selection_digest,
        confirmation_phrase: confirmation_phrase.clone(),
        entry_count,
        total_bytes,
        created_at: now,
        expires_at,
        items: entries
            .into_iter()
            .map(|entry| PermanentDeleteItem {
                entry,
                state: PermanentDeleteItemState::Planned,
            })
            .collect(),
    };
    journal.create_batch(&batch)?;
    Ok(PermanentDeleteChallenge {
        batch_id: batch.id,
        token: raw_token,
        mode,
        confirmation_phrase,
        entry_count,
        total_bytes,
        expires_at,
    })
}

/// Execute or explicitly resume one authorized batch.
#[allow(clippy::too_many_lines)]
pub fn execute(
    batch_id: Uuid,
    token: &str,
    confirmation: &str,
    journal: &dyn PermanentDeleteJournal,
    provider: &dyn MetadataProvider,
    deleter: &dyn SafeDeleter,
    control: &ControlToken,
    now: DateTime<Utc>,
) -> Result<PermanentDeleteOutcome> {
    let mut batch = journal.load_batch(batch_id)?;
    validate_loaded_batch(&batch)?;
    if !constant_time_equal(&digest_token(token), &batch.token_digest) {
        return Err(DedupeError::Safety(
            "Token cho phép xóa vĩnh viễn không hợp lệ".into(),
        ));
    }
    if confirmation != batch.confirmation_phrase {
        return Err(DedupeError::Safety(format!(
            "Câu xác nhận phải khớp chính xác: {}",
            batch.confirmation_phrase
        )));
    }
    if batch.state == PermanentDeleteBatchState::Completed {
        return Ok(outcome(&batch));
    }
    if batch.state == PermanentDeleteBatchState::Expired {
        return Err(DedupeError::Safety(
            "Token cho phép xóa vĩnh viễn đã hết hạn".into(),
        ));
    }
    if batch.state == PermanentDeleteBatchState::Prepared && now > batch.expires_at {
        transition_batch(
            &mut batch,
            PermanentDeleteBatchState::Expired,
            None,
            journal,
        )?;
        return Err(DedupeError::Safety(
            "Token cho phép xóa vĩnh viễn đã hết hạn".into(),
        ));
    }
    if matches!(
        batch.state,
        PermanentDeleteBatchState::Prepared | PermanentDeleteBatchState::RecoveryRequired
    ) {
        transition_batch(
            &mut batch,
            PermanentDeleteBatchState::Executing,
            None,
            journal,
        )?;
    }

    // Preflight every remaining path before the first new delete. A missing path is accepted only
    // when an earlier durable `deleting` intent proves this is interruption recovery.
    let mut snapshots = Vec::with_capacity(batch.items.len());
    for index in 0..batch.items.len() {
        control.checkpoint()?;
        let item = &batch.items[index];
        match item.state {
            PermanentDeleteItemState::Deleted => snapshots.push(None),
            PermanentDeleteItemState::Deleting | PermanentDeleteItemState::Failed => match provider
                .snapshot(&item.entry.quarantine_path)
            {
                Ok(snapshot) => {
                    if let Err(error) = verify_entry(&item.entry, &snapshot, provider, control) {
                        return fail_preflight(&mut batch, error, journal);
                    }
                    snapshots.push(Some(snapshot));
                }
                Err(error) if is_missing(&error) => {
                    let mut item = batch.items[index].clone();
                    transition_item(
                        &batch,
                        &mut item,
                        PermanentDeleteItemState::Deleted,
                        Some("Đã đối soát mục bị thiếu sau ý định xóa bền vững"),
                        journal,
                    )?;
                    batch.items[index] = item;
                    snapshots.push(None);
                }
                Err(error) => return fail_preflight(&mut batch, error, journal),
            },
            PermanentDeleteItemState::Planned => {
                let snapshot = match provider.snapshot(&item.entry.quarantine_path) {
                    Ok(snapshot) => snapshot,
                    Err(error) => return fail_preflight(&mut batch, error, journal),
                };
                if let Err(error) = verify_entry(&item.entry, &snapshot, provider, control) {
                    return fail_preflight(&mut batch, error, journal);
                }
                snapshots.push(Some(snapshot));
            }
        }
    }

    for (index, snapshot) in snapshots.into_iter().enumerate() {
        if batch.items[index].state == PermanentDeleteItemState::Deleted {
            continue;
        }
        control.checkpoint()?;
        let mut item = batch.items[index].clone();
        if item.state != PermanentDeleteItemState::Deleting {
            transition_item(
                &batch,
                &mut item,
                PermanentDeleteItemState::Deleting,
                None,
                journal,
            )?;
            batch.items[index] = item.clone();
        }
        let expected = snapshot.ok_or_else(|| {
            DedupeError::State(format!(
                "Thiếu ảnh chụp kiểm tra trước cho mục cách ly {}",
                item.entry.id
            ))
        })?;
        if let Err(error) = deleter.delete_exact(&expected) {
            let detail = error.to_string();
            transition_item(
                &batch,
                &mut item,
                PermanentDeleteItemState::Failed,
                Some(&detail),
                journal,
            )?;
            batch.items[index] = item;
            transition_batch(
                &mut batch,
                PermanentDeleteBatchState::RecoveryRequired,
                Some(&detail),
                journal,
            )?;
            return Err(error);
        }
        // If this append/fsync or DB commit fails, the durable `deleting` intent remains sufficient
        // to reconcile an absent file on the next explicit execute call.
        transition_item(
            &batch,
            &mut item,
            PermanentDeleteItemState::Deleted,
            None,
            journal,
        )?;
        batch.items[index] = item;
    }
    transition_batch(
        &mut batch,
        PermanentDeleteBatchState::Completed,
        None,
        journal,
    )?;
    Ok(outcome(&batch))
}

fn verify_entry(
    entry: &DeletionEntry,
    snapshot: &FileMetadataSnapshot,
    provider: &dyn MetadataProvider,
    control: &ControlToken,
) -> Result<()> {
    if snapshot.path != entry.quarantine_path
        || snapshot.identity.as_ref() != Some(&entry.identity)
        || snapshot.size_bytes != entry.size_bytes
        || !matches!(snapshot.link_kind, LinkKind::Regular)
    {
        return Err(DedupeError::Safety(format!(
            "Mục cách ly {} không còn khớp với danh tính vật lý",
            entry.id
        )));
    }
    let blake3 = full_hash::blake3_file(&entry.quarantine_path, provider, control)?;
    let sha256 = full_hash::sha256_file(&entry.quarantine_path, provider, control)?;
    if !blake3.stable
        || !sha256.stable
        || blake3.digest != entry.blake3
        || sha256.digest != entry.sha256
    {
        return Err(DedupeError::Safety(format!(
            "Bằng chứng nội dung của mục cách ly {} đã thay đổi",
            entry.id
        )));
    }
    Ok(())
}

fn validate_loaded_batch(batch: &PermanentDeleteBatch) -> Result<()> {
    let entries = batch
        .items
        .iter()
        .map(|item| item.entry.clone())
        .collect::<Vec<_>>();
    let count = u64::try_from(entries.len())
        .map_err(|_| DedupeError::State("Số mục xóa đã lưu vượt quá u64".into()))?;
    let bytes = entries.iter().try_fold(0_u64, |total, entry| {
        total
            .checked_add(entry.size_bytes)
            .ok_or_else(|| DedupeError::State("Tổng số byte xóa đã lưu bị tràn".into()))
    })?;
    if count != batch.entry_count
        || bytes != batch.total_bytes
        || digest_selection(&entries) != batch.selection_digest
        || batch.confirmation_phrase != confirmation_phrase(count, bytes, batch.mode)
        || entries
            .iter()
            .any(|entry| entry.project_id != batch.project_id)
    {
        return Err(DedupeError::Safety(
            "Bằng chứng cho phép xóa vĩnh viễn đã lưu không nhất quán".into(),
        ));
    }
    Ok(())
}

fn fail_preflight<T>(
    batch: &mut PermanentDeleteBatch,
    error: DedupeError,
    journal: &dyn PermanentDeleteJournal,
) -> Result<T> {
    let detail = error.to_string();
    transition_batch(
        batch,
        PermanentDeleteBatchState::RecoveryRequired,
        Some(&detail),
        journal,
    )?;
    Err(error)
}

fn transition_batch(
    batch: &mut PermanentDeleteBatch,
    next: PermanentDeleteBatchState,
    error: Option<&str>,
    journal: &dyn PermanentDeleteJournal,
) -> Result<()> {
    if !batch.state.can_transition_to(next) {
        return Err(DedupeError::State(format!(
            "Chuyển trạng thái lô xóa vĩnh viễn không hợp lệ {:?} -> {:?}",
            batch.state, next
        )));
    }
    journal.transition_batch(batch, next, error)?;
    batch.state = next;
    Ok(())
}

fn transition_item(
    batch: &PermanentDeleteBatch,
    item: &mut PermanentDeleteItem,
    next: PermanentDeleteItemState,
    error: Option<&str>,
    journal: &dyn PermanentDeleteJournal,
) -> Result<()> {
    if !item.state.can_transition_to(next) {
        return Err(DedupeError::State(format!(
            "Chuyển trạng thái mục xóa vĩnh viễn không hợp lệ {:?} -> {:?}",
            item.state, next
        )));
    }
    journal.transition_item(batch, item, next, error)?;
    item.state = next;
    Ok(())
}

fn confirmation_phrase(count: u64, bytes: u64, mode: PermanentDeleteMode) -> String {
    let action = match mode {
        PermanentDeleteMode::RetentionExpired => "XÓA VĨNH VIỄN",
        PermanentDeleteMode::Immediate => "XÓA NGAY VĨNH VIỄN",
    };
    format!("{action} {count} TỆP ĐÃ CÁCH LY ({bytes} BYTE) TRONG {CONFIRMATION_RELEASE}")
}

fn digest_token(token: &str) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key("safe-dedupe permanent-delete token v1");
    hasher.update(token.as_bytes());
    *hasher.finalize().as_bytes()
}

fn digest_selection(entries: &[DeletionEntry]) -> [u8; 32] {
    let mut ordered = entries.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|entry| entry.id);
    let mut hasher = blake3::Hasher::new_derive_key("safe-dedupe permanent-delete selection v1");
    for entry in ordered {
        hasher.update(entry.project_id.as_bytes());
        hasher.update(entry.id.as_bytes());
        hasher.update(&(entry.identity.volume_id.len() as u64).to_le_bytes());
        hasher.update(entry.identity.volume_id.as_bytes());
        hasher.update(&(entry.identity.file_id.len() as u64).to_le_bytes());
        hasher.update(entry.identity.file_id.as_bytes());
        hasher.update(&entry.size_bytes.to_le_bytes());
        hasher.update(&entry.blake3);
        hasher.update(&entry.sha256);
    }
    *hasher.finalize().as_bytes()
}

fn constant_time_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn is_missing(error: &DedupeError) -> bool {
    matches!(
        error,
        DedupeError::Io { source, .. } if source.kind() == std::io::ErrorKind::NotFound
    )
}

fn outcome(batch: &PermanentDeleteBatch) -> PermanentDeleteOutcome {
    let mut deleted_entries = 0_u64;
    let mut deleted_bytes = 0_u64;
    for item in &batch.items {
        if item.state == PermanentDeleteItemState::Deleted {
            deleted_entries = deleted_entries.saturating_add(1);
            deleted_bytes = deleted_bytes.saturating_add(item.entry.size_bytes);
        }
    }
    PermanentDeleteOutcome {
        deleted_entries,
        deleted_bytes,
    }
}
