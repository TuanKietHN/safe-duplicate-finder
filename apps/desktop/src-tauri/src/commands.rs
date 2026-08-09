//! Validated Tauri commands backed by the same reusable engine as the CLI.

use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

use dedupe_core::{
    DedupeError,
    control::ControlToken,
    duplicate_detector::confirm_preliminary_group_detailed_with_config,
    filters::{CompiledFilter, FilterConfig},
    keep_policy,
    model::{ComparisonMode, DuplicateGroup, KeepPolicy},
    permanent_delete::{self, PermanentDeleteChallenge, PermanentDeleteOutcome},
    ports::ScanSink,
    progress::{ProgressCounters, ProgressSnapshot},
    quarantine,
    scanner::scan_roots_with_config,
};
use dedupe_platform::PlatformFileSystem;
use dedupe_store::{
    Database, DatabaseMaintenance, DuplicateRepository, FileHistoryPage, HistoryRepository,
    LatestPlanContext, PermanentDeleteRepository, PlanRepository, PlanSummary, ProjectRecord,
    ProjectRepository, ProjectRootRecord, QuarantineEntryRecord, ScanControlMonitor,
    ScanControlRequest, ScanRepository, ScanSessionRecord, SqlitePermanentDeleteJournal,
    SqliteScanSink, SqliteTransactionJournal, TransactionRepository,
};
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use crate::{
    events::{DesktopEvent, EventHub},
    state::{EngineState, ScanJob},
};

/// Disk usage owned by the installed application, excluding source and quarantine files.
#[derive(Debug, Clone, Serialize)]
pub struct StorageOverview {
    /// Application-local directory containing the measured data.
    pub data_directory: String,
    /// Main database plus WAL and shared-memory files.
    pub database_bytes: u64,
    /// Durable append-only transaction manifests.
    pub manifest_bytes: u64,
    /// Human-readable and structured diagnostic logs.
    pub log_bytes: u64,
    /// Disposable `WebView2` browsing/cache data.
    pub interface_cache_bytes: u64,
    /// Other small files under the application-local directory.
    pub other_bytes: u64,
    /// Total application-local bytes.
    pub total_bytes: u64,
}

/// Result of deleting only diagnostic log files older than the requested retention.
#[derive(Debug, Clone, Serialize)]
pub struct LogCleanup {
    /// Files successfully removed.
    pub deleted_files: u64,
    /// Bytes represented by those files before removal.
    pub reclaimed_bytes: u64,
}

/// Verify the local database is ready.
#[tauri::command]
#[must_use]
pub fn engine_status(state: State<'_, EngineState>) -> String {
    format!(
        "Sẵn sàng · cơ sở dữ liệu cục bộ {}",
        state.database.path().display()
    )
}

/// List projects without triggering a scan.
#[tauri::command]
pub fn list_projects(state: State<'_, EngineState>) -> Result<Vec<ProjectRecord>, String> {
    list_projects_request(state.inner())
}

/// Testable request handler shared with the Tauri wrapper.
pub fn list_projects_request(state: &EngineState) -> Result<Vec<ProjectRecord>, String> {
    ProjectRepository::new(state.database.clone())
        .list()
        .map_err(message)
}

/// Create a project with strict mode by default.
#[tauri::command]
pub fn create_project(
    state: State<'_, EngineState>,
    name: String,
    mode: String,
) -> Result<String, String> {
    create_project_request(state.inner(), &name, &mode)
}

/// Testable project-create handler; creating configuration never starts a scan.
pub fn create_project_request(
    state: &EngineState,
    name: &str,
    mode: &str,
) -> Result<String, String> {
    ProjectRepository::new(state.database.clone())
        .create(name, parse_mode(mode)?)
        .map(|id| id.to_string())
        .map_err(message)
}

/// Rename an active project or update its comparison mode without scanning.
#[tauri::command]
pub fn update_project(
    state: State<'_, EngineState>,
    project_id: String,
    name: String,
    mode: String,
) -> Result<(), String> {
    ProjectRepository::new(state.database.clone())
        .update(
            parse_uuid(&project_id, "project")?,
            &name,
            parse_mode(&mode)?,
        )
        .map_err(message)
}

/// Persist a validated global worker limit without starting a scan.
#[tauri::command]
pub fn set_project_workers(
    state: State<'_, EngineState>,
    project_id: String,
    workers: usize,
) -> Result<(), String> {
    set_project_workers_request(state.inner(), &project_id, workers)
}

/// Testable persisted-worker request handler.
pub fn set_project_workers_request(
    state: &EngineState,
    project_id: &str,
    workers: usize,
) -> Result<(), String> {
    ProjectRepository::new(state.database.clone())
        .set_worker_limit(parse_uuid(project_id, "project")?, workers)
        .map_err(message)
}

