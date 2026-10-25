//! Story drafts. Every task can carry a drafted story (title + description with acceptance
//! criteria). Push sends the draft; the AI (or a template) writes it; you can edit it by hand.
//! The drafts live in the database, so they survive closing a dialog.

use crate::error::AppError;
use crate::llm::{self, template_improvement, Improvement, OllamaClient};
use crate::tasks::{Task, TaskStore};
use rusqlite::params;
use serde::Serialize;
use tauri::State;

const MAX_DRAFT_DESCRIPTION: usize = 4000;

/// Description + an "Acceptance criteria:" bullet list, as the plain text the Push form shows.
pub fn compose_description(imp: &Improvement) -> String {
    let mut text = imp.description.trim().to_string();
    if !imp.acceptance_criteria.is_empty() {
        let bullets: Vec<String> = imp.acceptance_criteria.iter().map(|c| format!("- {c}")).collect();
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&format!("Acceptance criteria:\n{}", bullets.join("\n")));
    }
    text
}

impl TaskStore {
    /// Save (or clear, when both are blank) the story draft of a task that isn't in Jira yet.
    pub fn save_story(&self, task_id: i64, title: Option<&str>, description: Option<&str>, source: &str) -> Result<Task, AppError> {
        let task = self.find(task_id)?;
        if task.context_only {
            return Err(AppError::Invalid("This is a parent heading from Jira, not one of your tasks".into()));
        }
        if task.jira_key.is_some() {
            return Err(AppError::Invalid("This task is already in Jira, so it has no story to draft.".into()));
        }
        let title = title.map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|t| !t.is_empty());
        let title = title.map(|t| t.chars().take(255).collect::<String>());
        let description = description
            .map(|d| d.trim().chars().take(MAX_DRAFT_DESCRIPTION).collect::<String>())
            .filter(|d| !d.is_empty());
        let source = (title.is_some() || description.is_some()).then_some(source);

        let conn = self.conn()?;
        conn.execute(
            "UPDATE tasks SET story_title = ?2, story_description = ?3, story_source = ?4 WHERE id = ?1",
            params![task_id, title, description, source],
        )?;
        drop(conn);
        self.find(task_id)
    }

    /// What to feed the AI for this task: its title and, when it came from merge requests,
    /// their descriptions and commit subjects.
    pub fn story_inputs(&self, task_id: i64) -> Result<(String, String), AppError> {
        let task = self.find(task_id)?;
        // Checked here, before any AI call, so we don't spend seconds writing a story we can't keep.
        if task.context_only || task.jira_key.is_some() {
            return Err(AppError::Invalid(format!("\"{}\" is already in Jira, so it needs no story", task.title)));
        }
        let mut details = self.mr_context(task_id)?;
        if details.is_empty() {
            details = task.story_description.clone().unwrap_or_default();
        }
        Ok((task.story_title.clone().unwrap_or(task.title), details))
    }
}

/// Draft one story with the local model and save it.
pub async fn generate_one(store: &TaskStore, client: &OllamaClient, task_id: i64) -> Result<Improvement, AppError> {
    let (title, details) = store.story_inputs(task_id)?;
    let improvement = client.improve(&title, &details).await?;
    store.save_story(task_id, Some(&improvement.title), Some(&compose_description(&improvement)), "ai")?;
    Ok(improvement)
}

/// Draft one story from the no-AI template and save it.
pub fn generate_template(store: &TaskStore, task_id: i64) -> Result<Improvement, AppError> {
    let (title, details) = store.story_inputs(task_id)?;
    let improvement = template_improvement(&title, &details);
    store.save_story(task_id, Some(&improvement.title), Some(&compose_description(&improvement)), "template")?;
    Ok(improvement)
}

#[derive(Debug, Serialize, PartialEq)]
pub struct StoryFailure {
    pub task_id: i64,
    pub message: String,
}

