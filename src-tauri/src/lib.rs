mod commands;
pub mod database;
pub mod engine;
#[cfg(target_os = "macos")]
mod macos;
pub mod model;
pub mod script;
pub mod state;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(state::AppState::load(app.path().app_data_dir()?)?);
            #[cfg(target_os = "macos")]
            macos::install_quit_guard(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::save_connection,
            commands::delete_connection,
            commands::test_connection,
            commands::connect,
            commands::disconnect,
            commands::list_databases,
            commands::list_objects,
            commands::describe_table,
            commands::open_session,
            commands::close_session,
            commands::execute_script,
            commands::cancel_execution,
            commands::read_page,
            commands::apply_changes,
            commands::export_table,
            commands::export_preview,
            commands::pick_ca,
            commands::open_sql_file,
            commands::save_sql_file,
            commands::save_settings,
            commands::statement_at_cursor,
        ])
        .build(tauri::generate_context!())
        .expect("DBM failed to start")
        .run(|app, event| {
            // Route macOS Quit / Cmd-Q through the same unsaved-work guard as
            // the window close button. Once JS destroys the window, allow exit.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                if let Some(window) = app.get_webview_window("main") {
                    api.prevent_exit();
                    let _ = window.close();
                }
            }
        });
}
