//! Black-box CLI output, safety acknowledgement, and exit-code contracts.

use std::{path::Path, process::Command};

use dedupe_core::model::ComparisonMode;
use dedupe_store::{Database, ProjectRepository, ScanRepository};

fn cli(database: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_safe-dedupe"));
    command.arg("--database").arg(database);
    command
}

#[test]
fn project_and_scan_commands_emit_machine_readable_contracts()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let database = temporary.path().join("state.db");
    let source = temporary.path().join("source");
    std::fs::create_dir(&source)?;
    std::fs::write(source.join("one.txt"), b"one")?;

    let created = cli(&database)
        .args(["project", "create", "--name", "CLI contract"])
        .output()?;
    assert!(created.status.success());
    let created_stdout = String::from_utf8(created.stdout)?;
    let project_id = created_stdout
        .split_whitespace()
        .last()
        .ok_or("project create returned no UUID")?;
    uuid::Uuid::parse_str(project_id)?;

    let configured = cli(&database)
        .args([
            "project",
            "set-workers",
            "--project",
            project_id,
            "--workers",
            "3",
        ])
        .output()?;
    assert!(configured.status.success());

    let listed = cli(&database)
        .args(["project", "list", "--json"])
        .output()?;
    assert!(listed.status.success());
    let projects: serde_json::Value = serde_json::from_slice(&listed.stdout)?;
    assert_eq!(projects[0]["id"], project_id);
    assert_eq!(projects[0]["mode"], "strict");
    assert_eq!(projects[0]["worker_limit"], 3);

    let invalid_workers = cli(&database)
        .args([
            "project",
            "set-workers",
            "--project",
            project_id,
            "--workers",
            "0",
        ])
        .output()?;
    assert_eq!(invalid_workers.status.code(), Some(2));

    let root_path = source.to_string_lossy().into_owned();
    let root = cli(&database)
        .args([
            "project",
            "add-root",
            "--project",
            project_id,
            "--path",
            &root_path,
            "--primary",
        ])
        .output()?;
    assert!(root.status.success());

    let scan = cli(&database)
        .args(["scan", "start", "--project", project_id, "--all-files"])
        .output()?;
    assert!(scan.status.success());
    let scan_json: serde_json::Value = serde_json::from_slice(&scan.stdout)?;
    assert_eq!(scan_json["discovered_files"], 1);
    assert_eq!(scan_json["processed_files"], 1);
    assert_eq!(scan_json["errors"], 0);

    let backup = temporary.path().join("backup.db");
    let backup_path = backup.to_string_lossy().into_owned();
    let backed_up = cli(&database)
        .args(["backup", "--destination", &backup_path])
        .output()?;
    assert!(backed_up.status.success());
    assert!(backup.exists());
    let no_overwrite = cli(&database)
        .args(["backup", "--destination", &backup_path])
        .output()?;
    assert_eq!(no_overwrite.status.code(), Some(10));
    Ok(())
}

#[test]
fn content_mode_and_mutations_fail_with_documented_safety_code()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let database = temporary.path().join("state.db");
    let source = temporary.path().join("source");
    std::fs::create_dir(&source)?;

    let created = cli(&database)
        .args([
            "project",
            "create",
            "--name",
            "Safety codes",
            "--mode",
            "content",
        ])
        .output()?;
    assert!(created.status.success());
    let created_stdout = String::from_utf8(created.stdout)?;
    let project_id = created_stdout
        .split_whitespace()
        .last()
        .ok_or("project create returned no UUID")?;
    let root_path = source.to_string_lossy().into_owned();
    assert!(
        cli(&database)
            .args([
                "project",
                "add-root",
                "--project",
                project_id,
                "--path",
                &root_path,
            ])
            .status()?
            .success()
    );

    let unacknowledged = cli(&database)
        .args(["scan", "start", "--project", project_id])
        .output()?;
    assert_eq!(unacknowledged.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&unacknowledged.stderr).contains("acknowledge-content-mode"));

    let wrong_confirmation = cli(&database)
        .args([
            "restore",
            "--entry",
            "00000000-0000-0000-0000-000000000001",
            "--confirm",
            "restore",
        ])
        .output()?;
    assert_eq!(wrong_confirmation.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&wrong_confirmation.stderr).contains("chính xác là RESTORE"));
    Ok(())
}

#[test]
fn malformed_identifiers_use_invalid_input_exit_code()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let output = cli(&temporary.path().join("state.db"))
        .args(["results", "list", "--session", "not-a-uuid"])
        .output()?;
    assert_eq!(output.status.code(), Some(2));
    Ok(())
}

#[test]
fn scan_control_commands_persist_cross_process_requests()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("control.db");
    let database = Database::open(&path, &[])?;
    let project =
        ProjectRepository::new(database.clone()).create("CLI control", ComparisonMode::Strict)?;
    let session = ScanRepository::new(database).create_session(project, ComparisonMode::Strict)?;
    let session_text = session.to_string();

    let initial = cli(&path)
        .args(["scan", "status", "--session", &session_text, "--json"])
        .output()?;
    assert!(initial.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&initial.stdout)?["state"],
        "enumerating"
    );
    assert!(
        cli(&path)
            .args(["scan", "pause", "--session", &session_text])
            .status()?
            .success()
    );
    assert!(
        cli(&path)
            .args(["scan", "resume", "--session", &session_text])
            .status()?
            .success()
    );
    assert!(
        cli(&path)
            .args(["scan", "cancel", "--session", &session_text])
            .status()?
            .success()
    );
    let final_status = cli(&path)
        .args(["scan", "status", "--session", &session_text, "--json"])
        .output()?;
    assert!(final_status.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&final_status.stdout)?["state"],
        "cancelling"
    );
    Ok(())
}

#[test]
fn permanent_delete_cli_is_quarantine_scoped_and_rejects_path_inputs()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let database = temporary.path().join("delete-contract.db");
    let help = cli(&database).args(["quarantine", "--help"]).output()?;
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout)?;
    assert!(help.contains("delete-prepare"));
    assert!(help.contains("delete-execute"));

    let source_path = cli(&database)
        .args([
            "quarantine",
            "delete-prepare",
            "--entry",
            r"D:\Source\document.pdf",
        ])
        .output()?;
    assert_eq!(source_path.status.code(), Some(2));
    assert!(!source_path.status.success());

    let unknown_entry = cli(&database)
        .args([
            "quarantine",
            "delete-prepare",
            "--entry",
            "00000000-0000-0000-0000-000000000001",
        ])
        .output()?;
    assert_eq!(unknown_entry.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&unknown_entry.stderr).contains("chưa được xác minh"));
    Ok(())
}