#[derive(Debug, Serialize, PartialEq, Default)]
pub struct GenerateReport {
    pub done: Vec<i64>,
    pub failed: Vec<StoryFailure>,
    /// Set when the AI itself is unavailable: continuing would fail every remaining task.
    pub stopped: Option<String>,
}

/// Problems with the AI setup (not with one task): stop instead of failing every task the same way.
pub(crate) fn is_setup_problem(e: &AppError) -> bool {
    matches!(e, AppError::Llm(m) if m.contains("Can't reach") || m.contains("isn't installed") || m.contains("took too long"))
        || matches!(e, AppError::Config(_))
}

/// Draft stories for many tasks, one after another. A task that fails doesn't stop the others.
pub async fn generate_many(store: &TaskStore, client: Option<&OllamaClient>, task_ids: &[i64]) -> GenerateReport {
    let mut report = GenerateReport::default();
    for &id in task_ids {
        let result = match client {
            Some(c) => generate_one(store, c, id).await.map(|_| ()),
            None => generate_template(store, id).map(|_| ()),
        };
        match result {
            Ok(()) => report.done.push(id),
            Err(e) if is_setup_problem(&e) => {
                report.stopped = Some(e.to_string());
                break;
            }
            Err(e) => report.failed.push(StoryFailure { task_id: id, message: e.to_string() }),
        }
    }
    report
}

// --- commands ----------------------------------------------------------------------------

/// Save a story edited by hand ("manual"), or clear it with blank title and description.
#[tauri::command]
pub fn save_story_draft(task_id: i64, title: String, description: String, store: State<'_, TaskStore>) -> Result<Task, AppError> {
    store.save_story(task_id, Some(&title), Some(&description), "manual")
}

#[tauri::command]
pub async fn generate_story(task_id: i64, use_template: bool, store: State<'_, TaskStore>) -> Result<Improvement, AppError> {
    if use_template {
        return generate_template(&store, task_id);
    }
    let client = llm::client_from_settings(&store)?; // the database lock is released before the await
    generate_one(&store, &client, task_id).await
}

