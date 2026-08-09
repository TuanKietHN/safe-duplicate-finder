//! Deterministic keeper selection that can never leave a group without a retained member.

use std::{cmp::Ordering, path::Path};

use crate::{
    DedupeError, Result,
    model::{DuplicateGroup, KeepPolicy, MemberAction},
};

/// Apply a keep policy to one proven group.
pub fn apply(group: &mut DuplicateGroup, policy: &KeepPolicy) -> Result<()> {
    if group.members.is_empty() {
        return Err(DedupeError::State(
            "Không thể chọn tệp giữ lại từ một nhóm rỗng".into(),
        ));
    }
    let keeper = match policy {
        KeepPolicy::Manual(path) => group
            .members
            .iter()
            .position(|member| member.file.metadata.path == *path)
            .ok_or_else(|| {
                DedupeError::InvalidInput(
                    "Tệp giữ lại được chọn thủ công không nằm trong nhóm".into(),
                )
            })?,
        KeepPolicy::Oldest => select_by(group, |left, right| {
            left.file
                .metadata
                .modified_ns
                .cmp(&right.file.metadata.modified_ns)
                .then_with(|| {
                    compare_path_length(&left.file.metadata.path, &right.file.metadata.path)
                })
        }),
        KeepPolicy::Newest => select_by(group, |left, right| {
            right
                .file
                .metadata
                .modified_ns
                .cmp(&left.file.metadata.modified_ns)
                .then_with(|| {
                    compare_path_length(&left.file.metadata.path, &right.file.metadata.path)
                })
        }),
        KeepPolicy::ShortestPath => select_by(group, |left, right| {
            compare_path_length(&left.file.metadata.path, &right.file.metadata.path)
        }),
        KeepPolicy::Default { primary_roots } => select_by(group, |left, right| {
            let left_primary = primary_roots
                .iter()
                .any(|root| left.file.metadata.path.starts_with(root));
            let right_primary = primary_roots
                .iter()
                .any(|root| right.file.metadata.path.starts_with(root));
            right_primary
                .cmp(&left_primary)
                .then_with(|| {
                    left.file
                        .metadata
                        .modified_ns
                        .cmp(&right.file.metadata.modified_ns)
                })
                .then_with(|| {
                    compare_path_length(&left.file.metadata.path, &right.file.metadata.path)
                })
        }),
    };
    for (index, member) in group.members.iter_mut().enumerate() {
        if index == keeper {
            member.action = MemberAction::Keep;
            member.reason = keeper_reason(policy);
        } else {
            member.action = MemberAction::Quarantine;
            member.reason = "Không được chính sách hiện hành chọn để giữ lại".into();
        }
    }
    group.validate_keeper()
}

fn select_by<F>(group: &DuplicateGroup, mut compare: F) -> usize
where
    F: FnMut(&crate::model::DuplicateMember, &crate::model::DuplicateMember) -> Ordering,
{
    group
        .members
        .iter()
        .enumerate()
        .min_by(|(_, left), (_, right)| compare(left, right))
        .map_or(0, |(index, _)| index)
}

fn compare_path_length(left: &Path, right: &Path) -> Ordering {
    left.as_os_str()
        .len()
        .cmp(&right.as_os_str().len())
        .then_with(|| left.cmp(right))
}

fn keeper_reason(policy: &KeepPolicy) -> String {
    match policy {
        KeepPolicy::Default { .. } => {
            "Chính sách mặc định: thư mục ưu tiên, cũ nhất, rồi đường dẫn ngắn nhất"
        }
        KeepPolicy::Oldest => "Thời điểm sửa đổi cũ nhất",
        KeepPolicy::Newest => "Thời điểm sửa đổi mới nhất",
        KeepPolicy::ShortestPath => "Đường dẫn ngắn nhất",
        KeepPolicy::Manual(_) => "Tệp giữ lại được chọn thủ công",
    }
    .into()
}
