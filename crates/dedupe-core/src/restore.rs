//! Verified restore uses the same durable move protocol in the reverse direction.

use chrono::Utc;
use uuid::Uuid;

use crate::{
    DedupeError, Result,
    control::ControlToken,
    model::{FileTransaction, TransactionKind, TransactionState},
    ports::{MetadataProvider, SafeMover, TransactionJournal},
    transaction_journal::execute_verified_move,
};

/// Construct a restore transaction by reversing a verified quarantine transaction.
pub fn planned_transaction(quarantine: &FileTransaction) -> Result<FileTransaction> {
    if quarantine.kind != TransactionKind::Quarantine
        || quarantine.state != TransactionState::Verified
    {
        return Err(DedupeError::Safety(
            "Chỉ giao dịch cách ly đã xác minh mới có thể được khôi phục".into(),
        ));
    }
    let provider_snapshot_token = quarantine.snapshot_token;
    Ok(FileTransaction {
        id: Uuid::new_v4(),
        project_id: quarantine.project_id,
        session_id: quarantine.session_id,
        plan_item_id: None,
        kind: TransactionKind::Restore,
        state: TransactionState::Planned,
        source: quarantine.destination.clone(),
        destination: quarantine.source.clone(),
        identity: quarantine.identity.clone(),
        size_bytes: quarantine.size_bytes,
        blake3: quarantine.blake3.clone(),
        sha256: quarantine.sha256.clone(),
        snapshot_token: provider_snapshot_token,
        started_at: Utc::now(),
    })
}

/// Execute a restore without overwriting an occupied original path.
pub fn execute(
    transaction: &mut FileTransaction,
    provider: &dyn MetadataProvider,
    mover: &dyn SafeMover,
    journal: &dyn TransactionJournal,
    control: &ControlToken,
) -> Result<()> {
    if transaction.kind != TransactionKind::Restore {
        return Err(DedupeError::State(
            "Dịch vụ khôi phục nhận được giao dịch không phải khôi phục".into(),
        ));
    }
    execute_verified_move(transaction, provider, mover, journal, control)
}
