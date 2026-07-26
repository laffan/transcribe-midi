mod commands;
mod error;
mod state;

use tauri::Manager;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // `app_data_dir` resolves per-platform: ~/Library/Application Support/<id>
            // on macOS, the app container on iOS. Resolving it here rather than at each
            // call site means the store has one, known root.
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            app.manage(AppState::new(data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_projects,
            commands::create_project,
            commands::load_project,
            commands::save_project,
            commands::rename_project,
            commands::delete_project,
            commands::add_track,
            commands::delete_track,
            commands::projects_root,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Unplugged");
}
