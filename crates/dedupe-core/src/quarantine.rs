//! Quarantine transaction construction and collision-safe layout.

use std::path::{Path, PathBuf};

use chrono::Utc;
use uuid::Uuid;

use crate::{
    DedupeError, Result,
    control::ControlToken,
    full_hash,
    model::{
        DuplicateGroup, FileTransaction, MemberAction, ProvenFile, TransactionKind,
        TransactionState,
    },
    ports::{MetadataProvider, SafeMover, TransactionJournal},
    transaction_journal::execute_verified_move,
};

/// Prove that the selected keeper still exists with the sealed identity and full content evidence.
pub fn verify_live_keeper(
    group: &DuplicateGroup,
    provider: &dyn MetadataProvider,
    control: &ControlToken,
) -> Result<()> {
    let keeper = group
        .members
        .iter()
        .find(|member| member.action == MemberAction::Keep)
        .ok_or_else(|| DedupeError::Safety(format!("Nhóm {} không có tệp giữ lại", group.id)))?;
    let expected = &keeper.file.metadata;
    let current = provider.snapshot(&expected.path)?;
    if current.identity != expected.identity
        || current.size_bytes != expected.size_bytes
        || current.snapshot_token != expected.snapshot_token
    {
        return Err(DedupeError::Safety(format!(
            "Tệp giữ lại đã thay đổi kể từ khi lập kế hoạch: {}",
            expected.path.display()
        )));
    }
    let blake3 = full_hash::blake3_file(&current.path, provider, control)?;
    let sha256 = full_hash::sha256_file(&current.path, provider, control)?;
    if !blake3.stable
        || !sha256.stable
        || blake3.digest != group.blake3
        || sha256.digest != group.sha256
    {
        return Err(DedupeError::Safety(format!(
            "Bằng chứng nội dung của tệp giữ lại đã lỗi thời: {}",
            current.path.display()
        )));
    }
    Ok(())
}

/// Construct a unique destination preserving the original relative structure.
pub fn quarantine_destination(
    quarantine_root: &Path,
    project_id: Uuid,
    session_id: Uuid,
    entry_id: Uuid,
    source_root: &Path,
    source: &Path,
) -> Result<PathBuf> {
    let relative = source.strip_prefix(source_root).map_err(|_| {
        DedupeError::Safety(format!(
            "Nguồn {} nằm ngoài thư mục gốc đã cấu hình {}",
            source.display(),
            source_root.display()
        ))
    })?;
    Ok(quarantine_root
        .join(project_id.to_string())
        .join(session_id.to_string())
        .join(entry_id.to_string())
        .join(relative))
}

/// Build an immutable planned quarantine transaction from proven evidence.
pub fn planned_transaction(
    project_id: Uuid,
    session_id: Uuid,
    plan_item_id: Uuid,
    file: &ProvenFile,
    destination: PathBuf,
) -> Result<FileTransaction> {
    let identity = file.metadata.identity.clone().ok_or_else(|| {
        DedupeError::Safety("Cách ly yêu cầu danh tính tệp vật lý ổn định".into())
    })?;
    if !file.blake3.stable || !file.sha256.stable {
        return Err(DedupeError::Safety(
            "Cách ly yêu cầu bằng chứng BLAKE3 và SHA-256 đầy đủ, ổn định".into(),
        ));
    }
    Ok(FileTransaction {
        id: Uuid::new_v4(),
        project_id,
        session_id: Some(session_id),
        plan_item_id: Some(plan_item_id),
        kind: TransactionKind::Quarantine,
        state: TransactionState::Planned,
        source: file.metadata.path.clone(),
        destination,
        identity,
        size_bytes: file.metadata.size_bytes,
        blake3: file.blake3.digest.clone(),
        sha256: file.sha256.digest.clone(),
        snapshot_token: file.metadata.snapshot_token,
        started_at: Utc::now(),
    })
}

/// Execute a planned quarantine transaction after the caller validates the group still has a keeper.
pub fn execute(
    transaction: &mut FileTransaction,
    provider: &dyn MetadataProvider,
    mover: &dyn SafeMover,
    journal: &dyn TransactionJournal,
    control: &ControlToken,
) -> Result<()> {
    if transaction.kind != TransactionKind::Quarantine {
        return Err(DedupeError::State(
            "Dịch vụ cách ly nhận được giao dịch không phải cách ly".into(),
        ));
    }
    execute_verified_move(transaction, provider, mover, journal, control)
}
