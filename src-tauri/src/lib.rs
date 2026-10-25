mod adf;
mod aiproc;
mod db;
mod error;
mod fields;
mod git;
mod gitlab;
mod jira;
mod links;
mod llm;
mod localgit;
mod push;
mod secrets;
mod stats;
mod story;
mod suggest;
mod tasks;
mod timer;

use secrets::SecretStore;
use tasks::TaskStore;
use tauri::Manager;

/// Under WSLg no cursor theme is set and the pointer looks wrong, so use Adwaita if it is installed.
/// A theme the user already set is kept. Must run before any threads start.
fn use_a_normal_cursor() {
    #[cfg(target_os = "linux")]
    {
        let installed = std::path::Path::new("/usr/share/icons/Adwaita/cursors").is_dir();
        if installed && std::env::var_os("XCURSOR_THEME").is_none() {
            std::env::set_var("XCURSOR_THEME", "Adwaita");
            if std::env::var_os("XCURSOR_SIZE").is_none() {
                std::env::set_var("XCURSOR_SIZE", "24");
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use_a_normal_cursor();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Linux: ~/.local/share/com.projectjira.app   Windows: %APPDATA%\com.projectjira.app
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let store = TaskStore::open(dir.join("projectjira.db"))?;
            app.manage(store);
            app.manage(SecretStore::new(dir.clone()));
            let ai = aiproc::AiProcess::default();
            ai.set_dir(dir); // where it keeps ollama.pid and ai.log
            app.manage(ai);
            aiproc::autostart(app.handle()); // stopped again on exit, see below

            // Ctrl+C, `kill` or a WSL shutdown should stop the AI too, not only closing the window.
            let handle = app.handle().clone();
            let installed = ctrlc::set_handler(move || {
                eprintln!("ProjectJira: asked to quit by the system, shutting the AI down first");
                aiproc::shutdown(&handle);
                handle.exit(0);
            });
            if installed.is_err() {
                eprintln!("ProjectJira: could not install the quit-signal handler");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            tasks::add_task,
            tasks::list_tasks,
            tasks::toggle_task,
            tasks::delete_task,
            tasks::rename_task,
            tasks::set_task_parent,
            timer::tracked_between,
            timer::start_timer,
            timer::stop_timer,
            timer::heartbeat,
            timer::stale_timer,
            jira::get_jira_settings,
            jira::save_jira_settings,
            jira::clear_jira_token,
            jira::test_jira_connection,
            jira::fetch_jira_issues,
            jira::set_fetch_days,
            jira::list_hidden_issues,
            jira::unhide_issue,
            push::list_jira_projects,
            push::list_issue_types,
            push::get_create_form,
            push::search_jira_users,
            push::get_push_preview,
            push::push_task,
            llm::get_llm_settings,
            llm::save_llm_settings,
            llm::llm_status,
            llm::suggest_story,
            llm::template_story,
            llm::pull_model,
            push::get_bulk_preview,
            push::push_tasks,
            git::list_git_connections,
            git::add_git_connection,
            git::update_git_connection,
            git::remove_git_connection,
            git::clear_git_token,
            git::test_git_connection,
            git::import_git_changes,
            git::list_merge_requests,
            story::save_story_draft,
            story::generate_story,
            story::generate_stories,
            links::open_link,
            stats::get_story_stats,
            stats::refresh_reported_stories,
            suggest::suggest_stories,
            suggest::adopt_suggestions,
            aiproc::ai_status,
            aiproc::start_ai,
            aiproc::stop_ai,
            aiproc::set_ai_autostart,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // The app is closing: free the AI's memory and stop the Ollama we started.
            if let tauri::RunEvent::Exit = event {
                aiproc::shutdown(app);
            }
        });
}
