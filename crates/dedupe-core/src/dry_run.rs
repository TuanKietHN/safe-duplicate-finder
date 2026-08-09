//! Immutable dry-run summary. This module contains no filesystem mutation function.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    DedupeError, Result,
    model::{DuplicateGroup, MemberAction, OperationPlan},
    ports::MetadataProvider,
};

/// One planned quarantine action shown to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunItem {
    /// Duplicate group.
    pub group_id: uuid::Uuid,
    /// Source that would be quarantined.
    pub source: PathBuf,
    /// Exact bytes represented by the file.
    pub size_bytes: u64,
    /// Reason for the proposal.
    pub reason: String,
}

/// Recheck immutable metadata evidence without mutating any source path.
pub fn validate_fresh(groups: &[DuplicateGroup], provider: &dyn MetadataProvider) -> Result<()> {
    for group in groups {
        for member in &group.members {
            let expected = &member.file.metadata;
            let current = provider.snapshot(&expected.path)?;
            if current.identity != expected.identity
                || current.size_bytes != expected.size_bytes
                || current.snapshot_token != expected.snapshot_token
            {
                return Err(DedupeError::Safety(format!(
                    "Bằng chứng của kế hoạch đã khóa đã lỗi thời đối với {}",
                    expected.path.display()
                )));
            }
        }
    }
    Ok(())
}

/// Exact non-mutating preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunReport {
    /// Plan identifier used by later explicit confirmation.
    pub plan_id: uuid::Uuid,
    /// Proposed quarantine items.
    pub items: Vec<DryRunItem>,
    /// Bytes that become reclaimable only after verified quarantine.
    pub potential_reclaimable_bytes: u64,
}

/// Validate keep invariants and produce a zero-mutation preview.
pub fn build(plan: &OperationPlan) -> Result<DryRunReport> {
    let mut items = Vec::new();
    let mut bytes = 0_u64;
    for group in &plan.groups {
        group.validate_keeper()?;
        for member in &group.members {
            if member.action == MemberAction::Quarantine {
                bytes = bytes.saturating_add(member.file.metadata.size_bytes);
                items.push(DryRunItem {
                    group_id: group.id,
                    source: member.file.metadata.path.clone(),
                    size_bytes: member.file.metadata.size_bytes,
                    reason: member.reason.clone(),
                });
            }
        }
    }
    Ok(DryRunReport {
        plan_id: plan.id,
        items,
        potential_reclaimable_bytes: bytes,
    })
}