/// Archive only the project record after an exact confirmation.
#[tauri::command]
pub fn archive_project(
    state: State<'_, EngineState>,
    project_id: String,
    confirmation: String,
) -> Result<(), String> {
    archive_project_request(state.inner(), &project_id, &confirmation)
}

/// Testable archive handler with exact confirmation and idempotent archived-state handling.
pub fn archive_project_request(
    state: &EngineState,
    project_id: &str,
    confirmation: &str,
) -> Result<(), String> {
    require_exact(confirmation, "ARCHIVE")?;
    ProjectRepository::new(state.database.clone())
        .archive(parse_uuid(project_id, "project")?)
        .map_err(message)
}

/// Load persistent read-only scan filters without starting a scan.
#[tauri::command]
pub fn get_filter_config(
    state: State<'_, EngineState>,
    project_id: String,
) -> Result<FilterConfig, String> {
    ProjectRepository::new(state.database.clone())
        .filter_config(parse_uuid(&project_id, "project")?)
        .map_err(message)
}

/// Validate and atomically save persistent scan filters without starting a scan.
#[tauri::command]
pub fn save_filter_config(
    state: State<'_, EngineState>,
    project_id: String,
    config: FilterConfig,
) -> Result<(), String> {
    ProjectRepository::new(state.database.clone())
        .replace_filter_config(parse_uuid(&project_id, "project")?, &config)
        .map_err(message)
}

/// Add a folder after overlap/database/quarantine validation.
#[tauri::command]
pub fn add_root(
    state: State<'_, EngineState>,
    project_id: String,
    path: String,
    primary: bool,
) -> Result<String, String> {
    add_root_request(state.inner(), &project_id, &path, primary)
}

/// Testable root-add handler; validation persists configuration without scanning.
pub fn add_root_request(
    state: &EngineState,
    project_id: &str,
    path: &str,
    primary: bool,
) -> Result<String, String> {
    ProjectRepository::new(state.database.clone())
        .add_root(parse_uuid(project_id, "project")?, Path::new(path), primary)
        .map(|id| id.to_string())
        .map_err(message)
}

/// List configured roots without starting a scan.
#[tauri::command]
pub fn list_roots(
    state: State<'_, EngineState>,
    project_id: String,
) -> Result<Vec<ProjectRootRecord>, String> {
    ProjectRepository::new(state.database.clone())
        .root_records(parse_uuid(&project_id, "project")?)
        .map_err(message)
}

/// Disable one source-root record without touching the filesystem.
#[tauri::command]
pub fn remove_root(
    state: State<'_, EngineState>,
    project_id: String,
    root_id: String,
) -> Result<(), String> {
    ProjectRepository::new(state.database.clone())
        .remove_root(
            parse_uuid(&project_id, "project")?,
            parse_uuid(&root_id, "root")?,
        )
        .map_err(message)
}

/// Start one bounded background scan and immediately return its durable session id.
#[tauri::command]
pub fn start_scan(
    state: State<'_, EngineState>,
    project_id: String,
    mode: String,
    acknowledged: bool,
    all_files: bool,
) -> Result<String, String> {
    start_scan_request(state.inner(), &project_id, &mode, acknowledged, all_files)
}

/// Testable scan-start handler. Validation completes before a durable session is allocated.
pub fn start_scan_request(
    state: &EngineState,
    project_id: &str,
    mode: &str,
    acknowledged: bool,
    all_files: bool,
) -> Result<String, String> {
    let project = parse_uuid(project_id, "project")?;
    let mode = parse_mode(mode)?;
    if mode == ComparisonMode::Content && !acknowledged {
        return Err(
            "Chế độ chỉ so sánh nội dung bỏ qua tên tệp; hãy xác nhận cảnh báo trước.".into(),
        );
    }
    let projects = ProjectRepository::new(state.database.clone());
    let roots = projects.roots(project).map_err(message)?;
    if roots.is_empty() {
        return Err("Dự án không có thư mục nguồn đang bật.".into());
    }
    let scans = ScanRepository::new(state.database.clone());
    let session = scans
        .create_session_with_config(project, mode, all_files)
        .map_err(message)?;
    launch_scan(state.clone(), project, session, mode, roots, all_files)?;
    Ok(session.to_string())
}

/// Restart an interrupted scan from a durable, read-only stage boundary.
#[tauri::command]
pub fn resume_scan(state: State<'_, EngineState>, session_id: String) -> Result<String, String> {
    let session = parse_uuid(&session_id, "session")?;
    if state.jobs.lock().contains_key(&session) {
        return Err(
            "Phiên quét đang hoạt động; hãy dùng điều khiển tiếp tục trong ứng dụng.".into(),
        );
    }
    let scans = ScanRepository::new(state.database.clone());
    let spec = scans.resume_spec(session).map_err(message)?;
    let roots = ProjectRepository::new(state.database.clone())
        .roots(spec.project_id)
        .map_err(message)?;
    if roots.is_empty() {
        return Err("Dự án không có thư mục nguồn đang bật.".into());
    }
    scans.prepare_resume(session).map_err(message)?;
    if let Err(error) = launch_scan(
        state.inner().clone(),
        spec.project_id,
        session,
        spec.mode,
        roots,
        spec.all_files,
    ) {
        let _ = scans.block_session(session, &error);
        return Err(error);
    }
    Ok(session.to_string())
}