#[tauri::command]
pub async fn generate_stories(task_ids: Vec<i64>, use_template: bool, store: State<'_, TaskStore>) -> Result<GenerateReport, AppError> {
    if use_template {
        return Ok(generate_many(&store, None, &task_ids).await);
    }
    let client = llm::client_from_settings(&store)?;
    Ok(generate_many(&store, Some(&client), &task_ids).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn imp() -> Improvement {
        Improvement {
            title: "Add search".into(),
            description: "Users can search.".into(),
            acceptance_criteria: vec!["Results appear".into(), "Empty search is handled".into()],
            source: "ai".into(),
            model: Some("m".into()),
        }
    }

    #[test]
    fn the_draft_text_has_a_bullet_list_of_criteria() {
        assert_eq!(compose_description(&imp()), "Users can search.\n\nAcceptance criteria:\n- Results appear\n- Empty search is handled");
        let bare = Improvement { acceptance_criteria: vec![], ..imp() };
        assert_eq!(compose_description(&bare), "Users can search.");
    }

    #[test]
    fn drafts_are_saved_cleaned_and_cleared() {
        let store = TaskStore::in_memory();
        let t = store.add("rough").unwrap();
        let saved = store.save_story(t.id, Some("  Add   search \n now "), Some("  text  "), "manual").unwrap();
        assert_eq!(saved.story_title.as_deref(), Some("Add search now"));
        assert_eq!((saved.story_description.as_deref(), saved.story_source.as_deref()), (Some("text"), Some("manual")));

        let cleared = store.save_story(t.id, Some(" "), Some(""), "manual").unwrap();
        assert_eq!((cleared.story_title, cleared.story_description, cleared.story_source), (None, None, None));
    }

    #[test]
    fn tasks_already_in_jira_or_heading_rows_have_no_draft() {
        let store = TaskStore::in_memory();
        let t = store.add("x").unwrap();
        store.link_jira_key(t.id, "ABC-1").unwrap();
        assert!(store.save_story(t.id, Some("a"), None, "manual").is_err());
    }

    #[test]
    fn the_template_path_saves_a_draft_without_any_ai() {
        let store = TaskStore::in_memory();
        let t = store.add("fix   login loop.").unwrap();
        let s = generate_template(&store, t.id).unwrap();
        assert_eq!(s.source, "template");
        let saved = store.find(t.id).unwrap();
        assert_eq!(saved.story_title.as_deref(), Some("Fix login loop"));
        assert_eq!(saved.story_source.as_deref(), Some("template"));
        assert!(saved.story_description.unwrap().contains("Acceptance criteria:"));
    }

    fn ai_answer(title: &str) -> serde_json::Value {
        json!({ "message": { "role": "assistant", "content": json!({
            "title": title, "description": "Does the thing.", "acceptance_criteria": ["It works"]
        }).to_string() } })
    }

    #[tokio::test]
    async fn bulk_generation_saves_a_draft_per_task_using_mr_context() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/chat")).respond_with(ResponseTemplate::new(200).set_body_json(ai_answer("Add search"))).mount(&server).await;
        let client = OllamaClient::new(&server.uri(), "m").unwrap();

        let store = TaskStore::in_memory();
        let a = store.add("search").unwrap();
        let b = store.add("export").unwrap();
        let connection = store.add_connection("Work", "gitlab", "https://gitlab.example", None, None).unwrap().id;
        store.import_changes(connection, "gitlab", &[crate::git::GitChange {
            url: "https://gitlab.example/mr/1".into(), reference: "g/p!1".into(), title: "Build global search".into(),
            description: "Uses the new index".into(), state: "merged".into(), source_branch: "b".into(),
            created_at: 1, merged_at: None, role: "author".into(), commit_notes: Some("- add index".into()),
        }]).unwrap();
        let mr_task = store.list().unwrap().into_iter().find(|t| t.title == "Build global search").unwrap().id;

        let report = generate_many(&store, Some(&client), &[a.id, b.id, mr_task]).await;
        assert_eq!(report.done, vec![a.id, b.id, mr_task]);
        assert!(report.failed.is_empty() && report.stopped.is_none());
        assert_eq!(store.find(a.id).unwrap().story_title.as_deref(), Some("Add search"));

        // The MR's description and commit subjects went into the prompt for that task only.
        let bodies: Vec<String> = server.received_requests().await.unwrap().iter().map(|r| String::from_utf8_lossy(&r.body).to_string()).collect();
        assert!(bodies[2].contains("Uses the new index") && bodies[2].contains("add index"));
        assert!(!bodies[0].contains("Uses the new index"));
    }

    #[tokio::test]
    async fn bulk_generation_stops_when_the_ai_is_unavailable() {
        let client = OllamaClient::new("http://127.0.0.1:1", "m").unwrap(); // nothing listens here
        let store = TaskStore::in_memory();
        let ids: Vec<i64> = (0..3).map(|i| store.add(&format!("t{i}")).unwrap().id).collect();
        let report = generate_many(&store, Some(&client), &ids).await;
        assert!(report.done.is_empty() && report.failed.is_empty());
        assert!(report.stopped.unwrap().contains("Can't reach the AI server"));
    }

    #[tokio::test]
    async fn one_bad_task_does_not_stop_the_others() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/chat")).respond_with(ResponseTemplate::new(200).set_body_json(ai_answer("Ok title"))).mount(&server).await;
        let client = OllamaClient::new(&server.uri(), "m").unwrap();
        let store = TaskStore::in_memory();
        let good = store.add("good").unwrap();
        let linked = store.add("linked").unwrap();
        store.link_jira_key(linked.id, "ABC-1").unwrap();

        let report = generate_many(&store, Some(&client), &[linked.id, good.id]).await;
        assert_eq!(report.done, vec![good.id]);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].task_id, linked.id);
    }
}
