use std::path::PathBuf;

use dedupe_core::{
    keep_policy,
    model::{
        AccessStatus, ComparisonMode, DuplicateGroup, DuplicateMember, FileIdentity,
        FileMetadataSnapshot, HashAlgorithm, HashResult, KeepPolicy, LinkKind, MemberAction,
        ProvenFile,
    },
};
use uuid::Uuid;

#[test]
fn every_automatic_policy_keeps_exactly_one_member_across_bounded_domain()
-> Result<(), Box<dyn std::error::Error>> {
    for member_count in 2..=64 {
        for seed in 0..=16_u64 {
            let base = group(member_count, seed);
            let policies = [
                KeepPolicy::Default {
                    primary_roots: vec![PathBuf::from("preferred")],
                },
                KeepPolicy::Oldest,
                KeepPolicy::Newest,
                KeepPolicy::ShortestPath,
            ];
            for policy in policies {
                let mut candidate = base.clone();
                keep_policy::apply(&mut candidate, &policy)?;
                let keepers = candidate
                    .members
                    .iter()
                    .filter(|member| member.action == MemberAction::Keep)
                    .count();
                let quarantined = candidate
                    .members
                    .iter()
                    .filter(|member| member.action == MemberAction::Quarantine)
                    .count();
                assert_eq!(keepers, 1, "policy {policy:?}, seed {seed}");
                assert_eq!(quarantined, member_count - 1);
                candidate.validate_keeper()?;
                assert_eq!(
                    candidate.maximum_reclaimable_bytes(),
                    4096_u64.saturating_mul((member_count - 1) as u64)
                );
            }
        }
    }
    Ok(())
}

#[test]
fn manual_policy_accepts_only_a_member_path() -> Result<(), Box<dyn std::error::Error>> {
    let mut candidate = group(8, 4);
    let selected = candidate.members[5].file.metadata.path.clone();
    keep_policy::apply(&mut candidate, &KeepPolicy::Manual(selected.clone()))?;
    assert_eq!(
        candidate
            .members
            .iter()
            .find(|member| member.action == MemberAction::Keep)
            .map(|member| &member.file.metadata.path),
        Some(&selected)
    );

    let result = keep_policy::apply(
        &mut candidate,
        &KeepPolicy::Manual(PathBuf::from("not-in-group.pdf")),
    );
    assert!(result.is_err());
    Ok(())
}

fn group(member_count: usize, seed: u64) -> DuplicateGroup {
    let digest = vec![0x5a; 32];
    let members = (0..member_count)
        .map(|index| {
            let primary = (index as u64 + seed).is_multiple_of(3);
            let path = if primary {
                PathBuf::from(format!("preferred/{index:03}/book.pdf"))
            } else {
                PathBuf::from(format!(
                    "other/{:0width$}/book.pdf",
                    index,
                    width = index % 7 + 1
                ))
            };
            let token = [u8::try_from((index as u64 + seed) % 251).unwrap_or(0); 32];
            DuplicateMember {
                file: ProvenFile {
                    metadata: FileMetadataSnapshot {
                        normalized_path: path.to_string_lossy().to_lowercase(),
                        normalized_name: "book.pdf".into(),
                        extension: Some("pdf".into()),
                        size_bytes: 4096,
                        created_ns: Some(i128::from(index as u64)),
                        modified_ns: i128::from((index as u64 * 37 + seed) % 19),
                        identity: Some(FileIdentity {
                            volume_id: "volume".into(),
                            file_id: format!("{seed}-{index}"),
                        }),
                        link_kind: LinkKind::Regular,
                        hardlink_count: Some(1),
                        access_status: AccessStatus::Readable,
                        snapshot_token: token,
                        path,
                    },
                    blake3: HashResult {
                        algorithm: HashAlgorithm::Blake3,
                        digest: digest.clone(),
                        bytes_read: 4096,
                        snapshot_before: token,
                        snapshot_after: token,
                        stable: true,
                    },
                    sha256: HashResult {
                        algorithm: HashAlgorithm::Sha256,
                        digest: digest.clone(),
                        bytes_read: 4096,
                        snapshot_before: token,
                        snapshot_after: token,
                        stable: true,
                    },
                },
                action: MemberAction::Manual,
                reason: String::new(),
            }
        })
        .collect();
    DuplicateGroup {
        id: Uuid::new_v4(),
        mode: ComparisonMode::Strict,
        size_bytes: 4096,
        normalized_name: Some("book.pdf".into()),
        blake3: digest.clone(),
        sha256: digest,
        members,
    }
}
