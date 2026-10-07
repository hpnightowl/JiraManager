mod tasks;

use tasks::TaskStore;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // Register shared state so commands can ask for `State<TaskStore>`.
        .manage(TaskStore::default())
        .invoke_handler(tauri::generate_handler![
            tasks::add_task,
            tasks::list_tasks,
            tasks::toggle_task,
            tasks::delete_task,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
