//! Black-box CLI versus reusable-core classification conformance.

use std::{path::Path, process::Command};

use dedupe_core::{
    control::ControlToken,
    duplicate_detector::confirm_preliminary_group,
    keep_policy,
    model::{ComparisonMode, DuplicateGroup, KeepPolicy},
    ports::MetadataProvider,
};
use dedupe_platform::PlatformFileSystem;

type GroupSignature = (u64, String, String, String, Vec<(String, String)>);

fn cli(database: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_safe-dedupe"));
    command.arg("--database").arg(database);
    command
}

#[test]
fn cli_and_core_produce_identical_proven_classification_on_repeated_scans()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let database = temporary.path().join("state.db");
    let root = temporary.path().join("source");
    let first = root.join("primary/book.pdf");
    let second = root.join("copy/book.pdf");
    let different = root.join("different/book.pdf");
    std::fs::create_dir_all(first.parent().ok_or("first fixture has no parent")?)?;
    std::fs::create_dir_all(second.parent().ok_or("second fixture has no parent")?)?;
    std::fs::create_dir_all(
        different
            .parent()
            .ok_or("different fixture has no parent")?,
    )?;
    std::fs::write(&first, b"identical-payload")?;
    std::fs::write(&second, b"identical-payload")?;
    std::fs::write(&different, b"identical-payloae")?;

    let created = cli(&database)
        .args(["project", "create", "--name", "Conformance"])
        .output()?;
    assert!(created.status.success());
    let project = String::from_utf8(created.stdout)?
        .split_whitespace()
        .last()
        .ok_or("project create returned no UUID")?
        .to_owned();
    let root_text = root.to_string_lossy().into_owned();
    assert!(
        cli(&database)
            .args([
                "project",
                "add-root",
                "--project",
                &project,
                "--path",
                &root_text,
                "--primary",
            ])
            .status()?
            .success()
    );

    let first_cli = scan_groups(&database, &project)?;
    let second_cli = scan_groups(&database, &project)?;
    let provider = PlatformFileSystem;
    let candidates = [&first, &second, &different]
        .into_iter()
        .map(|path| provider.snapshot(path))
        .collect::<dedupe_core::Result<Vec<_>>>()?;
    let mut core = confirm_preliminary_group(
        ComparisonMode::Strict,
        &candidates,
        &provider,
        &ControlToken::new(),
    )?;
    for group in &mut core {
        keep_policy::apply(
            group,
            &KeepPolicy::Default {
                primary_roots: vec![root.clone()],
            },
        )?;
    }

    assert_eq!(signatures(&first_cli), signatures(&core));
    assert_eq!(signatures(&second_cli), signatures(&core));
    assert_eq!(core.len(), 1);
    assert_eq!(core[0].members.len(), 2);
    assert_eq!(std::fs::read(&first)?, b"identical-payload");
    assert_eq!(std::fs::read(&second)?, b"identical-payload");
    assert_eq!(std::fs::read(&different)?, b"identical-payloae");
    Ok(())
}

fn scan_groups(
    database: &Path,
    project: &str,
) -> std::result::Result<Vec<DuplicateGroup>, Box<dyn std::error::Error>> {
    let scan = cli(database)
        .args(["scan", "start", "--project", project, "--all-files"])
        .output()?;
    assert!(scan.status.success());
    let scan_json: serde_json::Value = serde_json::from_slice(&scan.stdout)?;
    let session = scan_json["session_id"]
        .as_str()
        .ok_or("scan output has no session id")?;
    let results = cli(database)
        .args(["results", "list", "--session", session, "--json"])
        .output()?;
    assert!(results.status.success());
    Ok(serde_json::from_slice(&results.stdout)?)
}

fn signatures(groups: &[DuplicateGroup]) -> Vec<GroupSignature> {
    let mut signatures = groups
        .iter()
        .map(|group| {
            let mut members = group
                .members
                .iter()
                .map(|member| {
                    (
                        member.file.metadata.normalized_path.clone(),
                        format!("{:?}", member.action),
                    )
                })
                .collect::<Vec<_>>();
            members.sort();
            (
                group.size_bytes,
                group.normalized_name.clone().unwrap_or_default(),
                hex::encode(&group.blake3),
                hex::encode(&group.sha256),
                members,
            )
        })
        .collect::<Vec<_>>();
    signatures.sort();
    signatures
}
