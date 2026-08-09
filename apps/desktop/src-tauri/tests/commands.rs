//! Tauri request-handler tests for validation order, idempotency, and no-auto-scan behavior.

use dedupe_store::Database;
use safe_dedupe_desktop::{commands, state::EngineState};

#[test]
fn configuration_commands_are_validated_idempotent_and_never_auto_scan()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let database = Database::open(&temporary.path().join("state.db"), &[])?;
    let logs = dedupe_core::logging::init(&temporary.path().join("logs"))?;
    let engine = EngineState::new(database.clone(), logs)?;

    let project_id = commands::create_project_request(&engine, "Desktop contract", "strict")?;
    assert_eq!(scan_session_count(&database)?, 0);
    assert!(engine.jobs.lock().is_empty());

    commands::set_project_workers_request(&engine, &project_id, 3)?;
    commands::set_project_workers_request(&engine, &project_id, 3)?;
    let source = temporary.path().join("source");
    std::fs::create_dir(&source)?;
    std::fs::write(source.join("document.pdf"), b"source remains read-only")?;
    commands::add_root_request(&engine, &project_id, &source.to_string_lossy(), true)?;
    assert_eq!(scan_session_count(&database)?, 0);
    assert!(engine.jobs.lock().is_empty());

    let unacknowledged = commands::start_scan_request(&engine, &project_id, "content", false, true);
    assert!(unacknowledged.is_err_and(|error| error.contains("xác nhận cảnh báo trước")));
    assert_eq!(scan_session_count(&database)?, 0);

    assert!(
        commands::archive_project_request(&engine, &project_id, "archive")
            .is_err_and(|error| error.contains("chính xác là ARCHIVE"))
    );
    assert!(
        commands::restore_entry_request(&engine, "not-a-uuid", "restore")
            .is_err_and(|error| error.contains("chính xác là RESTORE"))
    );
    assert!(
        commands::apply_quarantine_request(&engine, "not-a-uuid", "quarantine")
            .is_err_and(|error| error.contains("chính xác là QUARANTINE"))
    );
    assert!(
        commands::reconcile_transaction_request(&engine, "not-a-uuid", "reconcile")
            .is_err_and(|error| error.contains("chính xác là RECONCILE"))
    );
    assert!(
        commands::prepare_permanent_delete_request(
            &engine,
            &[source.to_string_lossy().into_owned()],
            false,
        )
        .is_err_and(|error| error.contains("Mã mục cách ly không hợp lệ"))
    );
    assert_history_and_maintenance_contract(&engine, &project_id)?;

    commands::archive_project_request(&engine, &project_id, "ARCHIVE")?;
    commands::archive_project_request(&engine, &project_id, "ARCHIVE")?;
    let archived = commands::list_projects_request(&engine)?
        .into_iter()
        .find(|project| project.id.to_string() == project_id)
        .ok_or("archived project was not returned")?;
    assert_eq!(archived.status, "archived");
    assert_eq!(
        std::fs::read(source.join("document.pdf"))?,
        b"source remains read-only"
    );
    assert_eq!(scan_session_count(&database)?, 0);
    Ok(())
}

fn assert_history_and_maintenance_contract(
    engine: &EngineState,
    project_id: &str,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let history = commands::list_file_history_request(engine, project_id, "", false, 0, 50)?;
    assert_eq!(history.total_processed, 0);
    assert!(history.items.is_empty());
    let storage = commands::storage_overview_request(engine)?;
    assert!(storage.database_bytes > 0);
    assert!(storage.total_bytes >= storage.database_bytes);
    let logs = commands::cleanup_old_logs_request(engine, 30)?;
    assert_eq!(logs.deleted_files, 0);
    assert_eq!(logs.reclaimed_bytes, 0);
    Ok(())
}

fn scan_session_count(database: &Database) -> std::result::Result<i64, Box<dyn std::error::Error>> {
    Ok(database
        .connection()
        .query_row("SELECT COUNT(*) FROM scan_sessions", [], |row| row.get(0))?)
}
