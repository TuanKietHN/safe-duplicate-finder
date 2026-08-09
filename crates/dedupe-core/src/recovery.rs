//! Startup reconciliation observes actual paths and never assumes an interrupted move succeeded.

use crate::{
    DedupeError, Result,
    control::ControlToken,
    model::{FileTransaction, TransactionState},
    ports::{MetadataProvider, TransactionJournal},
    transaction_journal::{transition, verify_destination},
};

/// Observable filesystem disposition after interruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reconciliation {
    /// Only the source exists; no data appears moved.
    SourceOnly,
    /// Only the destination exists and was fully verified.
    DestinationVerified,
    /// Both exist; preserve both for manual resolution.
    Both,
    /// Neither exists; urgent external recovery is required.
    Missing,
}

/// Inspect source/destination and append the safest explainable state.
pub fn reconcile(
    transaction: &mut FileTransaction,
    provider: &dyn MetadataProvider,
    journal: &dyn TransactionJournal,
    control: &ControlToken,
) -> Result<Reconciliation> {
    let source_exists = transaction.source.exists();
    let destination_exists = transaction.destination.exists();
    match (source_exists, destination_exists) {
        (true, false) => {
            transition(
                transaction,
                TransactionState::ReconciledSourceOnly,
                Some("Đối soát khi khởi động chỉ tìm thấy nguồn; không giả định đã có thay đổi"),
                None,
                journal,
            )?;
            Ok(Reconciliation::SourceOnly)
        }
        (false, true) => {
            verify_destination(transaction, provider, control)?;
            if transaction.state == TransactionState::Moving {
                transition(
                    transaction,
                    TransactionState::MovedUnverified,
                    Some("Đối soát khi khởi động đã tìm thấy đích"),
                    None,
                    journal,
                )?;
            }
            if matches!(
                transaction.state,
                TransactionState::MovedUnverified | TransactionState::RecoveryRequired
            ) {
                transition(
                    transaction,
                    TransactionState::Verified,
                    Some("Đối soát khi khởi động đã xác minh đích"),
                    None,
                    journal,
                )?;
            }
            Ok(Reconciliation::DestinationVerified)
        }
        (true, true) => {
            transition(
                transaction,
                TransactionState::ReconciledBoth,
                Some("Đối soát khi khởi động giữ nguyên cả hai đường dẫn để xem xét rõ ràng"),
                None,
                journal,
            )?;
            Ok(Reconciliation::Both)
        }
        (false, false) => {
            let reason = "Cả nguồn lẫn đích của giao dịch đều không tồn tại";
            transition(
                transaction,
                TransactionState::ReconciledMissing,
                Some("Đối soát khi khởi động không tìm thấy đường dẫn nào"),
                Some(reason),
                journal,
            )?;
            Err(DedupeError::Safety(reason.into()))
        }
    }
}
