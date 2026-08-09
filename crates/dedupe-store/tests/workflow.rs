//! Durable end-to-end workflow against real `SQLite` and the host filesystem adapter.

use std::{sync::Arc, thread, time::Duration};

use dedupe_core::{
    DedupeError,
    control::ControlToken,
    duplicate_detector::confirm_preliminary_group,
    filters::{CompiledFilter, FilterConfig},
    full_hash, keep_policy,
    model::{ComparisonMode, KeepPolicy, ProvenFile, TransactionState},
    ports::{MetadataProvider, TransactionJournal},
    progress::ProgressCounters,
    quarantine,
    scanner::scan_roots,
};
use dedupe_platform::PlatformFileSystem;
use dedupe_store::{
    Database, DuplicateRepository, PlanRepository, ProjectRepository, ScanControlMonitor,
    ScanControlRequest, ScanRepository, SqliteScanSink, SqliteTransactionJournal,
    TransactionRepository,
};
use dedupe_testkit::Fixture;
use uuid::Uuid;

#[test]
fn wal_schema_defaults_are_safety_first() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let database = Database::open(&fixture.path().join("state.db"), &[])?;
    let projects = ProjectRepository::new(database.clone());
    let project = projects.create("Safety defaults", ComparisonMode::Strict)?;
    assert_eq!(projects.worker_config(project)?.metadata_workers, 4);
    assert_eq!(
        projects
            .worker_config(project)?
            .full_hash_workers_per_volume,
        1
    );
    projects.set_worker_limit(project, 7)?;
    assert_eq!(projects.worker_config(project)?.metadata_workers, 7);
    assert!(matches!(
        projects.set_worker_limit(project, 0),
        Err(DedupeError::InvalidInput(_))
    ));
    assert_eq!(projects.worker_config(project)?.metadata_workers, 7);
    let connection = database.connection();
    let journal_mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    let synchronous: i64 = connection.query_row("PRAGMA synchronous", [], |row| row.get(0))?;
    let foreign_keys: i64 = connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
    let automatic_delete: i64 = connection.query_row(
        "SELECT automatic_permanent_delete FROM projects WHERE id=?1",
        [project.to_string()],
        |row| row.get(0),
    )?;

    assert_eq!(journal_mode.to_lowercase(), "wal");
    assert_eq!(synchronous, 2);
    assert_eq!(foreign_keys, 1);
    assert_eq!(automatic_delete, 0);
    let migration_version: i64 =
        connection.query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })?;
    assert_eq!(migration_version, 6);
    let event_id = Uuid::new_v4().to_string();
    connection.execute(
        "INSERT INTO audit_events (event_id,project_id,actor,event_type,payload_json,occurred_at)
         VALUES (?1,?2,'system','schema_test','{}',?3)",
        rusqlite::params![
            event_id,
            project.to_string(),
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    assert!(
        connection
            .execute(
                "UPDATE audit_events SET event_type='rewritten' WHERE event_id=?1",
                [&event_id],
            )
            .is_err()
    );
    assert!(
        connection
            .execute("DELETE FROM audit_events WHERE event_id=?1", [&event_id])
            .is_err()
    );
    let audit_rows: i64 = connection.query_row(
        "SELECT COUNT(*) FROM audit_events WHERE event_id=?1",
        [&event_id],
        |row| row.get(0),
    )?;
    assert_eq!(audit_rows, 1);
    Ok(())
}

#[test]
fn version_one_database_is_migrated_through_permanent_delete_schema()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("version-one.db");
    let connection = rusqlite::Connection::open(&path)?;
    connection.execute_batch(include_str!("../migrations/0001_initial.sql"))?;
    drop(connection);

    let database = Database::open(&path, &[])?;
    let version: i64 = database.connection().query_row(
        "SELECT MAX(version) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    let control_columns: i64 = database.connection().query_row(
        "SELECT COUNT(*) FROM pragma_table_info('scan_sessions')
         WHERE name IN ('control_request','resume_state')",
        [],
        |row| row.get(0),
    )?;
    let delete_columns: i64 = database.connection().query_row(
        "SELECT COUNT(*) FROM pragma_table_info('quarantine_entries')
         WHERE name IN ('permanent_delete_state','permanent_delete_batch_id','deleted_at')",
        [],
        |row| row.get(0),
    )?;
    let delete_mode_columns: i64 = database.connection().query_row(
        "SELECT COUNT(*) FROM pragma_table_info('permanent_delete_batches')
         WHERE name='deletion_mode'",
        [],
        |row| row.get(0),
    )?;
    let block_reason_columns: i64 = database.connection().query_row(
        "SELECT COUNT(*) FROM pragma_table_info('scan_sessions')
         WHERE name='blocked_reason'",
        [],
        |row| row.get(0),
    )?;
    let history_indexes: i64 = database.connection().query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index'
         AND name IN ('idx_duplicate_members_snapshot','idx_operation_plans_session_created',
                      'idx_plan_items_snapshot','idx_file_transactions_plan_item')",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(version, 6);
    assert_eq!(control_columns, 2);
    assert_eq!(delete_columns, 3);
    assert_eq!(delete_mode_columns, 1);
    assert_eq!(block_reason_columns, 1);
    assert_eq!(history_indexes, 4);
    Ok(())
}