fn launch_scan(
    state: EngineState,
    project: Uuid,
    session: Uuid,
    mode: ComparisonMode,
    roots: Vec<(Uuid, PathBuf, bool)>,
    all_files: bool,
) -> Result<(), String> {
    let control = ControlToken::new();
    let progress = Arc::new(ProgressCounters::default());
    let mut jobs = state.jobs.lock();
    if jobs.contains_key(&session) {
        return Err("Phiên quét đang hoạt động.".into());
    }
    if jobs.len() >= 2 {
        return Err("Chỉ được chạy đồng thời tối đa hai phiên quét.".into());
    }
    jobs.insert(
        session,
        ScanJob {
            progress: Arc::clone(&progress),
        },
    );
    drop(jobs);
    state
        .events
        .publish_scan(project, session, "scan://state", progress.snapshot());
    let state_clone = state;
    tauri::async_runtime::spawn_blocking(move || {
        let monitor = ScanControlMonitor::start(
            state_clone.database.clone(),
            session,
            control.clone(),
            Arc::clone(&progress),
        );
        let result = run_scan_job(
            state_clone.database.clone(),
            project,
            session,
            mode,
            roots,
            all_files,
            &control,
            &progress,
            &state_clone.events,
        );
        let monitor_result = monitor.finish();
        if let Err(error) = result.and(monitor_result) {
            let repository = ScanRepository::new(state_clone.database.clone());
            let _ = repository.update_progress(session, progress.snapshot());
            if matches!(error, DedupeError::Cancelled) {
                let _ = repository.set_state(session, "cancelled");
            } else {
                let _ = repository.block_session(session, &error.to_string());
            }
        }
        state_clone
            .events
            .publish_scan(project, session, "scan://state", progress.snapshot());
        state_clone.jobs.lock().remove(&session);
    });
    Ok(())
}

/// Poll durable and live scan progress.
#[tauri::command]
pub fn scan_status(
    state: State<'_, EngineState>,
    session_id: String,
) -> Result<ScanSessionRecord, String> {
    let session = parse_uuid(&session_id, "session")?;
    if let Some(job) = state.jobs.lock().get(&session).cloned() {
        ScanRepository::new(state.database.clone())
            .update_progress(session, job.progress.snapshot())
            .map_err(message)?;
    }
    let scans = ScanRepository::new(state.database.clone());
    let status = scans.status(session).map_err(message)?;
    state.events.publish_scan(
        scans.project_id(session).map_err(message)?,
        session,
        "scan://snapshot",
        status_progress(&status),
    );
    Ok(status)
}

/// Pause, resume, or cancel a running scan at cooperative boundaries.
#[tauri::command]
pub fn control_scan(
    state: State<'_, EngineState>,
    session_id: String,
    action: String,
) -> Result<(), String> {
    let session = parse_uuid(&session_id, "session")?;
    let job = state
        .jobs
        .lock()
        .get(&session)
        .cloned()
        .ok_or_else(|| "Phiên quét không còn hoạt động.".to_owned())?;
    let request = match action.as_str() {
        "pause" => ScanControlRequest::Pause,
        "resume" => ScanControlRequest::Resume,
        "cancel" => ScanControlRequest::Cancel,
        _ => return Err("Thao tác phải là pause, resume hoặc cancel.".into()),
    };
    ScanRepository::new(state.database.clone())
        .request_control(session, request)
        .map_err(message)?;
    state.events.publish_scan(
        ScanRepository::new(state.database.clone())
            .project_id(session)
            .map_err(message)?,
        session,
        "scan://state",
        job.progress.snapshot(),
    );
    Ok(())
}

/// Take the newest pending scan event. Missing events are normal; durable status is authoritative.
#[tauri::command]
pub fn next_scan_event(
    state: State<'_, EngineState>,
    session_id: String,
) -> Result<Option<DesktopEvent>, String> {
    Ok(state.events.take(parse_uuid(&session_id, "session")?))
}

/// Load proven groups only; sampled matches are never returned here.
#[tauri::command]
pub async fn list_results(
    state: State<'_, EngineState>,
    session_id: String,
) -> Result<Vec<DuplicateGroup>, String> {
    let database = state.database.clone();
    let session = parse_uuid(&session_id, "session")?;
    run_blocking(move || {
        DuplicateRepository::new(database)
            .load_session_groups(session)
            .map_err(message)
    })
    .await
}

