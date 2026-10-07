mod db;
mod error;
mod tasks;

use tasks::TaskStore;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // `setup` runs once at startup, when we know where the OS wants app data.
        .setup(|app| {
            // Linux: ~/.local/share/com.projectjira.app   Windows: %APPDATA%\com.projectjira.app
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let store = TaskStore::open(dir.join("projectjira.db"))?;
            app.manage(store);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            tasks::add_task,
            tasks::list_tasks,
            tasks::toggle_task,
            tasks::delete_task,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
