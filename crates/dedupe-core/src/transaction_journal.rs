//! Shared transaction execution with durable ordering and destination verification.

use crate::{
    DedupeError, Result,
    control::ControlToken,
    full_hash,
    model::{FileTransaction, TransactionState},
    ports::{MetadataProvider, SafeMover, TransactionJournal},
};

/// Execute one preconstructed quarantine or restore transaction.
///
/// The caller chooses source/destination and expected evidence. This function never overwrites and
/// counts no reclaimed bytes; persistence adapters may count bytes only after `Verified` is durable.
pub fn execute_verified_move(
    transaction: &mut FileTransaction,
    provider: &dyn MetadataProvider,
    mover: &dyn SafeMover,
    journal: &dyn TransactionJournal,
    control: &ControlToken,
) -> Result<()> {
    if transaction.state != TransactionState::Planned {
        return Err(DedupeError::State(
            "Thao tác thay đổi mới phải bắt đầu ở trạng thái đã lên kế hoạch".into(),
        ));
    }
    journal.create(transaction)?;
    control.checkpoint()?;
    let source = match provider.snapshot(&transaction.source) {
        Ok(source) => source,
        Err(error) => {
            transition(
                transaction,
                TransactionState::PreflightFailed,
                None,
                Some(&error.to_string()),
                journal,
            )?;
            return Err(error);
        }
    };
    if source.identity.as_ref() != Some(&transaction.identity)
        || source.size_bytes != transaction.size_bytes
        || source.snapshot_token != transaction.snapshot_token
    {
        let reason =
            "Danh tính, kích thước hoặc bằng chứng sửa đổi của nguồn đã thay đổi trước thao tác";
        transition(
            transaction,
            TransactionState::PreflightFailed,
            None,
            Some(reason),
            journal,
        )?;
        return Err(DedupeError::Safety(reason.into()));
    }
    transition(
        transaction,
        TransactionState::PreflightValidated,
        Some("Siêu dữ liệu và danh tính nguồn khớp"),
        None,
        journal,
    )?;
    control.checkpoint()?;
    transition(transaction, TransactionState::Moving, None, None, journal)?;
    if let Err(error) =
        mover.move_no_replace(&transaction.source, &transaction.destination, &source)
    {
        transition(
            transaction,
            TransactionState::MoveFailed,
            None,
            Some(&error.to_string()),
            journal,
        )?;
        transition(
            transaction,
            TransactionState::RecoveryRequired,
            None,
            Some("Kết quả di chuyển yêu cầu đối soát nguồn/đích"),
            journal,
        )?;
        return Err(error);
    }
    transition(
        transaction,
        TransactionState::MovedUnverified,
        None,
        None,
        journal,
    )?;
    let verification = verify_destination(transaction, provider, control);
    if let Err(error) = verification {
        transition(
            transaction,
            TransactionState::VerifyFailed,
            Some("Xác minh đích thất bại"),
            Some(&error.to_string()),
            journal,
        )?;
        transition(
            transaction,
            TransactionState::RecoveryRequired,
            None,
            Some("Đích tồn tại nhưng chưa được xác minh"),
            journal,
        )?;
        return Err(error);
    }
    transition(
        transaction,
        TransactionState::Verified,
        Some("Đã xác minh kích thước, danh tính, BLAKE3 và SHA-256 của đích"),
        None,
        journal,
    )?;
    Ok(())
}

/// Reproduce full destination verification during startup recovery.
pub fn verify_destination(
    transaction: &FileTransaction,
    provider: &dyn MetadataProvider,
    control: &ControlToken,
) -> Result<()> {
    control.checkpoint()?;
    let destination = provider.snapshot(&transaction.destination)?;
    if destination.identity.as_ref() != Some(&transaction.identity)
        || destination.size_bytes != transaction.size_bytes
    {
        return Err(DedupeError::Safety(
            "Danh tính vật lý hoặc kích thước chính xác của đích không khớp giao dịch".into(),
        ));
    }
    let blake3 = full_hash::blake3_file(&transaction.destination, provider, control)?;
    if !blake3.stable || blake3.digest != transaction.blake3 {
        return Err(DedupeError::Safety(
            "Xác minh BLAKE3 hoặc độ ổn định của đích thất bại".into(),
        ));
    }
    let sha256 = full_hash::sha256_file(&transaction.destination, provider, control)?;
    if !sha256.stable || sha256.digest != transaction.sha256 {
        return Err(DedupeError::Safety(
            "Xác minh SHA-256 hoặc độ ổn định của đích thất bại".into(),
        ));
    }
    Ok(())
}

/// Append a validated transition and only then update the in-memory current-state projection.
pub fn transition(
    transaction: &mut FileTransaction,
    next: TransactionState,
    verification: Option<&str>,
    error: Option<&str>,
    journal: &dyn TransactionJournal,
) -> Result<()> {
    if !transaction.state.can_transition_to(next) {
        return Err(DedupeError::State(format!(
            "Chuyển trạng thái giao dịch không hợp lệ {:?} -> {:?}",
            transaction.state, next
        )));
    }
    journal.transition(transaction, next, verification, error)?;
    transaction.state = next;
    Ok(())
}