/// Apply a keeper policy and seal the immutable plan.
#[tauri::command]
pub async fn create_plan(
    state: State<'_, EngineState>,
    session_id: String,
    policy: String,
) -> Result<String, String> {
    let state = state.inner().clone();
    run_blocking(move || create_plan_request(&state, &session_id, &policy)).await
}

fn create_plan_request(
    state: &EngineState,
    session_id: &str,
    policy: &str,
) -> Result<String, String> {
    let session = parse_uuid(session_id, "session")?;
    let scans = ScanRepository::new(state.database.clone());
    let project = scans.project_id(session).map_err(message)?;
    let roots = ProjectRepository::new(state.database.clone())
        .roots(project)
        .map_err(message)?;
    let primary_roots = roots
        .into_iter()
        .filter(|(_, _, primary)| *primary)
        .map(|(_, path, _)| path)
        .collect();
    let policy = match policy {
        "default" => KeepPolicy::Default { primary_roots },
        "oldest" => KeepPolicy::Oldest,
        "newest" => KeepPolicy::Newest,
        "shortest" => KeepPolicy::ShortestPath,
        _ => return Err("Không nhận diện được chính sách giữ tệp.".into()),
    };
    let duplicates = DuplicateRepository::new(state.database.clone());
    let mut groups = duplicates.load_session_groups(session).map_err(message)?;
    for group in &mut groups {
        keep_policy::apply(group, &policy).map_err(message)?;
    }
    duplicates
        .replace_session_groups(session, &groups)
        .map_err(message)?;
    PlanRepository::new(state.database.clone())
        .create_and_seal(session, &policy, &groups)
        .map(|id| id.to_string())
        .map_err(message)
}

/// Return the newest sealed plan for a completed session so navigation/restart can restore context.
#[tauri::command]
pub fn latest_plan_for_session(
    state: State<'_, EngineState>,
    session_id: String,
) -> Result<Option<String>, String> {
    latest_plan_for_session_request(state.inner(), &session_id)
}

/// Testable lookup that validates the session identifier before reading a sealed plan.
pub fn latest_plan_for_session_request(
    state: &EngineState,
    session_id: &str,
) -> Result<Option<String>, String> {
    PlanRepository::new(state.database.clone())
        .latest_sealed_for_session(parse_uuid(session_id, "session")?)
        .map(|plan| plan.map(|id| id.to_string()))
        .map_err(message)
}

/// Restore the newest sealed workflow when `WebView` state is empty after restart or upgrade.
#[tauri::command]
pub fn latest_plan_context(
    state: State<'_, EngineState>,
) -> Result<Option<LatestPlanContext>, String> {
    latest_plan_context_request(state.inner())
}

/// Testable read-only lookup for the newest sealed desktop workflow.
pub fn latest_plan_context_request(
    state: &EngineState,
) -> Result<Option<LatestPlanContext>, String> {
    PlanRepository::new(state.database.clone())
        .latest_sealed_context()
        .map_err(message)
}

/// Return exact dry-run totals after a read-only metadata freshness check.
#[tauri::command]
pub async fn dry_run(
    state: State<'_, EngineState>,
    plan_id: String,
) -> Result<PlanSummary, String> {
    let state = state.inner().clone();
    run_blocking(move || dry_run_request(&state, &plan_id)).await
}

fn dry_run_request(state: &EngineState, plan_id: &str) -> Result<PlanSummary, String> {
    let plan = parse_uuid(plan_id, "plan")?;
    let plans = PlanRepository::new(state.database.clone());
    let summary = plans.summary(plan).map_err(message)?;
    if summary.status == "sealed" {
        let groups = DuplicateRepository::new(state.database.clone())
            .load_session_groups(summary.session_id)
            .map_err(message)?;
        if let Err(error) = dedupe_core::dry_run::validate_fresh(&groups, &PlatformFileSystem) {
            plans.mark_stale(plan).map_err(message)?;
            return Err(message(error));
        }
    }
    Ok(summary)
}

/// Apply a sealed plan after an exact high-friction confirmation.
#[tauri::command]
pub async fn apply_quarantine(
    state: State<'_, EngineState>,
    plan_id: String,
    confirmation: String,
) -> Result<u64, String> {
    let state = state.inner().clone();
    run_blocking(move || apply_quarantine_request(&state, &plan_id, &confirmation)).await
}

/// Testable quarantine handler that validates confirmation before identifiers or state.
pub fn apply_quarantine_request(
    state: &EngineState,
    plan_id: &str,
    confirmation: &str,
) -> Result<u64, String> {
    require_exact(confirmation, "QUARANTINE")?;
    apply_plan(state.database.clone(), parse_uuid(plan_id, "plan")?)
}

