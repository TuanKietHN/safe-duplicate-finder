//! Cross-format report conformance and escaping tests.

use std::path::PathBuf;

use dedupe_core::model::{
    AccessStatus, ComparisonMode, DuplicateGroup, DuplicateMember, FileIdentity,
    FileMetadataSnapshot, HashAlgorithm, HashResult, LinkKind, MemberAction, ProvenFile,
};
use uuid::Uuid;

#[test]
fn csv_json_and_html_report_the_same_proven_members_without_content()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let group = fixture_group();
    let secret_document_content = "TOP-SECRET-CONTENT-MUST-NOT-LEAK";

    let mut csv_bytes = Vec::new();
    dedupe_report::write_csv(std::slice::from_ref(&group), &mut csv_bytes)?;
    let mut csv = csv::Reader::from_reader(csv_bytes.as_slice());
    let csv_rows = csv.records().collect::<std::result::Result<Vec<_>, _>>()?;
    let group_id = group.id.to_string();
    assert_eq!(csv_rows.len(), 2);
    assert!(
        csv_rows
            .iter()
            .all(|row| row.get(0) == Some(group_id.as_str()))
    );

    let mut json_bytes = Vec::new();
    dedupe_report::write_json(std::slice::from_ref(&group), &mut json_bytes)?;
    let json_groups: Vec<DuplicateGroup> = serde_json::from_slice(&json_bytes)?;
    assert_eq!(json_groups.len(), 1);
    assert_eq!(json_groups[0].members.len(), csv_rows.len());
    assert_eq!(json_groups[0].size_bytes, group.size_bytes);

    let mut html_bytes = Vec::new();
    dedupe_report::write_html(std::slice::from_ref(&group), &mut html_bytes)?;
    let html = String::from_utf8(html_bytes)?;
    assert_eq!(html.matches("<tr class=").count(), csv_rows.len());
    assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;.pdf"));
    assert!(html.contains("&lt;script&gt;alert(2)&lt;/script&gt;"));
    assert!(!html.contains("<script>"));

    for report in [csv_bytes, json_bytes, html.into_bytes()] {
        assert!(!String::from_utf8_lossy(&report).contains(secret_document_content));
    }
    Ok(())
}

fn fixture_group() -> DuplicateGroup {
    let digest = vec![0x5a; 32];
    DuplicateGroup {
        id: Uuid::new_v4(),
        mode: ComparisonMode::Strict,
        size_bytes: 4096,
        normalized_name: Some("report.pdf".into()),
        blake3: digest.clone(),
        sha256: digest.clone(),
        members: vec![
            member(
                "C:/Books/<img src=x onerror=alert(1)>.pdf",
                "keep </td><script>alert(2)</script>",
                MemberAction::Keep,
                1,
                &digest,
            ),
            member(
                "D:/Archive/report.pdf",
                "independent verified copy",
                MemberAction::Quarantine,
                2,
                &digest,
            ),
        ],
    }
}

fn member(
    path: &str,
    reason: &str,
    action: MemberAction,
    identity: u8,
    digest: &[u8],
) -> DuplicateMember {
    let token = [identity; 32];
    let hash = |algorithm| HashResult {
        algorithm,
        digest: digest.to_vec(),
        bytes_read: 4096,
        snapshot_before: token,
        snapshot_after: token,
        stable: true,
    };
    DuplicateMember {
        file: ProvenFile {
            metadata: FileMetadataSnapshot {
                path: PathBuf::from(path),
                normalized_path: path.to_lowercase(),
                normalized_name: "report.pdf".into(),
                extension: Some("pdf".into()),
                size_bytes: 4096,
                created_ns: Some(1),
                modified_ns: 2,
                identity: Some(FileIdentity {
                    volume_id: "volume".into(),
                    file_id: identity.to_string(),
                }),
                link_kind: LinkKind::Regular,
                hardlink_count: Some(1),
                access_status: AccessStatus::Readable,
                snapshot_token: token,
            },
            blake3: hash(HashAlgorithm::Blake3),
            sha256: hash(HashAlgorithm::Sha256),
        },
        action,
        reason: reason.into(),
    }
}
