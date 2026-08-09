//! `SQLite` full/busy/rollback and batched-writer shutdown behavior.

use std::path::PathBuf;

use chrono::Utc;
use dedupe_core::{
    DedupeError,
    metadata::snapshot_token,
    model::{
        AccessStatus, ComparisonMode, DuplicateGroup, DuplicateMember, FileMetadataSnapshot,
        HashAlgorithm, HashResult, KeepPolicy, LinkKind, MemberAction, ProvenFile,
    },
    path_normalization::path_key,
    ports::ScanSink,
};
use dedupe_store::{
    Database, DuplicateRepository, HistoryRepository, PlanRepository, ProjectRepository,
    ScanRepository, SqliteScanSink,
};
use rusqlite::{Connection, params};
use uuid::Uuid;

#[test]
fn busy_writer_returns_durability_error_without_partial_project()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("busy.db");
    let database = Database::open(&path, &[])?;
    database
        .connection()
        .execute_batch("PRAGMA busy_timeout=50;")?;
    let locker = Connection::open(&path)?;
    locker.execute_batch("PRAGMA busy_timeout=50; BEGIN IMMEDIATE;")?;

    let result = ProjectRepository::new(database.clone()).create("blocked", ComparisonMode::Strict);
    assert!(matches!(result, Err(DedupeError::Durability(_))));
    locker.execute_batch("ROLLBACK;")?;
    let projects: i64 =
        database
            .connection()
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))?;
    assert_eq!(projects, 0);
    Ok(())
}

#[test]
fn database_full_rolls_back_insert_and_remains_integrity_checkable()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("full.db");
    let database = Database::open(&path, &[])?;
    {
        let connection = database.connection();
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
        let pages: i64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        connection.pragma_update(None, "max_page_count", pages)?;
    }
    let oversized_name = "x".repeat(2 * 1024 * 1024);
    let result =
        ProjectRepository::new(database.clone()).create(&oversized_name, ComparisonMode::Strict);
    assert!(matches!(result, Err(DedupeError::Durability(_))));
    let connection = database.connection();
    let projects: i64 =
        connection.query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))?;
    let integrity: String = connection.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    assert_eq!(projects, 0);
    assert_eq!(integrity, "ok");
    Ok(())
}