/// List recoverable inventory.
#[tauri::command]
pub fn list_quarantine(
    state: State<'_, EngineState>,
    project_id: String,
) -> Result<Vec<QuarantineEntryRecord>, String> {
    TransactionRepository::new(state.database.clone())
        .list_quarantine(parse_uuid(&project_id, "project")?)
        .map_err(message)
}

/// Prepare a short-lived challenge for individually selected verified quarantine entries.
#[tauri::command]
pub async fn prepare_permanent_delete(
    state: State<'_, EngineState>,
    entry_ids: Vec<String>,
    delete_now: bool,
) -> Result<PermanentDeleteChallenge, String> {
    let state = state.inner().clone();
    run_blocking(move || prepare_permanent_delete_request(&state, &entry_ids, delete_now)).await
}

/// Testable prepare handler; its input surface contains UUIDs only, never filesystem paths.
pub fn prepare_permanent_delete_request(
    state: &EngineState,
    entry_ids: &[String],
    delete_now: bool,
) -> Result<PermanentDeleteChallenge, String> {
    let ids = entry_ids
        .iter()
        .map(|entry| parse_uuid(entry, "quarantine entry"))
        .collect::<Result<Vec<_>, _>>()?;
    let selected = PermanentDeleteRepository::new(state.database.clone())
        .selected_entries(&ids)
        .map_err(message)?;
    let journal = SqlitePermanentDeleteJournal::new(
        state.database.clone(),
        permanent_delete_manifest_path(&state.database),
    )
    .map_err(message)?;
    if delete_now {
        permanent_delete::prepare_immediate(selected, &journal, chrono::Utc::now()).map_err(message)
    } else {
        permanent_delete::prepare(selected, &journal, chrono::Utc::now()).map_err(message)
    }
}

/// Execute or resume one prepared quarantine-only permanent-delete batch.
#[tauri::command]
pub async fn execute_permanent_delete(
    state: State<'_, EngineState>,
    batch_id: String,
    token: String,
    confirmation: String,
) -> Result<PermanentDeleteOutcome, String> {
    let state = state.inner().clone();
    run_blocking(move || execute_permanent_delete_request(&state, &batch_id, &token, &confirmation))
        .await
}

/// Testable execute handler shared with the Tauri wrapper.
pub fn execute_permanent_delete_request(
    state: &EngineState,
    batch_id: &str,
    token: &str,
    confirmation: &str,
) -> Result<PermanentDeleteOutcome, String> {
    let journal = SqlitePermanentDeleteJournal::new(
        state.database.clone(),
        permanent_delete_manifest_path(&state.database),
    )
    .map_err(message)?;
    let provider = PlatformFileSystem;
    permanent_delete::execute(
        parse_uuid(batch_id, "permanent-delete batch")?,
        token,
        confirmation,
        &journal,
        &provider,
        &provider,
        &ControlToken::new(),
        chrono::Utc::now(),
    )
    .map_err(message)
}

/// Restore one verified entry, refusing overwrite.
#[tauri::command]
pub async fn restore_entry(
    state: State<'_, EngineState>,
    entry_id: String,
    confirmation: String,
) -> Result<(), String> {
    let state = state.inner().clone();
    run_blocking(move || restore_entry_request(&state, &entry_id, &confirmation)).await
}

/// Testable restore handler that validates confirmation before inventory lookup.
pub fn restore_entry_request(
    state: &EngineState,
    entry_id: &str,
    confirmation: &str,
) -> Result<(), String> {
    require_exact(confirmation, "RESTORE")?;
    let database = state.database.clone();
    let origin = TransactionRepository::new(database.clone())
        .verified_quarantine_transaction(parse_uuid(entry_id, "entry")?)
        .map_err(message)?;
    let mut restore = dedupe_core::restore::planned_transaction(&origin).map_err(message)?;
    let journal = SqliteTransactionJournal::new(database.clone(), manifest_path(&database))
        .map_err(message)?;
    let provider = PlatformFileSystem;
    dedupe_core::restore::execute(
        &mut restore,
        &provider,
        &provider,
        &journal,
        &ControlToken::new(),
    )
    .map_err(message)
}

/// Inspect incomplete transactions.
#[tauri::command]
pub fn inspect_recovery(
    state: State<'_, EngineState>,
    project_id: String,
) -> Result<Vec<dedupe_core::model::FileTransaction>, String> {
    TransactionRepository::new(state.database.clone())
        .pending_recovery(parse_uuid(&project_id, "project")?)
        .map_err(message)
}

/// Explicitly reconcile one incomplete transaction.
#[tauri::command]
pub async fn reconcile_transaction(
    state: State<'_, EngineState>,
    transaction_id: String,
    confirmation: String,
) -> Result<String, String> {
    let state = state.inner().clone();
    run_blocking(move || reconcile_transaction_request(&state, &transaction_id, &confirmation))
        .await
}

