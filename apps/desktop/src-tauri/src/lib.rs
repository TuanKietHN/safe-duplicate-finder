//! Reusable Tauri adapter surface shared by the desktop binary and integration tests.

pub mod commands;
pub mod events;
pub mod state;

/// Attach every validated desktop command to a Tauri builder.
pub fn with_commands<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        commands::engine_status,
        commands::list_projects,
        commands::create_project,
        commands::update_project,
        commands::set_project_workers,
        commands::archive_project,
        commands::get_filter_config,
        commands::save_filter_config,
        commands::add_root,
        commands::list_roots,
        commands::remove_root,
        commands::start_scan,
        commands::resume_scan,
        commands::scan_status,
        commands::control_scan,
        commands::next_scan_event,
        commands::list_results,
        commands::create_plan,
        commands::latest_plan_for_session,
        commands::latest_plan_context,
        commands::dry_run,
        commands::apply_quarantine,
        commands::list_quarantine,
        commands::prepare_permanent_delete,
        commands::execute_permanent_delete,
        commands::restore_entry,
        commands::inspect_recovery,
        commands::reconcile_transaction,
        commands::export_report,
        commands::list_file_history,
        commands::storage_overview,
        commands::optimize_storage,
        commands::cleanup_old_logs,
    ])
}