#[test]
fn dropping_uncommitted_transaction_simulates_interrupted_commit_rollback()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let database = Database::open(&temporary.path().join("rollback.db"), &[])?;
    let project = Uuid::new_v4();
    {
        let mut connection = database.connection();
        let transaction = connection.transaction()?;
        let now = Utc::now().to_rfc3339();
        transaction.execute(
            "INSERT INTO projects(id,name,mode,created_at,updated_at)
             VALUES (?1,'uncommitted','strict',?2,?2)",
            params![project.to_string(), now],
        )?;
    }
    let rows: i64 = database.connection().query_row(
        "SELECT COUNT(*) FROM projects WHERE id=?1",
        [project.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(rows, 0);
    Ok(())
}

#[test]
fn batched_scan_writer_flushes_every_record_before_orderly_shutdown()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("flush.db");
    let root = temporary.path().join("source");
    std::fs::create_dir(&root)?;
    let database = Database::open(&path, &[])?;
    let projects = ProjectRepository::new(database.clone());
    let project = projects.create("Writer flush", ComparisonMode::Strict)?;
    let root_id = projects.add_root(project, &root, false)?;
    let session =
        ScanRepository::new(database.clone()).create_session(project, ComparisonMode::Strict)?;
    let mut sink = SqliteScanSink::new(
        database.clone(),
        project,
        session,
        vec![(root_id, root.clone())],
    );
    for index in 0..300_u64 {
        sink.record(&snapshot(root.join(format!("item-{index:03}.pdf")), index))?;
    }
    sink.flush()?;
    drop(sink);
    drop(database);

    let reopened = Database::open(&path, &[])?;
    let snapshots: i64 = reopened.connection().query_row(
        "SELECT COUNT(*) FROM file_snapshots WHERE session_id=?1",
        [session.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(snapshots, 300);
    Ok(())
}

#[test]
fn batched_scan_writer_preserves_unicode_path_collisions_and_repeated_observations()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let path = temporary.path().join("unicode-collision.db");
    let root = temporary.path().join("source");
    std::fs::create_dir(&root)?;
    let database = Database::open(&path, &[])?;
    let projects = ProjectRepository::new(database.clone());
    let project = projects.create("Unicode collision", ComparisonMode::Strict)?;
    let root_id = projects.add_root(project, &root, false)?;
    let scans = ScanRepository::new(database.clone());
    let first_session = scans.create_session(project, ComparisonMode::Strict)?;
    let composed = snapshot(root.join("café.pdf"), 1);
    let mut decomposed = snapshot(root.join("cafe\u{301}.pdf"), 1);
    decomposed.normalized_name = composed.normalized_name.clone();
    let mut sink = SqliteScanSink::new(
        database.clone(),
        project,
        first_session,
        vec![(root_id, root.clone())],
    );
    sink.record(&composed)?;
    sink.record(&decomposed)?;
    sink.record(&composed)?;
    sink.flush()?;

    let blake3 = vec![7; 32];
    let sha256 = vec![9; 32];
    let group = DuplicateGroup {
        id: Uuid::new_v4(),
        mode: ComparisonMode::Strict,
        size_bytes: composed.size_bytes,
        normalized_name: Some(composed.normalized_name.clone()),
        blake3: blake3.clone(),
        sha256: sha256.clone(),
        members: vec![
            duplicate_member(composed.clone(), &blake3, &sha256, MemberAction::Keep),
            duplicate_member(
                decomposed.clone(),
                &blake3,
                &sha256,
                MemberAction::Quarantine,
            ),
        ],
    };
    let duplicates = DuplicateRepository::new(database.clone());
    duplicates.replace_session_groups(first_session, &[group])?;
    let stored_groups = duplicates.load_session_groups(first_session)?;
    assert_eq!(stored_groups.len(), 1);
    assert_ne!(
        stored_groups[0].members[0].file.metadata.path,
        stored_groups[0].members[1].file.metadata.path
    );
    let plans = PlanRepository::new(database.clone());
    let plan = plans.create_and_seal(
        first_session,
        &KeepPolicy::Default {
            primary_roots: Vec::new(),
        },
        &stored_groups,
    )?;
    assert_eq!(plans.latest_sealed_for_session(first_session)?, Some(plan));
    assert_latest_plan_context(&plans, project, first_session, plan)?;
    let summary = plans.summary(plan)?;
    assert_eq!(summary.quarantine_files, 1);
    assert_eq!(summary.quarantine_bytes, composed.size_bytes);

    let second_session = scans.create_session(project, ComparisonMode::Strict)?;
    let mut sink = SqliteScanSink::new(
        database.clone(),
        project,
        second_session,
        vec![(root_id, root)],
    );
    sink.record(&decomposed)?;
    sink.record(&composed)?;
    sink.flush()?;

    assert_collision_snapshot_counts(
        &database,
        project,
        first_session,
        second_session,
        plan,
        &composed.path,
    )?;
    assert_history_survives_compaction(&database, project, &composed.path, &decomposed.path)?;
    Ok(())
}

fn assert_collision_snapshot_counts(
    database: &Database,
    project: Uuid,
    first_session: Uuid,
    second_session: Uuid,
    plan: Uuid,
    path: &std::path::Path,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let connection = database.connection();
    let entries: i64 = connection.query_row(
        "SELECT COUNT(*) FROM file_entries WHERE project_id=?1",
        [project.to_string()],
        |row| row.get(0),
    )?;
    let first_snapshots: i64 = connection.query_row(
        "SELECT COUNT(*) FROM file_snapshots WHERE session_id=?1",
        [first_session.to_string()],
        |row| row.get(0),
    )?;
    let second_snapshots: i64 = connection.query_row(
        "SELECT COUNT(*) FROM file_snapshots WHERE session_id=?1",
        [second_session.to_string()],
        |row| row.get(0),
    )?;
    let planned_snapshots: i64 = connection.query_row(
        "SELECT COUNT(DISTINCT snapshot_id) FROM plan_items WHERE plan_id=?1",
        [plan.to_string()],
        |row| row.get(0),
    )?;
    assert_eq!(entries, 2);
    assert_eq!(first_snapshots, 2);
    assert_eq!(second_snapshots, 2);
    assert_eq!(planned_snapshots, 2);
    assert_snapshot_lookup_uses_composite_indexes(&connection, first_session, path)?;
    Ok(())
}

fn assert_history_survives_compaction(
    database: &Database,
    project: Uuid,
    first_path: &std::path::Path,
    second_path: &std::path::Path,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let history = HistoryRepository::new(database.clone());
    let before = history.list_files(project, "", true, 0, 50)?;
    assert_eq!(before.total_processed, 4);
    assert_eq!(before.duplicate_files, 2);
    assert_eq!(before.duplicate_groups, 1);
    let first = before
        .items
        .iter()
        .find(|item| item.path == first_path)
        .ok_or("first duplicate path missing from history")?;
    assert!(
        first
            .duplicate_locations
            .iter()
            .any(|path| path == second_path)
    );
    let maintenance = database.compact()?;
    assert!(maintenance.after_bytes > 0);
    let after = history.list_files(project, "", true, 0, 50)?;
    assert_eq!(after.total_processed, before.total_processed);
    assert_eq!(after.items.len(), before.items.len());
    Ok(())
}

fn assert_latest_plan_context(
    plans: &PlanRepository,
    project: Uuid,
    session: Uuid,
    plan: Uuid,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let context = plans
        .latest_sealed_context()?
        .ok_or("sealed plan context was not returned")?;
    assert_eq!(context.project_id, project);
    assert_eq!(context.session_id, session);
    assert_eq!(context.plan_id, plan);
    Ok(())
}

fn assert_snapshot_lookup_uses_composite_indexes(
    connection: &rusqlite::Connection,
    session: Uuid,
    path: &std::path::Path,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut plan_statement = connection.prepare(
        "EXPLAIN QUERY PLAN
         SELECT s.id,e.original_path FROM scan_sessions ss
         JOIN file_entries e ON e.project_id=ss.project_id AND e.path_key=?2
         JOIN file_snapshots s ON s.session_id=ss.id AND s.file_entry_id=e.id
         WHERE ss.id=?1",
    )?;
    let plan = plan_statement
        .query_map(
            params![session.to_string(), path_key(path)?.as_slice()],
            |row| row.get::<_, String>(3),
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n");
    assert!(
        plan.contains("file_entries_2 (project_id=? AND path_key=?)"),
        "kế hoạch truy vấn không dùng chỉ mục đường dẫn tổng hợp:\n{plan}"
    );
    assert!(
        plan.contains("file_snapshots_2 (session_id=? AND file_entry_id=?)"),
        "kế hoạch truy vấn không dùng chỉ mục snapshot tổng hợp:\n{plan}"
    );
    Ok(())
}

fn duplicate_member(
    metadata: FileMetadataSnapshot,
    blake3: &[u8],
    sha256: &[u8],
    action: MemberAction,
) -> DuplicateMember {
    let token = metadata.snapshot_token;
    let bytes_read = metadata.size_bytes;
    DuplicateMember {
        file: ProvenFile {
            metadata,
            blake3: HashResult {
                algorithm: HashAlgorithm::Blake3,
                digest: blake3.to_vec(),
                bytes_read,
                snapshot_before: token,
                snapshot_after: token,
                stable: true,
            },
            sha256: HashResult {
                algorithm: HashAlgorithm::Sha256,
                digest: sha256.to_vec(),
                bytes_read,
                snapshot_before: token,
                snapshot_after: token,
                stable: true,
            },
        },
        action,
        reason: "Kiểm thử va chạm đường dẫn Unicode".into(),
    }
}

fn snapshot(path: PathBuf, index: u64) -> FileMetadataSnapshot {
    let modified_ns = i128::from(index);
    let size_bytes = index.saturating_add(1);
    FileMetadataSnapshot {
        normalized_path: path.to_string_lossy().to_lowercase(),
        normalized_name: path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().to_lowercase()),
        extension: Some("pdf".into()),
        path,
        size_bytes,
        created_ns: None,
        modified_ns,
        identity: None,
        link_kind: LinkKind::Regular,
        hardlink_count: Some(1),
        access_status: AccessStatus::Readable,
        snapshot_token: snapshot_token(None, size_bytes, modified_ns),
    }
}