/// Testable recovery handler that validates confirmation before transaction lookup.
pub fn reconcile_transaction_request(
    state: &EngineState,
    transaction_id: &str,
    confirmation: &str,
) -> Result<String, String> {
    require_exact(confirmation, "RECONCILE")?;
    let database = state.database.clone();
    let mut transaction = TransactionRepository::new(database.clone())
        .transaction(parse_uuid(transaction_id, "transaction")?)
        .map_err(message)?;
    let journal = SqliteTransactionJournal::new(database.clone(), manifest_path(&database))
        .map_err(message)?;
    dedupe_core::recovery::reconcile(
        &mut transaction,
        &PlatformFileSystem,
        &journal,
        &ControlToken::new(),
    )
    .map(|outcome| format!("{outcome:?}"))
    .map_err(message)
}

/// Export a report to an explicitly selected local path.
#[tauri::command]
pub async fn export_report(
    state: State<'_, EngineState>,
    session_id: String,
    format: String,
    destination: String,
) -> Result<(), String> {
    let database = state.database.clone();
    let session = parse_uuid(&session_id, "session")?;
    run_blocking(move || export_report_request(database, session, &format, &destination)).await
}

fn export_report_request(
    database: Database,
    session: Uuid,
    format: &str,
    destination: &str,
) -> Result<(), String> {
    let groups = DuplicateRepository::new(database)
        .load_session_groups(session)
        .map_err(message)?;
    let path = PathBuf::from(destination);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let file = File::create(&path).map_err(|error| error.to_string())?;
    match format {
        "csv" => dedupe_report::write_csv(&groups, file).map_err(|error| error.to_string()),
        "json" => dedupe_report::write_json(&groups, file).map_err(|error| error.to_string()),
        "html" => dedupe_report::write_html(&groups, file).map_err(|error| error.to_string()),
        _ => Err("Định dạng báo cáo phải là csv, json hoặc html.".into()),
    }
}

/// Return one bounded page of durable per-file processing and duplicate-location history.
#[tauri::command]
pub async fn list_file_history(
    state: State<'_, EngineState>,
    project_id: String,
    search: String,
    duplicate_only: bool,
    offset: u64,
    limit: u64,
) -> Result<FileHistoryPage, String> {
    let state = state.inner().clone();
    run_blocking(move || {
        list_file_history_request(&state, &project_id, &search, duplicate_only, offset, limit)
    })
    .await
}

/// Testable read-only file-history query.
pub fn list_file_history_request(
    state: &EngineState,
    project_id: &str,
    search: &str,
    duplicate_only: bool,
    offset: u64,
    limit: u64,
) -> Result<FileHistoryPage, String> {
    HistoryRepository::new(state.database.clone())
        .list_files(
            parse_uuid(project_id, "project")?,
            search,
            duplicate_only,
            offset,
            limit,
        )
        .map_err(message)
}

/// Measure only application-owned local data; source and quarantine trees are excluded.
#[tauri::command]
pub async fn storage_overview(state: State<'_, EngineState>) -> Result<StorageOverview, String> {
    let state = state.inner().clone();
    run_blocking(move || storage_overview_request(&state)).await
}

/// Testable disk-usage measurement for the application-local directory.
pub fn storage_overview_request(state: &EngineState) -> Result<StorageOverview, String> {
    let data_directory = state
        .database
        .path()
        .parent()
        .ok_or_else(|| "Không xác định được thư mục dữ liệu ứng dụng.".to_owned())?;
    let database_bytes = database_file_bytes(state.database.path()).map_err(message)?;
    let manifest_bytes = file_bytes(&manifest_path(&state.database)).map_err(message)?
        + file_bytes(&permanent_delete_manifest_path(&state.database)).map_err(message)?;
    let log_bytes = directory_bytes(&data_directory.join("logs")).map_err(message)?;
    let interface_cache_bytes =
        directory_bytes(&data_directory.join("EBWebView")).map_err(message)?;
    let total_bytes = directory_bytes(data_directory).map_err(message)?;
    let known = database_bytes
        .saturating_add(manifest_bytes)
        .saturating_add(log_bytes)
        .saturating_add(interface_cache_bytes);
    Ok(StorageOverview {
        data_directory: data_directory.to_string_lossy().into_owned(),
        database_bytes,
        manifest_bytes,
        log_bytes,
        interface_cache_bytes,
        other_bytes: total_bytes.saturating_sub(known),
        total_bytes,
    })
}

/// Checkpoint and compact the database without deleting history or filesystem evidence.
#[tauri::command]
pub async fn optimize_storage(
    state: State<'_, EngineState>,
) -> Result<DatabaseMaintenance, String> {
    if !state.jobs.lock().is_empty() {
        return Err("Hãy chờ mọi phiên quét đang chạy kết thúc trước khi tối ưu SQLite.".into());
    }
    let database = state.database.clone();
    run_blocking(move || database.compact().map_err(message)).await
}