#[test]
fn durable_scan_control_preserves_stage_across_pause_resume_and_cancel()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let database = Database::open(&fixture.path().join("control.db"), &[])?;
    let project =
        ProjectRepository::new(database.clone()).create("Control", ComparisonMode::Strict)?;
    let scans = ScanRepository::new(database.clone());
    let session = scans.create_session(project, ComparisonMode::Strict)?;
    let control = ControlToken::new();
    let progress = Arc::new(ProgressCounters::default());
    let monitor =
        ScanControlMonitor::start(database, session, control.clone(), Arc::clone(&progress));

    scans.request_control(session, ScanControlRequest::Pause)?;
    wait_for_scan_state(&scans, session, "paused")?;
    scans.set_state(session, "quick_hashing")?;
    assert_eq!(scans.status(session)?.state, "paused");

    let worker_control = control.clone();
    let worker = thread::spawn(move || worker_control.checkpoint());
    thread::sleep(Duration::from_millis(30));
    assert!(!worker.is_finished());
    scans.request_control(session, ScanControlRequest::Resume)?;
    wait_for_scan_state(&scans, session, "quick_hashing")?;
    assert!(worker.join().is_ok_and(|result| result.is_ok()));

    scans.request_control(session, ScanControlRequest::Cancel)?;
    for _ in 0..100 {
        if control.is_cancelled() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(control.is_cancelled());
    assert!(control.checkpoint().is_err());
    scans.set_state(session, "cancelled")?;
    monitor.finish()?;
    assert_eq!(scans.status(session)?.state, "cancelled");
    Ok(())
}

fn wait_for_scan_state(
    scans: &ScanRepository,
    session: Uuid,
    expected: &str,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    for _ in 0..100 {
        if scans.status(session)?.state == expected {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(format!("scan {session} did not reach {expected}").into())
}

#[test]
fn database_backup_is_consistent_and_never_overwrites()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let live_path = fixture.path().join("state.db");
    let backup_path = fixture.path().join("backups/state-backup.db");
    let database = Database::open(&live_path, &[])?;
    let project =
        ProjectRepository::new(database.clone()).create("Backed up", ComparisonMode::Strict)?;

    assert_eq!(database.backup_to(&backup_path)?, backup_path);
    let backup = Database::open(&backup_path, &[])?;
    let projects = ProjectRepository::new(backup).list()?;
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].id, project);
    assert!(database.backup_to(&backup_path).is_err());
    Ok(())
}

#[test]
fn project_roots_reject_parent_child_overlap() -> std::result::Result<(), Box<dyn std::error::Error>>
{
    let fixture = Fixture::new()?;
    let parent = fixture.path().join("library");
    let child = parent.join("nested");
    std::fs::create_dir_all(&child)?;
    let database = Database::open(&fixture.path().join("state.db"), &[])?;
    let projects = ProjectRepository::new(database);
    let project = projects.create("Root overlap", ComparisonMode::Strict)?;
    projects.add_root(project, &parent, true)?;

    let result = projects.add_root(project, &child, false);

    assert!(result.is_err());
    assert_eq!(projects.roots(project)?.len(), 1);
    Ok(())
}

