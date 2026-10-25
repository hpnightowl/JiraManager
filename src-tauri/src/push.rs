//! "Push to Jira": create the issue if the task has none yet, then log its finished time as
//! worklogs. Safe to repeat: finished time that was already logged is remembered and skipped.
//!
//! Known limit (Jira has no idempotency keys): if the app is killed in the instant between
//! Jira answering "created" and us saving that answer, a retry would create a second issue.
//! We save the answer immediately to keep that window tiny, and the UI shows a dry-run
//! summary and disables the button while pushing.

use crate::adf::text_to_adf;
use crate::error::AppError;
use crate::fields::{self, FieldSpec};
use crate::jira::{load_client, IssueType, JiraClient, JiraProject, JiraUser};
use crate::secrets::SecretStore;
use crate::tasks::TaskStore;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tauri::State;

/// Jira rejects worklogs shorter than one minute.
const MIN_WORKLOG_SECS: i64 = 60;
const ALLOWED_ROUNDING_MINUTES: &[u32] = &[0, 5, 10, 15, 30, 60];

// ---------------------------------------------------------------------------------------
// Database side
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct PendingEntry {
    pub entry_id: i64,
    pub started_at: i64,
    pub seconds: i64,
}

impl TaskStore {
    /// Finished time entries of a task that are not in Jira yet, oldest first.
    pub fn pending_entries(&self, task_id: i64) -> Result<Vec<PendingEntry>, AppError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, started_at, ended_at - started_at FROM time_entries
             WHERE task_id = ?1 AND ended_at IS NOT NULL AND jira_worklog_id IS NULL
             ORDER BY started_at",
        )?;
        let entries = stmt
            .query_map(params![task_id], |row| {
                Ok(PendingEntry { entry_id: row.get(0)?, started_at: row.get(1)?, seconds: row.get(2)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(entries)
    }

    pub fn mark_entry_pushed(&self, entry_id: i64, worklog_id: &str, now: i64) -> Result<(), AppError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE time_entries SET jira_worklog_id = ?2, pushed_at = ?3 WHERE id = ?1",
            params![entry_id, worklog_id, now],
        )?;
        Ok(())
    }

    /// The issue now exists in Jira: remember its key, type and parent in one go (so a crash
    /// can't leave a half-updated task), and start counting Jira-logged time from zero.
    pub fn record_created(
        &self,
        task_id: i64,
        key: &str,
        issue_type: Option<&str>,
        parent_key: Option<&str>,
    ) -> Result<(), AppError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE tasks SET jira_key = ?2, issue_type = COALESCE(?3, issue_type), parent_key = ?4,
                    pending_parent_key = NULL, jira_logged_seconds = 0
             WHERE id = ?1 AND jira_key IS NULL",
            params![task_id, key, issue_type, parent_key],
        )?;
        if changed == 0 {
            return Err(AppError::Invalid("This task is already linked to a Jira issue".into()));
        }
        // The ledger behind "stories created with ProjectJira". It keeps its own copy of the title,
        // so hiding or deleting the task later doesn't change the count.
        conn.execute(
            "INSERT OR IGNORE INTO created_issues (jira_key, title, issue_type, created_at)
             VALUES (?1, (SELECT title FROM tasks WHERE id = ?2), ?3, CAST(strftime('%s', 'now') AS INTEGER))",
            params![key, task_id, issue_type],
        )?;
        Ok(())
    }

    /// Jira accepted the staged parent: it is no longer "pending".
    pub fn commit_parent(&self, task_id: i64, parent_key: &str) -> Result<(), AppError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE tasks SET parent_key = ?2, pending_parent_key = NULL WHERE id = ?1",
            params![task_id, parent_key],
        )?;
        Ok(())
    }

    /// Keep the on-screen "logged in Jira" total right immediately after a push
    /// (the next Refresh replaces it with Jira's own number).
    pub fn add_logged_seconds(&self, task_id: i64, seconds: i64) -> Result<(), AppError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE tasks SET jira_logged_seconds = COALESCE(jira_logged_seconds, 0) + ?2 WHERE id = ?1",
            params![task_id, seconds],
        )?;
        Ok(())
    }

    /// Link a task to an issue key without any other details (tests use this as a shortcut;
    /// the real push uses `record_created`).
    #[cfg(test)]
    pub fn link_jira_key(&self, task_id: i64, key: &str) -> Result<(), AppError> {
        let conn = self.conn()?;
        let changed = conn.execute(
            "UPDATE tasks SET jira_key = ?2 WHERE id = ?1 AND jira_key IS NULL",
            params![task_id, key],
        )?;
        if changed == 0 {
            return Err(AppError::Invalid("This task is already linked to a Jira issue".into()));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------
// Preview (dry run)
// ---------------------------------------------------------------------------------------

/// Round `seconds` UP to a whole number of `minutes`-minute blocks. 0 minutes = no rounding.
pub fn round_up(seconds: i64, minutes: u32) -> i64 {
    if minutes == 0 {
        return seconds;
    }
    let block = i64::from(minutes) * 60;
    (seconds + block - 1) / block * block
}

#[derive(Debug, Serialize, PartialEq)]
pub struct PreviewEntry {
    pub entry_id: i64,
    pub started_at: i64,
    /// What you actually worked.
    pub seconds: i64,
    /// What will be logged after rounding.
    pub log_seconds: i64,
    /// Why this entry will NOT be logged, if so.
    pub skipped: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct PushPreview {
    pub task_id: i64,
    pub title: String,
    pub jira_key: Option<String>,
    /// A parent chosen here that will be sent to Jira on Push.
    pub pending_parent_key: Option<String>,
    pub entries: Vec<PreviewEntry>,
    pub total_log_seconds: i64,
}

fn check_rounding(minutes: u32) -> Result<(), AppError> {
    if ALLOWED_ROUNDING_MINUTES.contains(&minutes) {
        Ok(())
    } else {
        Err(AppError::Invalid(format!("Rounding must be one of {ALLOWED_ROUNDING_MINUTES:?} minutes")))
    }
}

fn build_entries(pending: Vec<PendingEntry>, round_minutes: u32) -> Vec<PreviewEntry> {
    pending
        .into_iter()
        .map(|e| {
            let tiny = e.seconds < MIN_WORKLOG_SECS;
            PreviewEntry {
                entry_id: e.entry_id,
                started_at: e.started_at,
                seconds: e.seconds,
                log_seconds: if tiny { e.seconds } else { round_up(e.seconds, round_minutes) },
                skipped: tiny.then(|| "shorter than 1 minute (Jira's minimum)".to_string()),
            }
        })
        .collect()
}

pub fn preview(store: &TaskStore, task_id: i64, round_minutes: u32) -> Result<PushPreview, AppError> {
    check_rounding(round_minutes)?;
    let task = store.find(task_id)?;
    let entries = build_entries(store.pending_entries(task_id)?, round_minutes);
    let total_log_seconds = entries.iter().filter(|e| e.skipped.is_none()).map(|e| e.log_seconds).sum();
    Ok(PushPreview {
        task_id,
        title: task.title,
        jira_key: task.jira_key,
        pending_parent_key: task.pending_parent_key,
        entries,
        total_log_seconds,
    })
}

// ---------------------------------------------------------------------------------------
// The push
// ---------------------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct PushRequest {
    pub task_id: i64,
    /// Needed only when the task has no Jira issue yet.
    pub project_key: Option<String>,
    pub issue_type_id: Option<String>,
    /// "Story", "Epic"...: kept so the list can group epics and stories without a refresh.
    #[serde(default)]
    pub issue_type_name: Option<String>,
    /// A reworded title (for example from the AI suggestion). Only used for a task that isn't
    /// in Jira yet: it becomes the issue summary and the task's new name.
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: String,
    /// Answers for the required fields, keyed by Jira field id.
    #[serde(default)]
    pub fields: Map<String, Value>,
    #[serde(default)]
    pub round_minutes: u32,
    /// Remember the answers as defaults for this project + issue type.
    #[serde(default)]
    pub remember: bool,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct PushResult {
    pub jira_key: String,
    pub created_issue: bool,
    pub worklogs_logged: usize,
    pub seconds_logged: i64,
    pub skipped: usize,
    /// True if a staged parent was sent to Jira (on create, or as an update).
    pub parent_set: bool,
    /// Things that went wrong after the issue existed (e.g. a worklog was refused).
    /// The work that did succeed is kept; pushing again retries only the rest.
    pub warnings: Vec<String>,
}

fn defaults_key(project_key: &str, issue_type_id: &str) -> String {
    format!("defaults.{project_key}.{issue_type_id}")
}

fn saved_defaults(store: &TaskStore, project_key: &str, issue_type_id: &str) -> Map<String, Value> {
    store
        .get_setting(&defaults_key(project_key, issue_type_id))
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub async fn push_task_impl(
    store: &TaskStore,
    client: &JiraClient,
    request: &PushRequest,
    now: i64,
) -> Result<PushResult, AppError> {
    check_rounding(request.round_minutes)?;
    let mut task = store.find(request.task_id)?;
    if task.jira_key.is_none() {
        if let Some(new_title) = request.title.as_deref().map(str::trim).filter(|t| !t.is_empty() && *t != task.title) {
            task = store.rename(task.id, new_title)?;
        }
    }
    let entries = build_entries(store.pending_entries(task.id)?, request.round_minutes);

    let mut warnings = Vec::new();

    // 1. Make sure the task has a Jira issue.
    let (jira_key, created_issue, mut parent_set) = match task.jira_key.clone() {
        Some(key) => (key, false, false),
        None => {
            let missing = |what: &str| AppError::Config(format!("Choose a {what} first"));
            let project = request.project_key.as_deref().ok_or_else(|| missing("project"))?;
            let issue_type = request.issue_type_id.as_deref().ok_or_else(|| missing("issue type"))?;

            // Ask Jira again for the field list: it is the source of truth for what is required.
            let specs = client.create_fields(project, issue_type).await?;
            let mut payload = fields::build_fields_payload(&specs, &request.fields).map_err(AppError::Invalid)?;

            // A parent chosen in the app (e.g. an epic) is sent together with the new issue.
            let parent = task.pending_parent_key.as_deref();
            if let Some(parent_key) = parent {
                if !same_project(parent_key, project) {
                    return Err(AppError::Invalid(format!(
                        "The parent {parent_key} belongs to a different project than {project}. \
                         Pick a parent from {project}, or clear it."
                    )));
                }
                if !fields::has_field(&specs, "parent") {
                    return Err(AppError::Invalid(
                        "This issue type can't have a parent in this project. Clear the parent first.".into(),
                    ));
                }
                payload.insert("parent".into(), json!({ "key": parent_key }));
            }

            let description = request.description.trim();
            if description.is_empty() && fields::description_required(&specs) {
                return Err(AppError::Invalid("This issue type requires a description".into()));
            }
            let description = (!description.is_empty()).then(|| text_to_adf(description));

            // Jira limits summaries to 255 characters.
            let summary: String = task.title.chars().take(255).collect();
            let key = client.create_issue(project, issue_type, &summary, description, payload).await?;
            store.record_created(task.id, &key, request.issue_type_name.as_deref(), parent)?; // save immediately

            if request.remember {
                remember_answers(store, project, issue_type, &specs, &request.fields);
            }
            (key, true, parent.is_some())
        }
    };

    // 1b. An existing issue that got a parent staged in the app: set it now. A failure here is a
    //     warning, not an error: it must not stop the time from being logged.
    if !created_issue {
        if let Some(parent_key) = task.pending_parent_key.as_deref() {
            if !same_project(parent_key, &jira_key) {
                warnings.push(format!("Parent {parent_key} is in a different project than {jira_key}; it was not set."));
            } else {
                match client.set_parent(&jira_key, parent_key).await {
                    Ok(()) => {
                        store.commit_parent(task.id, parent_key)?;
                        parent_set = true;
                    }
                    Err(e) => warnings.push(format!("Could not set the parent {parent_key} on {jira_key}: {e}")),
                }
            }
        }
    }

    // 2. Log finished, not-yet-logged time. Stop at the first failure: if Jira refuses one
    //    worklog (no permission, closed issue...), the next ones would fail the same way.
    let (mut logged, mut seconds_logged, mut skipped) = (0, 0, 0);
    for entry in &entries {
        if entry.skipped.is_some() {
            skipped += 1;
            continue;
        }
        match client.add_worklog(&jira_key, entry.started_at, entry.log_seconds).await {
            Ok(worklog_id) => {
                store.mark_entry_pushed(entry.entry_id, &worklog_id, now)?;
                store.add_logged_seconds(task.id, entry.log_seconds)?;
                logged += 1;
                seconds_logged += entry.log_seconds;
            }
            Err(e) => {
                warnings.push(format!("Could not log time on {jira_key}: {e}. Nothing else was logged; push again to retry."));
                break;
            }
        }
    }

    Ok(PushResult { jira_key, created_issue, worklogs_logged: logged, seconds_logged, skipped, parent_set, warnings })
}

/// "ABC-12" belongs to project "ABC". Jira only allows a parent from the same project.
fn same_project(issue_key: &str, project_key: &str) -> bool {
    // `project_key` may itself be an issue key ("ABC-7"): compare only the part before the dash.
    let project_of = |k: &str| k.rsplit_once('-').map_or(k.to_string(), |(p, _)| p.to_string());
    project_of(issue_key).eq_ignore_ascii_case(&project_of(project_key))
}

// ---------------------------------------------------------------------------------------
// Pushing many tasks at once
// ---------------------------------------------------------------------------------------

/// One set of answers (project, type, assignee, required fields...) applied to every selected
/// task that still needs an issue. Tasks already in Jira just get their time and staged parent.
#[derive(Debug, Deserialize)]
pub struct BulkPushRequest {
    pub task_ids: Vec<i64>,
    pub project_key: Option<String>,
    pub issue_type_id: Option<String>,
    pub issue_type_name: Option<String>,
    #[serde(default)]
    pub fields: Map<String, Value>,
    #[serde(default)]
    pub round_minutes: u32,
    #[serde(default)]
    pub remember: bool,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct BulkItem {
    pub task_id: i64,
    pub title: String,
    pub result: Option<PushResult>,
    pub error: Option<String>,
}

/// Push the tasks one after another. Each task succeeds or fails on its own: a refused issue
/// never undoes or blocks the others, and a repeat push only does what is still missing.
pub async fn push_many_impl(
    store: &TaskStore,
    client: &JiraClient,
    request: &BulkPushRequest,
    now: i64,
) -> Result<Vec<BulkItem>, AppError> {
    check_rounding(request.round_minutes)?;
    if request.task_ids.is_empty() {
        return Err(AppError::Invalid("Select at least one task".into()));
    }

    let mut items = Vec::new();
    for &task_id in &request.task_ids {
        let task = match store.find(task_id) {
            Ok(t) => t,
            Err(e) => {
                items.push(BulkItem { task_id, title: String::new(), result: None, error: Some(e.to_string()) });
                continue;
            }
        };
        if task.context_only {
            items.push(BulkItem { task_id, title: task.title, result: None, error: Some("This is a heading from Jira, not one of your tasks".into()) });
            continue;
        }
        // Each task uses ITS OWN drafted story; everything else is shared.
        let single = PushRequest {
            task_id,
            project_key: request.project_key.clone(),
            issue_type_id: request.issue_type_id.clone(),
            issue_type_name: request.issue_type_name.clone(),
            title: task.story_title.clone(),
            description: task.story_description.clone().unwrap_or_default(),
            fields: request.fields.clone(),
            round_minutes: request.round_minutes,
            remember: request.remember,
        };
        let outcome = push_task_impl(store, client, &single, now).await;
        let title = store.find(task_id).map(|t| t.title).unwrap_or(task.title);
        items.push(match outcome {
            Ok(result) => BulkItem { task_id, title, result: Some(result), error: None },
            Err(e) => BulkItem { task_id, title, result: None, error: Some(e.to_string()) },
        });
    }
    Ok(items)
}

fn remember_answers(store: &TaskStore, project: &str, issue_type: &str, specs: &[FieldSpec], answers: &Map<String, Value>) {
    let kept: Map<String, Value> = fields::required_fields(specs)
        .iter()
        .filter_map(|spec| answers.get(&spec.id).map(|v| (spec.id.clone(), v.clone())))
        .collect();
    // Best effort: failing to save defaults must never fail the push itself.
    if let Ok(json) = serde_json::to_string(&kept) {
        let _ = store.set_setting(&defaults_key(project, issue_type), &json);
    }
}

// ---------------------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------------------

#[tauri::command]
pub async fn list_jira_projects(
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<Vec<JiraProject>, AppError> {
    load_client(&store, &secrets)?.projects().await
}

#[tauri::command]
pub async fn list_issue_types(
    project_key: String,
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<Vec<IssueType>, AppError> {
    let types = load_client(&store, &secrets)?.issue_types(&project_key).await?;
    Ok(types.into_iter().filter(|t| !t.subtask).collect()) // sub-tasks need a parent: not supported
}

#[derive(Serialize)]
pub struct CreateForm {
    /// The fields the user must answer (already filtered).
    pub fields: Vec<FieldSpec>,
    /// Useful extras the project's create screen offers: assignee, priority, labels...
    pub optional: Vec<FieldSpec>,
    /// The logged-in Jira user, used as the default assignee.
    pub me: Option<JiraUser>,
    /// Does this issue type accept a parent (e.g. an epic) on creation?
    pub can_set_parent: bool,
    pub description_required: bool,
    /// Answers remembered from last time.
    pub saved: Map<String, Value>,
}

#[tauri::command]
pub async fn get_create_form(
    project_key: String,
    issue_type_id: String,
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<CreateForm, AppError> {
    let client = load_client(&store, &secrets)?;
    let specs = client.create_fields(&project_key, &issue_type_id).await?;
    Ok(CreateForm {
        fields: fields::required_fields(&specs).into_iter().cloned().collect(),
        optional: fields::optional_fields(&specs).into_iter().cloned().collect(),
        me: client.me().await.ok(), // best effort: without it the assignee just starts empty
        can_set_parent: fields::has_field(&specs, "parent"),
        description_required: fields::description_required(&specs),
        saved: saved_defaults(&store, &project_key, &issue_type_id),
    })
}

#[tauri::command]
pub async fn search_jira_users(
    project_key: String,
    query: String,
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<Vec<JiraUser>, AppError> {
    load_client(&store, &secrets)?.search_users(&project_key, query.trim()).await
}

#[tauri::command]
pub fn get_push_preview(task_id: i64, round_minutes: u32, store: State<'_, TaskStore>) -> Result<PushPreview, AppError> {
    preview(&store, task_id, round_minutes)
}

/// The dry run for several tasks at once (one preview per task).
#[tauri::command]
pub fn get_bulk_preview(task_ids: Vec<i64>, round_minutes: u32, store: State<'_, TaskStore>) -> Result<Vec<PushPreview>, AppError> {
    task_ids.iter().map(|&id| preview(&store, id, round_minutes)).collect()
}

#[tauri::command]
pub async fn push_tasks(
    request: BulkPushRequest,
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<Vec<BulkItem>, AppError> {
    let client = load_client(&store, &secrets)?;
    push_many_impl(&store, &client, &request, chrono::Utc::now().timestamp()).await
}

#[tauri::command]
pub async fn push_task(
    request: PushRequest,
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<PushResult, AppError> {
    let client = load_client(&store, &secrets)?;
    push_task_impl(&store, &client, &request, chrono::Utc::now().timestamp()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn rounding_goes_up_to_the_next_block() {
        assert_eq!(round_up(61, 0), 61);
        assert_eq!(round_up(61, 15), 900);
        assert_eq!(round_up(900, 15), 900);
        assert_eq!(round_up(901, 15), 1800);
        assert_eq!(round_up(100, 5), 300);
    }

    #[test]
    fn bad_rounding_values_are_refused() {
        let store = TaskStore::in_memory();
        let t = store.add("x").unwrap();
        assert!(preview(&store, t.id, 7).is_err());
    }

    /// A task with two finished sessions (15 min and 40 s) and one still running.
    fn task_with_time(store: &TaskStore) -> i64 {
        let t = store.add("Write the report").unwrap();
        store.start_timer(t.id, 1_000).unwrap();
        store.stop_timer(1_900, 1_900).unwrap(); // 900 s
        store.start_timer(t.id, 3_000).unwrap();
        store.stop_timer(3_040, 3_040).unwrap(); // 40 s: too short
        store.start_timer(t.id, 5_000).unwrap(); // still running: must not be pushed
        t.id
    }

    #[test]
    fn preview_skips_tiny_and_running_entries() {
        let store = TaskStore::in_memory();
        let id = task_with_time(&store);
        let p = preview(&store, id, 15).unwrap();
        assert_eq!(p.entries.len(), 2);
        assert_eq!(p.entries[0].log_seconds, 900);
        assert!(p.entries[1].skipped.is_some());
        assert_eq!(p.total_log_seconds, 900);
    }

    // --- full push against a fake Jira ---

    fn fields_response() -> Value {
        json!({ "total": 2, "fields": [
            { "required": true, "name": "Summary", "fieldId": "summary", "schema": {"type":"string","system":"summary"} },
            { "required": true, "name": "Team", "fieldId": "customfield_10050",
              "schema": {"type":"option","custom":"x:select"},
              "allowedValues": [{"id":"201","value":"Platform"}] }
        ]})
    }

    async fn fake_jira() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/createmeta/ABC/issuetypes/10001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fields_response()))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/rest/api/3/issue"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "1", "key": "ABC-7" })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/rest/api/3/issue/ABC-7/worklog"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "9001" })))
            .mount(&server)
            .await;
        server
    }

    fn new_issue_request(task_id: i64) -> PushRequest {
        serde_json::from_value(json!({
            "task_id": task_id, "project_key": "ABC", "issue_type_id": "10001",
            "description": "Details here", "fields": { "customfield_10050": "201" },
            "round_minutes": 15, "remember": true
        }))
        .unwrap()
    }

    fn requests_to(requests: &[wiremock::Request], http_method: &str, p: &str) -> Vec<Value> {
        requests
            .iter()
            .filter(|r| r.method.as_str() == http_method && r.url.path() == p)
            .map(|r| serde_json::from_slice(&r.body).unwrap_or(Value::Null))
            .collect()
    }

    #[tokio::test]
    async fn push_creates_the_issue_then_logs_time_and_is_repeatable() {
        let server = fake_jira().await;
        let client = JiraClient::new(&server.uri(), "me@example.com", "tok").unwrap();
        let store = TaskStore::in_memory();
        let id = task_with_time(&store);

        let result = push_task_impl(&store, &client, &new_issue_request(id), 9_000).await.unwrap();
        assert_eq!(
            result,
            PushResult { jira_key: "ABC-7".into(), created_issue: true, worklogs_logged: 1, seconds_logged: 900, skipped: 1, parent_set: false, warnings: vec![] }
        );
        assert_eq!(store.find(id).unwrap().jira_logged_seconds, Some(900)); // "logged in Jira" updated on screen

        // What we sent to Jira:
        let requests = server.received_requests().await.unwrap();
        let issue = &requests_to(&requests, "POST", "/rest/api/3/issue")[0]["fields"];
        assert_eq!(issue["project"]["key"], "ABC");
        assert_eq!(issue["issuetype"]["id"], "10001");
        assert_eq!(issue["summary"], "Write the report");
        assert_eq!(issue["customfield_10050"], json!({"id":"201"}));
        assert_eq!(issue["description"]["type"], "doc");
        let worklog = &requests_to(&requests, "POST", "/rest/api/3/issue/ABC-7/worklog")[0];
        assert_eq!(worklog["timeSpentSeconds"], 900);
        assert_eq!(worklog["started"], "1970-01-01T00:16:40.000+0000");

        // What we remembered:
        let task = store.find(id).unwrap();
        assert_eq!(task.jira_key.as_deref(), Some("ABC-7"));
        assert_eq!(task.unpushed_seconds, 40); // only the too-short entry is left
        assert_eq!(saved_defaults(&store, "ABC", "10001")["customfield_10050"], "201");

        // Pushing again: no new issue, no duplicate worklog.
        let again = push_task_impl(&store, &client, &new_issue_request(id), 9_100).await.unwrap();
        assert!(!again.created_issue);
        assert_eq!(again.worklogs_logged, 0);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests_to(&requests, "POST", "/rest/api/3/issue").len(), 1);
        assert_eq!(requests_to(&requests, "POST", "/rest/api/3/issue/ABC-7/worklog").len(), 1);
    }

    #[tokio::test]
    async fn missing_required_field_stops_before_anything_is_created() {
        let server = fake_jira().await;
        let client = JiraClient::new(&server.uri(), "me@example.com", "tok").unwrap();
        let store = TaskStore::in_memory();
        let id = task_with_time(&store);
        let mut request = new_issue_request(id);
        request.fields.clear();

        let err = push_task_impl(&store, &client, &request, 9_000).await.unwrap_err().to_string();
        assert!(err.contains("Please fill in: Team"), "got: {err}");
        let requests = server.received_requests().await.unwrap();
        assert!(requests_to(&requests, "POST", "/rest/api/3/issue").is_empty());
        assert!(store.find(id).unwrap().jira_key.is_none());
    }

    #[tokio::test]
    async fn linked_task_only_logs_time_and_a_refused_worklog_is_kept_for_retry() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/rest/api/3/issue/ABC-1/worklog"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let client = JiraClient::new(&server.uri(), "me@example.com", "tok").unwrap();
        let store = TaskStore::in_memory();
        let id = task_with_time(&store);
        store.link_jira_key(id, "ABC-1").unwrap();

        let request: PushRequest = serde_json::from_value(json!({ "task_id": id })).unwrap();
        let result = push_task_impl(&store, &client, &request, 9_000).await.unwrap();
        assert!(!result.created_issue);
        assert_eq!(result.worklogs_logged, 0);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("denied access"), "got: {:?}", result.warnings);
        assert_eq!(store.find(id).unwrap().unpushed_seconds, 940); // nothing was marked pushed
    }

    /// Create screen that also offers Assignee and Parent.
    async fn fake_jira_with_parent() -> MockServer {
        let server = fake_jira().await; // same endpoints; add the extra fields by mounting a first-match mock
        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/createmeta/ABC/issuetypes/10002"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "total": 4, "fields": [
                { "required": true, "name": "Summary", "fieldId": "summary", "schema": {"type":"string","system":"summary"} },
                { "required": false, "name": "Assignee", "fieldId": "assignee", "schema": {"type":"user","system":"assignee"} },
                { "required": false, "name": "Parent", "fieldId": "parent", "schema": {"type":"issuelink","system":"parent"} },
                { "required": false, "name": "Priority", "fieldId": "priority", "schema": {"type":"priority","system":"priority"},
                  "hasDefaultValue": true, "allowedValues": [{"id":"3","name":"Medium"}] }
            ]})))
            .mount(&server)
            .await;
        server
    }

    fn story_request(task_id: i64) -> PushRequest {
        serde_json::from_value(json!({
            "task_id": task_id, "project_key": "ABC", "issue_type_id": "10002", "issue_type_name": "Story",
            "fields": { "assignee": "me-1", "priority": "3" }
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn new_issue_gets_assignee_and_the_staged_parent() {
        let server = fake_jira_with_parent().await;
        let client = JiraClient::new(&server.uri(), "me@example.com", "tok").unwrap();
        let store = TaskStore::in_memory();
        let t = store.add("Story under an epic").unwrap();
        store.set_pending_parent(t.id, Some("ABC-1")).unwrap();

        let result = push_task_impl(&store, &client, &story_request(t.id), 9_000).await.unwrap();
        assert!(result.created_issue && result.parent_set);

        let requests = server.received_requests().await.unwrap();
        let fields = &requests_to(&requests, "POST", "/rest/api/3/issue")[0]["fields"];
        assert_eq!(fields["assignee"], json!({"accountId":"me-1"}));
        assert_eq!(fields["priority"], json!({"id":"3"}));
        assert_eq!(fields["parent"], json!({"key":"ABC-1"}));

        let saved = store.find(t.id).unwrap();
        assert_eq!(saved.parent_key.as_deref(), Some("ABC-1"));
        assert_eq!(saved.pending_parent_key, None);
        assert_eq!(saved.issue_type.as_deref(), Some("Story"));
        assert_eq!(saved.jira_logged_seconds, Some(0));
    }

    #[tokio::test]
    async fn parent_from_another_project_is_refused_before_anything_is_created() {
        let server = fake_jira_with_parent().await;
        let client = JiraClient::new(&server.uri(), "me@example.com", "tok").unwrap();
        let store = TaskStore::in_memory();
        let t = store.add("x").unwrap();
        store.set_pending_parent(t.id, Some("OTHER-1")).unwrap();

        let err = push_task_impl(&store, &client, &story_request(t.id), 9_000).await.unwrap_err().to_string();
        assert!(err.contains("different project"), "got: {err}");
        let requests = server.received_requests().await.unwrap();
        assert!(requests_to(&requests, "POST", "/rest/api/3/issue").is_empty());
    }

    #[tokio::test]
    async fn staged_parent_is_set_on_an_existing_issue_and_a_failure_does_not_block_time() {
        let server = MockServer::start().await;
        Mock::given(method("PUT")).and(path("/rest/api/3/issue/ABC-2")).respond_with(ResponseTemplate::new(204)).mount(&server).await;
        Mock::given(method("POST")).and(path("/rest/api/3/issue/ABC-2/worklog"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "1" }))).mount(&server).await;
        let client = JiraClient::new(&server.uri(), "me@example.com", "tok").unwrap();
        let store = TaskStore::in_memory();
        let id = task_with_time(&store);
        store.link_jira_key(id, "ABC-2").unwrap();
        store.set_pending_parent(id, Some("ABC-1")).unwrap();

        let request: PushRequest = serde_json::from_value(json!({ "task_id": id })).unwrap();
        let result = push_task_impl(&store, &client, &request, 9_000).await.unwrap();
        assert!(result.parent_set && !result.created_issue);
        assert_eq!(result.worklogs_logged, 1);
        let t = store.find(id).unwrap();
        assert_eq!((t.parent_key.as_deref(), t.pending_parent_key.as_deref()), (Some("ABC-1"), None));
        assert_eq!(t.jira_logged_seconds, Some(900));

        // Now Jira refuses the parent change: the warning is reported, the time is still logged.
        let server = MockServer::start().await;
        Mock::given(method("PUT")).respond_with(ResponseTemplate::new(403)).mount(&server).await;
        Mock::given(method("POST")).and(path("/rest/api/3/issue/ABC-3/worklog"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "2" }))).mount(&server).await;
        let client = JiraClient::new(&server.uri(), "me@example.com", "tok").unwrap();
        let store = TaskStore::in_memory(); // fresh store: only one timer may run at a time
        let id = task_with_time(&store);
        store.link_jira_key(id, "ABC-3").unwrap();
        store.set_pending_parent(id, Some("ABC-1")).unwrap();
        let request: PushRequest = serde_json::from_value(json!({ "task_id": id })).unwrap();
        let result = push_task_impl(&store, &client, &request, 9_000).await.unwrap();
        assert!(!result.parent_set);
        assert_eq!(result.worklogs_logged, 1);
        assert!(result.warnings[0].contains("Could not set the parent"), "got: {:?}", result.warnings);
        assert_eq!(store.find(id).unwrap().pending_parent_key.as_deref(), Some("ABC-1")); // still staged
    }

    // --- bulk push ---

    /// One finished 10-minute session for a task.
    fn finished_work(store: &TaskStore, task_id: i64, start: i64) {
        store.start_timer(task_id, start).unwrap();
        store.stop_timer(start + 600, start + 600).unwrap();
    }

    fn bulk(task_ids: Vec<i64>) -> BulkPushRequest {
        serde_json::from_value(json!({
            "task_ids": task_ids, "project_key": "ABC", "issue_type_id": "10001", "issue_type_name": "Story",
            "fields": { "customfield_10050": "201" }
        }))
        .unwrap()
    }

    /// A Jira that hands out ABC-7, ABC-8, ... for each created issue.
    async fn counting_jira() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/createmeta/ABC/issuetypes/10001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fields_response()))
            .mount(&server)
            .await;
        for n in [7, 8] {
            Mock::given(method("POST"))
                .and(path("/rest/api/3/issue"))
                .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": n.to_string(), "key": format!("ABC-{n}") })))
                .up_to_n_times(1)
                .mount(&server)
                .await;
        }
        Mock::given(method("POST"))
            .and(wiremock::matchers::path_regex(r"^/rest/api/3/issue/ABC-\d+/worklog$"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "1" })))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn bulk_push_creates_each_issue_from_its_own_story_and_logs_existing_ones() {
        let server = counting_jira().await;
        let client = JiraClient::new(&server.uri(), "me@example.com", "tok").unwrap();
        let store = TaskStore::in_memory();
        let a = store.add("rough a").unwrap();
        let b = store.add("rough b").unwrap();
        let c = store.add("already there").unwrap();
        store.link_jira_key(c.id, "ABC-2").unwrap();
        store.save_story(a.id, Some("Better A"), Some("Story for A.\n\nAcceptance criteria:\n- A works"), "ai").unwrap();
        finished_work(&store, a.id, 1_000);
        finished_work(&store, c.id, 5_000);

        let items = push_many_impl(&store, &client, &bulk(vec![a.id, b.id, c.id]), 9_000).await.unwrap();

        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|i| i.error.is_none()), "{items:?}");
        assert_eq!(items[0].title, "Better A"); // the drafted title replaced the rough one
        assert_eq!(items[0].result.as_ref().unwrap().jira_key, "ABC-7");
        assert_eq!(items[1].result.as_ref().unwrap().jira_key, "ABC-8");
        assert!(items[0].result.as_ref().unwrap().created_issue && !items[2].result.as_ref().unwrap().created_issue);
        assert_eq!(items[2].result.as_ref().unwrap().worklogs_logged, 1);

        let requests = server.received_requests().await.unwrap();
        let created = requests_to(&requests, "POST", "/rest/api/3/issue");
        assert_eq!(created.len(), 2);
        assert_eq!(created[0]["fields"]["summary"], "Better A");
        // "Story for A." paragraph, "Acceptance criteria:" paragraph, then the criteria as a real list
        assert_eq!(created[0]["fields"]["description"]["content"][2]["type"], "bulletList");
        assert_eq!(created[1]["fields"]["summary"], "rough b"); // no draft: the task's own title, no description
        assert!(created[1]["fields"].get("description").is_none());
        assert_eq!(created[1]["fields"]["customfield_10050"], json!({"id":"201"})); // shared answer applied to both
    }

    #[tokio::test]
    async fn one_failing_task_does_not_stop_or_undo_the_others() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/createmeta/ABC/issuetypes/10001"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fields_response()))
            .mount(&server)
            .await;
        // The first create is refused by Jira, the second works.
        Mock::given(method("POST"))
            .and(path("/rest/api/3/issue"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({ "errors": { "summary": "Summary is too long." } })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/rest/api/3/issue"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "9", "key": "ABC-9" })))
            .mount(&server)
            .await;
        let client = JiraClient::new(&server.uri(), "me@example.com", "tok").unwrap();
        let store = TaskStore::in_memory();
        let bad = store.add("bad").unwrap();
        let good = store.add("good").unwrap();

        let items = push_many_impl(&store, &client, &bulk(vec![bad.id, good.id]), 9_000).await.unwrap();

        assert!(items[0].error.as_deref().unwrap().contains("Summary is too long."));
        assert!(store.find(bad.id).unwrap().jira_key.is_none()); // nothing half-saved: it can be pushed again
        assert_eq!(items[1].result.as_ref().unwrap().jira_key, "ABC-9");
        assert_eq!(store.find(good.id).unwrap().jira_key.as_deref(), Some("ABC-9"));
    }

    #[tokio::test]
    async fn bulk_push_needs_a_selection_and_valid_rounding() {
        let server = MockServer::start().await;
        let client = JiraClient::new(&server.uri(), "me@example.com", "tok").unwrap();
        let store = TaskStore::in_memory();
        assert!(push_many_impl(&store, &client, &bulk(vec![]), 0).await.is_err());
        let t = store.add("x").unwrap();
        let mut request = bulk(vec![t.id]);
        request.round_minutes = 7;
        assert!(push_many_impl(&store, &client, &request, 0).await.is_err());
    }

    #[tokio::test]
    async fn a_reworded_title_becomes_the_issue_summary_and_the_task_name() {
        let server = fake_jira().await;
        let client = JiraClient::new(&server.uri(), "me@example.com", "tok").unwrap();
        let store = TaskStore::in_memory();
        let id = task_with_time(&store);
        let mut request = new_issue_request(id);
        request.title = Some("  Write the quarterly report  ".into());

        push_task_impl(&store, &client, &request, 9_000).await.unwrap();

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests_to(&requests, "POST", "/rest/api/3/issue")[0]["fields"]["summary"], "Write the quarterly report");
        assert_eq!(store.find(id).unwrap().title, "Write the quarterly report");
    }

    #[test]
    fn same_project_compares_keys_and_project_names() {
        assert!(same_project("ABC-12", "ABC") && same_project("ABC-12", "ABC-7") && same_project("abc-1", "ABC"));
        assert!(!same_project("OTHER-1", "ABC"));
    }

    #[test]
    fn a_task_cannot_be_linked_twice() {
        let store = TaskStore::in_memory();
        let t = store.add("x").unwrap();
        store.link_jira_key(t.id, "ABC-1").unwrap();
        assert!(store.link_jira_key(t.id, "ABC-2").is_err());
    }
}