/// Delete only diagnostic log files older than the explicit retention period.
#[tauri::command]
pub async fn cleanup_old_logs(
    state: State<'_, EngineState>,
    older_than_days: u64,
) -> Result<LogCleanup, String> {
    if !(7..=3_650).contains(&older_than_days) {
        return Err("Thời gian giữ log phải từ 7 đến 3.650 ngày.".into());
    }
    let state = state.inner().clone();
    run_blocking(move || cleanup_old_logs_request(&state, older_than_days)).await
}

/// Testable cleanup that never enters source, quarantine, manifest or database paths.
pub fn cleanup_old_logs_request(
    state: &EngineState,
    older_than_days: u64,
) -> Result<LogCleanup, String> {
    let data_directory = state
        .database
        .path()
        .parent()
        .ok_or_else(|| "Không xác định được thư mục dữ liệu ứng dụng.".to_owned())?;
    let logs = data_directory.join("logs");
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(
            older_than_days.saturating_mul(86_400),
        ))
        .ok_or_else(|| "Mốc thời gian dọn log không hợp lệ.".to_owned())?;
    let mut result = LogCleanup {
        deleted_files: 0,
        reclaimed_bytes: 0,
    };
    let entries = match std::fs::read_dir(&logs) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(result),
        Err(error) => return Err(error.to_string()),
    };
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        if metadata.is_file() && metadata.modified().is_ok_and(|modified| modified < cutoff) {
            std::fs::remove_file(entry.path()).map_err(|error| error.to_string())?;
            result.deleted_files = result.deleted_files.saturating_add(1);
            result.reclaimed_bytes = result.reclaimed_bytes.saturating_add(metadata.len());
        }
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn run_scan_job(
    database: Database,
    project: Uuid,
    session: Uuid,
    mode: ComparisonMode,
    roots: Vec<(Uuid, PathBuf, bool)>,
    all_files: bool,
    control: &ControlToken,
    progress: &ProgressCounters,
    events: &EventHub,
) -> dedupe_core::Result<()> {
    let source_paths = roots
        .iter()
        .map(|(_, path, _)| path.clone())
        .collect::<Vec<_>>();
    let projects = ProjectRepository::new(database.clone());
    let mut filter_config = projects.filter_config(project)?;
    let workers = projects.worker_config(project)?;
    if all_files {
        filter_config.include_extensions.clear();
    }
    let filter = CompiledFilter::new(filter_config)?;
    let mut sink = SqliteScanSink::new(
        database.clone(),
        project,
        session,
        roots
            .iter()
            .map(|(id, path, _)| (*id, path.clone()))
            .collect(),
    );
    let provider = PlatformFileSystem;
    scan_roots_with_config(
        &source_paths,
        &filter,
        &provider,
        &mut sink,
        control,
        progress,
        workers,
    )?;
    let scans = ScanRepository::new(database.clone());
    let enumeration = progress.snapshot();
    scans.checkpoint(session, "metadata_complete", enumeration.processed_files)?;
    scans.update_progress(session, enumeration)?;
    scans.set_state(session, "quick_hashing")?;
    events.publish_scan(project, session, "scan://snapshot", enumeration);
    let primary_roots = roots
        .iter()
        .filter(|(_, _, primary)| *primary)
        .map(|(_, path, _)| path.clone())
        .collect();
    let policy = KeepPolicy::Default { primary_roots };
    let mut groups = Vec::new();
    scans.for_each_candidate_group(session, mode, |candidates| {
        control.checkpoint()?;
        let mut outcome = confirm_preliminary_group_detailed_with_config(
            mode,
            &candidates,
            &provider,
            control,
            workers,
        )?;
        progress.add_bytes(outcome.bytes_read);
        progress.add_unstable(outcome.unstable_files);
        progress.add_errors(outcome.errors.len() as u64);
        events.publish_scan(project, session, "scan://snapshot", progress.snapshot());
        for error in outcome.errors {
            sink.record_error(&error.path, &error.error)?;
        }
        for group in &mut outcome.groups {
            keep_policy::apply(group, &policy)?;
        }
        groups.extend(outcome.groups);
        Ok(())
    })?;
    sink.flush()?;
    scans.checkpoint(session, "hashing_complete", groups.len() as u64)?;
    scans.set_state(session, "grouping")?;
    DuplicateRepository::new(database.clone()).replace_session_groups(session, &groups)?;
    scans.complete_session(session, progress.snapshot())?;
    Ok(())
}

fn status_progress(status: &ScanSessionRecord) -> ProgressSnapshot {
    ProgressSnapshot {
        discovered_files: status.discovered_files,
        processed_files: status.processed_files,
        bytes_read: status.bytes_read,
        errors: status.errors,
        skipped: status.skipped,
        unstable: status.unstable,
    }
}