#[test]
fn project_filters_are_validated_atomically_and_survive_reopen()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let database_path = fixture.path().join("state.db");
    let database = Database::open(&database_path, &[])?;
    let projects = ProjectRepository::new(database.clone());
    let project = projects.create("Persistent filters", ComparisonMode::Strict)?;
    let persisted_root = fixture.path().join("persisted-root");
    std::fs::create_dir_all(&persisted_root)?;
    let root_id = projects.add_root(project, &persisted_root, true)?;
    let expected = FilterConfig {
        include_extensions: vec!["epub".into(), "pdf".into()],
        exclude_extensions: vec!["partial".into()],
        exclude_globs: vec!["**/cache/**".into()],
        minimum_size: 4096,
        skip_hidden: false,
        skip_system: true,
    };
    projects.replace_filter_config(project, &expected)?;
    assert_eq!(projects.filter_config(project)?, expected);

    let invalid = FilterConfig {
        exclude_globs: vec!["[invalid".into()],
        ..expected.clone()
    };
    assert!(projects.replace_filter_config(project, &invalid).is_err());
    assert_eq!(projects.filter_config(project)?, expected);
    drop(projects);
    drop(database);

    let reopened = Database::open(&database_path, &[])?;
    let reopened_projects = ProjectRepository::new(reopened);
    assert_eq!(reopened_projects.filter_config(project)?, expected);
    assert_eq!(
        reopened_projects.roots(project)?,
        vec![(root_id, persisted_root, true)]
    );
    Ok(())
}

#[test]
fn project_update_archive_and_root_removal_never_touch_source_data()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source_root = fixture.path().join("configured-source");
    let source_file = fixture.write("configured-source/keep.pdf", b"never delete me")?;
    let database = Database::open(&fixture.path().join("state.db"), &[])?;
    let projects = ProjectRepository::new(database);
    let project = projects.create("Original", ComparisonMode::Strict)?;
    let root = projects.add_root(project, &source_root, true)?;

    projects.update(project, "Renamed", ComparisonMode::Content)?;
    let record = projects
        .list()?
        .into_iter()
        .find(|record| record.id == project)
        .ok_or("updated project missing")?;
    assert_eq!(record.name, "Renamed");
    assert_eq!(record.mode, ComparisonMode::Content);
    projects.remove_root(project, root)?;
    assert!(projects.root_records(project)?.is_empty());
    assert_eq!(std::fs::read(&source_file)?, b"never delete me");

    projects.archive(project)?;
    projects.archive(project)?;
    let record = projects
        .list()?
        .into_iter()
        .find(|record| record.id == project)
        .ok_or("archived project missing")?;
    assert_eq!(record.status, "archived");
    assert!(projects.add_root(project, &source_root, false).is_err());
    assert_eq!(std::fs::read(source_file)?, b"never delete me");
    Ok(())
}

