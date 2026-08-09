#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Điểm khởi chạy desktop Tauri của Trình tìm tệp trùng lặp an toàn.

use safe_dedupe_desktop::state;
use tauri::Manager;

fn main() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let database = dedupe_store::Database::open(&data_dir.join("state.db"), &[])
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            let logs = dedupe_core::logging::init(&data_dir.join("logs"))
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            tracing::info!(database = %database.path().display(), "Đã khởi tạo bộ máy desktop");
            let state = state::EngineState::new(database, logs)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            app.manage(state);
            Ok(())
        });
    let result = safe_dedupe_desktop::with_commands(builder).run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("Không thể chạy Trình tìm tệp trùng lặp an toàn: {error}");
        std::process::exit(1);
    }
}