fn apply_plan(database: Database, plan: Uuid) -> Result<u64, String> {
    let plans = PlanRepository::new(database.clone());
    let summary = plans.summary(plan).map_err(message)?;
    let items = plans.quarantine_items(plan).map_err(message)?;
    let project = plans.project_id(plan).map_err(message)?;
    let roots = ProjectRepository::new(database.clone())
        .roots(project)
        .map_err(message)?;
    let groups = DuplicateRepository::new(database.clone())
        .load_session_groups(summary.session_id)
        .map_err(message)?;
    for group in &groups {
        quarantine::verify_live_keeper(group, &PlatformFileSystem, &ControlToken::new())
            .map_err(message)?;
    }
    plans.mark_executing(plan).map_err(message)?;
    let journal = SqliteTransactionJournal::new(database.clone(), manifest_path(&database))
        .map_err(message)?;
    let provider = PlatformFileSystem;
    let mut verified_bytes = 0_u64;
    for item in items {
        let group = groups
            .iter()
            .find(|group| group.id == item.group_id)
            .ok_or_else(|| "Kế hoạch tham chiếu đến nhóm trùng không tồn tại.".to_owned())?;
        quarantine::verify_live_keeper(group, &provider, &ControlToken::new()).map_err(message)?;
        let source_root = roots
            .iter()
            .filter(|(_, root, _)| item.file.metadata.path.starts_with(root))
            .max_by_key(|(_, root, _)| root.as_os_str().len())
            .map(|(_, root, _)| root)
            .ok_or_else(|| {
                "Nguồn trong kế hoạch nằm ngoài các thư mục gốc đã cấu hình.".to_owned()
            })?;
        let destination = quarantine::quarantine_destination(
            &source_root.join(".safe-duplicate-finder-quarantine"),
            project,
            summary.session_id,
            item.plan_item_id,
            source_root,
            &item.file.metadata.path,
        )
        .map_err(message)?;
        let mut transaction = quarantine::planned_transaction(
            project,
            summary.session_id,
            item.plan_item_id,
            &item.file,
            destination,
        )
        .map_err(message)?;
        quarantine::execute(
            &mut transaction,
            &provider,
            &provider,
            &journal,
            &ControlToken::new(),
        )
        .map_err(message)?;
        verified_bytes = verified_bytes.saturating_add(item.file.metadata.size_bytes);
    }
    if !plans.mark_completed_if_verified(plan).map_err(message)? {
        return Err("Kế hoạch vẫn còn mục chưa xác minh; hãy kiểm tra phần phục hồi.".into());
    }
    Ok(verified_bytes)
}

fn parse_mode(value: &str) -> Result<ComparisonMode, String> {
    match value {
        "strict" => Ok(ComparisonMode::Strict),
        "content" => Ok(ComparisonMode::Content),
        _ => Err("Chế độ phải là strict hoặc content.".into()),
    }
}

fn parse_uuid(value: &str, kind: &str) -> Result<Uuid, String> {
    let label = match kind {
        "project" => "dự án",
        "root" => "thư mục gốc",
        "session" => "phiên quét",
        "plan" => "kế hoạch",
        "quarantine entry" | "entry" => "mục cách ly",
        "permanent-delete batch" => "lô xóa vĩnh viễn",
        "transaction" => "giao dịch",
        _ => kind,
    };
    Uuid::parse_str(value).map_err(|error| format!("Mã {label} không hợp lệ: {error}"))
}

fn require_exact(actual: &str, expected: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("Xác nhận phải chính xác là {expected}."))
    }
}

fn manifest_path(database: &Database) -> PathBuf {
    database.path().with_extension("transactions.jsonl")
}

fn permanent_delete_manifest_path(database: &Database) -> PathBuf {
    database.path().with_extension("permanent-delete.jsonl")
}

fn database_file_bytes(path: &Path) -> std::io::Result<u64> {
    let mut total = file_bytes(path)?;
    total = total.saturating_add(file_bytes(&PathBuf::from(format!(
        "{}-wal",
        path.display()
    )))?);
    total = total.saturating_add(file_bytes(&PathBuf::from(format!(
        "{}-shm",
        path.display()
    )))?);
    Ok(total)
}

fn file_bytes(path: &Path) -> std::io::Result<u64> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

fn directory_bytes(path: &Path) -> std::io::Result<u64> {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut total = 0_u64;
    for entry in entries {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_file() {
            total = total.saturating_add(entry.metadata()?.len());
        } else if file_type.is_dir() {
            total = total.saturating_add(directory_bytes(&entry.path())?);
        }
    }
    Ok(total)
}

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

async fn run_blocking<T, F>(operation: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|error| format!("Tác vụ nền bị gián đoạn: {error}"))?
}