#[test]
fn manifest_failure_blocks_state_advance_before_filesystem_mutation()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source = fixture.write("manifest/source.pdf", b"manifest must be durable first")?;
    let destination = fixture.path().join("manifest-quarantine/source.pdf");
    let database = Database::open(&fixture.path().join("state.db"), &[])?;
    let project = ProjectRepository::new(database.clone())
        .create("Manifest ordering", ComparisonMode::Strict)?;
    let provider = PlatformFileSystem;
    let control = ControlToken::new();
    let proven = ProvenFile {
        metadata: provider.snapshot(&source)?,
        blake3: full_hash::blake3_file(&source, &provider, &control)?,
        sha256: full_hash::sha256_file(&source, &provider, &control)?,
    };
    let mut transaction = quarantine::planned_transaction(
        project,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &proven,
        destination.clone(),
    )?;
    transaction.session_id = None;
    transaction.plan_item_id = None;
    let manifest = fixture.path().join("transactions.jsonl");
    let journal = SqliteTransactionJournal::new(database.clone(), &manifest)?;
    journal.create(&transaction)?;
    std::fs::remove_file(&manifest)?;
    std::fs::create_dir(&manifest)?;

    let result = dedupe_core::transaction_journal::transition(
        &mut transaction,
        TransactionState::PreflightValidated,
        Some("test preflight"),
        None,
        &journal,
    );

    assert!(result.is_err());
    assert_eq!(transaction.state, TransactionState::Planned);
    assert!(source.exists());
    assert!(!destination.exists());
    let status: String = database.connection().query_row(
        "SELECT status FROM file_transactions WHERE id=?1",
        [transaction.id.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(status, "planned");
    Ok(())
}

#[test]
fn sqlite_commit_failure_keeps_projection_planned_and_source_untouched()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source = fixture.write("commit-failure/source.pdf", b"database commit must succeed")?;
    let destination = fixture.path().join("commit-failure-quarantine/source.pdf");
    let database = Database::open(&fixture.path().join("commit-failure.db"), &[])?;
    let project = ProjectRepository::new(database.clone())
        .create("Commit failure", ComparisonMode::Strict)?;
    let provider = PlatformFileSystem;
    let control = ControlToken::new();
    let proven = ProvenFile {
        metadata: provider.snapshot(&source)?,
        blake3: full_hash::blake3_file(&source, &provider, &control)?,
        sha256: full_hash::sha256_file(&source, &provider, &control)?,
    };
    let mut transaction = quarantine::planned_transaction(
        project,
        Uuid::new_v4(),
        Uuid::new_v4(),
        &proven,
        destination.clone(),
    )?;
    transaction.session_id = None;
    transaction.plan_item_id = None;
    let manifest = fixture.path().join("commit-failure.jsonl");
    let journal = SqliteTransactionJournal::new(database.clone(), &manifest)?;
    journal.create(&transaction)?;
    database.connection().execute_batch(
        "CREATE TEMP TRIGGER injected_transaction_commit_failure
         BEFORE UPDATE ON file_transactions
         BEGIN SELECT RAISE(ABORT, 'injected transaction commit failure'); END;",
    )?;

    let result = dedupe_core::transaction_journal::transition(
        &mut transaction,
        TransactionState::PreflightValidated,
        Some("preflight evidence"),
        None,
        &journal,
    );

    assert!(matches!(result, Err(DedupeError::Durability(_))));
    assert_eq!(transaction.state, TransactionState::Planned);
    assert!(source.exists());
    assert!(!destination.exists());
    let status: String = database.connection().query_row(
        "SELECT status FROM file_transactions WHERE id=?1",
        [transaction.id.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(status, "planned");
    assert!(manifest.metadata()?.len() > 0);
    Ok(())
}

#[test]
fn interrupted_scan_resets_only_read_only_evidence_for_safe_resume()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source_root = fixture.path().join("resume-source");
    fixture.write("resume-source/document.txt", b"resume evidence")?;
    let database = Database::open(&fixture.path().join("state.db"), &[])?;
    let projects = ProjectRepository::new(database.clone());
    let project = projects.create("Resume", ComparisonMode::Strict)?;
    let root_id = projects.add_root(project, &source_root, true)?;
    let scans = ScanRepository::new(database.clone());
    let session = scans.create_session_with_config(project, ComparisonMode::Strict, true)?;
    let progress = ProgressCounters::default();
    let filter = CompiledFilter::new(FilterConfig {
        include_extensions: Vec::new(),
        ..FilterConfig::default()
    })?;
    let mut sink = SqliteScanSink::new(
        database.clone(),
        project,
        session,
        vec![(root_id, source_root.clone())],
    );
    scan_roots(
        std::slice::from_ref(&source_root),
        &filter,
        &PlatformFileSystem,
        &mut sink,
        &ControlToken::new(),
        &progress,
    )?;
    scans.update_progress(session, progress.snapshot())?;
    scans.checkpoint(session, "metadata_complete", 1)?;
    scans.set_state(session, "quick_hashing")?;

    assert_eq!(scans.mark_incomplete_interrupted()?, 1);
    scans.block_session(session, "lỗi ghi lô có ngữ cảnh")?;
    let blocked = scans.status(session)?;
    assert_eq!(blocked.state, "blocked");
    assert_eq!(
        blocked.blocked_reason.as_deref(),
        Some("lỗi ghi lô có ngữ cảnh")
    );
    let spec = scans.resume_spec(session)?;
    assert_eq!(spec.session_id, session);
    assert_eq!(spec.project_id, project);
    assert_eq!(spec.mode, ComparisonMode::Strict);
    assert!(spec.all_files);
    scans.prepare_resume(session)?;

    let status = scans.status(session)?;
    assert_eq!(status.state, "enumerating");
    assert_eq!(status.discovered_files, 0);
    assert_eq!(status.processed_files, 0);
    assert_eq!(status.blocked_reason, None);
    let connection = database.connection();
    let snapshots: i64 = connection.query_row(
        "SELECT COUNT(*) FROM file_snapshots WHERE session_id=?1",
        [session.to_string()],
        |row| row.get(0),
    )?;
    let checkpoints: i64 = connection.query_row(
        "SELECT COUNT(*) FROM scan_checkpoints WHERE session_id=?1",
        [session.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(snapshots, 0);
    assert_eq!(checkpoints, 0);
    assert!(source_root.join("document.txt").exists());
    Ok(())
}

#[test]
fn abrupt_reopen_invalidates_stale_identity_size_time_and_hash_evidence()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let database_path = fixture.path().join("abrupt-resume.db");
    let source_root = fixture.path().join("abrupt-source");
    fixture.write("abrupt-source/a/document.pdf", b"original evidence")?;
    let changed = fixture.write("abrupt-source/b/document.pdf", b"original evidence")?;
    let (project, session) = {
        let database = Database::open(&database_path, &[])?;
        let projects = ProjectRepository::new(database.clone());
        let project = projects.create("Abrupt resume", ComparisonMode::Strict)?;
        let root_id = projects.add_root(project, &source_root, true)?;
        let scans = ScanRepository::new(database.clone());
        let session = scans.create_session_with_config(project, ComparisonMode::Strict, true)?;
        let provider = PlatformFileSystem;
        let control = ControlToken::new();
        let progress = ProgressCounters::default();
        let mut sink = SqliteScanSink::new(
            database.clone(),
            project,
            session,
            vec![(root_id, source_root.clone())],
        );
        scan_roots(
            std::slice::from_ref(&source_root),
            &CompiledFilter::new(FilterConfig::default())?,
            &provider,
            &mut sink,
            &control,
            &progress,
        )?;
        let mut groups = Vec::new();
        scans.for_each_candidate_group(session, ComparisonMode::Strict, |candidates| {
            groups.extend(confirm_preliminary_group(
                ComparisonMode::Strict,
                &candidates,
                &provider,
                &control,
            )?);
            Ok(())
        })?;
        assert_eq!(groups.len(), 1);
        keep_policy::apply(
            &mut groups[0],
            &KeepPolicy::Default {
                primary_roots: vec![source_root.clone()],
            },
        )?;
        DuplicateRepository::new(database).replace_session_groups(session, &groups)?;
        scans.checkpoint(session, "hashing_complete", 2)?;
        scans.set_state(session, "grouping")?;
        (project, session)
    };

    std::fs::write(&changed, b"changed size and content after abrupt stop")?;
    let reopened = Database::open(&database_path, &[])?;
    let scans = ScanRepository::new(reopened.clone());
    assert_eq!(scans.mark_incomplete_interrupted()?, 1);
    let spec = scans.resume_spec(session)?;
    assert_eq!(spec.project_id, project);
    scans.prepare_resume(session)?;
    let connection = reopened.connection();
    for (label, sql) in [
        (
            "file_snapshots",
            "SELECT COUNT(*) FROM file_snapshots WHERE session_id=?1",
        ),
        (
            "hash_results",
            "SELECT COUNT(*) FROM hash_results WHERE ?1 IS NOT NULL",
        ),
        (
            "duplicate_groups",
            "SELECT COUNT(*) FROM duplicate_groups WHERE session_id=?1",
        ),
        (
            "scan_checkpoints",
            "SELECT COUNT(*) FROM scan_checkpoints WHERE session_id=?1",
        ),
    ] {
        let count: i64 = connection.query_row(sql, [session.to_string()], |row| row.get(0))?;
        assert_eq!(count, 0, "stale rows remained in {label}");
    }
    assert_eq!(
        std::fs::read(changed)?,
        b"changed size and content after abrupt stop"
    );
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn group_session_and_project_batch_restore_are_idempotent()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source_root = fixture.path().join("batch-source");
    let first = fixture.write("batch-source/a/book.pdf", b"batch restore payload")?;
    let second = fixture.write("batch-source/b/book.pdf", b"batch restore payload")?;
    let third = fixture.write("batch-source/c/book.pdf", b"batch restore payload")?;
    let database = Database::open(&fixture.path().join("batch.db"), &[])?;
    let projects = ProjectRepository::new(database.clone());
    let project = projects.create("Batch restore", ComparisonMode::Strict)?;
    let root_id = projects.add_root(project, &source_root, true)?;
    let scans = ScanRepository::new(database.clone());
    let session = scans.create_session(project, ComparisonMode::Strict)?;
    let provider = PlatformFileSystem;
    let control = ControlToken::new();
    let progress = ProgressCounters::default();
    let mut sink = SqliteScanSink::new(
        database.clone(),
        project,
        session,
        vec![(root_id, source_root.clone())],
    );
    let snapshot = scan_roots(
        std::slice::from_ref(&source_root),
        &CompiledFilter::new(FilterConfig::default())?,
        &provider,
        &mut sink,
        &control,
        &progress,
    )?;
    let mut groups = Vec::new();
    scans.for_each_candidate_group(session, ComparisonMode::Strict, |candidates| {
        groups.extend(confirm_preliminary_group(
            ComparisonMode::Strict,
            &candidates,
            &provider,
            &control,
        )?);
        Ok(())
    })?;
    assert_eq!(groups.len(), 1);
    keep_policy::apply(
        &mut groups[0],
        &KeepPolicy::Default {
            primary_roots: vec![source_root.clone()],
        },
    )?;
    let group_id = groups[0].id;
    DuplicateRepository::new(database.clone()).replace_session_groups(session, &groups)?;
    scans.complete_session(session, snapshot)?;
    let plans = PlanRepository::new(database.clone());
    let plan = plans.create_and_seal(
        session,
        &KeepPolicy::Default {
            primary_roots: vec![source_root.clone()],
        },
        &groups,
    )?;
    let items = plans.quarantine_items(plan)?;
    assert_eq!(items.len(), 2);
    plans.mark_executing(plan)?;
    let journal = SqliteTransactionJournal::new(
        database.clone(),
        fixture.path().join("batch-transactions.jsonl"),
    )?;
    for item in items {
        let destination = quarantine::quarantine_destination(
            &source_root.join(".safe-duplicate-finder-quarantine"),
            project,
            session,
            item.plan_item_id,
            &source_root,
            &item.file.metadata.path,
        )?;
        let mut transaction = quarantine::planned_transaction(
            project,
            session,
            item.plan_item_id,
            &item.file,
            destination,
        )?;
        quarantine::execute(&mut transaction, &provider, &provider, &journal, &control)?;
    }
    assert!(plans.mark_completed_if_verified(plan)?);

    let transactions = TransactionRepository::new(database);
    assert_eq!(transactions.verified_entries_for_group(group_id)?.len(), 2);
    assert_eq!(transactions.verified_entries_for_session(session)?.len(), 2);
    assert_eq!(transactions.verified_entries_for_project(project)?.len(), 2);
    for entry in transactions.verified_entries_for_session(session)? {
        let origin = transactions.verified_quarantine_transaction(entry)?;
        let mut restore = dedupe_core::restore::planned_transaction(&origin)?;
        dedupe_core::restore::execute(&mut restore, &provider, &provider, &journal, &control)?;
    }
    assert!(
        transactions
            .verified_entries_for_group(group_id)?
            .is_empty()
    );
    assert!(
        transactions
            .verified_entries_for_session(session)?
            .is_empty()
    );
    assert!(
        transactions
            .verified_entries_for_project(project)?
            .is_empty()
    );
    assert!(first.exists() && second.exists() && third.exists());
    Ok(())
}

#[test]
#[allow(clippy::too_many_lines)]
fn proven_plan_quarantine_and_restore_are_durable()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let source_root = fixture.path().join("source");
    let first = fixture.write("source/a/book.pdf", b"identical payload")?;
    let second = fixture.write("source/b/book.pdf", b"identical payload")?;
    let database = Database::open(&fixture.path().join("state.db"), &[])?;
    let projects = ProjectRepository::new(database.clone());
    let project = projects.create("Workflow", ComparisonMode::Strict)?;
    let root_id = projects.add_root(project, &source_root, true)?;
    let scans = ScanRepository::new(database.clone());
    let session = scans.create_session(project, ComparisonMode::Strict)?;
    let provider = PlatformFileSystem;
    let control = ControlToken::new();
    let progress = ProgressCounters::default();
    let filter = CompiledFilter::new(FilterConfig::default())?;
    let mut sink = SqliteScanSink::new(
        database.clone(),
        project,
        session,
        vec![(root_id, source_root.clone())],
    );
    let snapshot = scan_roots(
        std::slice::from_ref(&source_root),
        &filter,
        &provider,
        &mut sink,
        &control,
        &progress,
    )?;
    let mut groups = Vec::new();
    scans.for_each_candidate_group(session, ComparisonMode::Strict, |candidates| {
        groups.extend(confirm_preliminary_group(
            ComparisonMode::Strict,
            &candidates,
            &provider,
            &control,
        )?);
        Ok(())
    })?;
    assert_eq!(groups.len(), 1);
    keep_policy::apply(
        &mut groups[0],
        &KeepPolicy::Default {
            primary_roots: vec![source_root.clone()],
        },
    )?;
    let duplicates = DuplicateRepository::new(database.clone());
    duplicates.replace_session_groups(session, &groups)?;
    scans.complete_session(session, snapshot)?;
    assert!(
        projects
            .list()?
            .into_iter()
            .find(|record| record.id == project)
            .and_then(|record| record.last_scan_at)
            .is_some()
    );
    let reloaded = duplicates.load_session_groups(session)?;
    assert_eq!(reloaded.len(), 1);
    assert_eq!(reloaded[0].members.len(), 2);

    let plans = PlanRepository::new(database.clone());
    let plan = plans.create_and_seal(
        session,
        &KeepPolicy::Default {
            primary_roots: vec![source_root.clone()],
        },
        &reloaded,
    )?;
    let summary = plans.summary(plan)?;
    assert_eq!(summary.quarantine_files, 1);
    assert_eq!(summary.quarantine_bytes, b"identical payload".len() as u64);
    let items = plans.quarantine_items(plan)?;
    assert_eq!(items.len(), 1);
    plans.mark_executing(plan)?;

    let journal_path = fixture.path().join("transactions.jsonl");
    let journal = SqliteTransactionJournal::new(database.clone(), &journal_path)?;
    let item = &items[0];
    let destination = quarantine::quarantine_destination(
        &source_root.join(".safe-duplicate-finder-quarantine"),
        project,
        session,
        item.plan_item_id,
        &source_root,
        &item.file.metadata.path,
    )?;
    let mut transaction = quarantine::planned_transaction(
        project,
        session,
        item.plan_item_id,
        &item.file,
        destination,
    )?;
    quarantine::execute(&mut transaction, &provider, &provider, &journal, &control)?;
    assert_eq!(transaction.state, TransactionState::Verified);
    assert!(plans.mark_completed_if_verified(plan)?);
    assert!(journal_path.metadata()?.len() > 0);
    assert!(first.exists() ^ second.exists());

    let transactions = TransactionRepository::new(database.clone());
    let inventory = transactions.list_quarantine(project)?;
    assert_eq!(inventory.len(), 1);
    assert_eq!(inventory[0].state, "verified");
    let origin = transactions.verified_quarantine_transaction(inventory[0].id)?;
    let mut restore = dedupe_core::restore::planned_transaction(&origin)?;
    dedupe_core::restore::execute(&mut restore, &provider, &provider, &journal, &control)?;
    assert_eq!(restore.state, TransactionState::Verified);
    assert!(first.exists() && second.exists());
    assert_eq!(std::fs::read(first)?, b"identical payload");
    assert_eq!(std::fs::read(second)?, b"identical payload");
    assert_eq!(transactions.list_quarantine(project)?[0].state, "restored");
    let audit_events: i64 = database.connection().query_row(
        "SELECT COUNT(*) FROM audit_events WHERE project_id=?1 AND transaction_id IS NOT NULL",
        [project.to_string()],
        |row| row.get(0),
    )?;
    assert!(audit_events >= 10);
    Ok(())
}
