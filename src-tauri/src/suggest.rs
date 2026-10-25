//! "I need N stories": pick the N pieces of work that deserve one, and draft each.
//!
//! Where the work comes from (all of it already in the local database, nothing new is fetched
//! except Jira comments, read-only):
//!   * **merge requests / branches** that have no Jira issue yet,
//!   * **defects** assigned to me in Jira (their description and comments are read for the draft),
//!   * **other tracked tasks** that have time on them but no issue.
//! Each candidate gets a score from the evidence (time spent, commits, merged, Jira comments).
//! The N best become suggestions; the local model (or the template, with no AI) drafts each one.
//! Nothing is created in Jira: "adopt" only saves the drafts on tasks, and Push stays explicit.

use crate::error::AppError;
use crate::jira::{load_client, JiraClient};
use crate::llm::{self, template_improvement, OllamaClient};
use crate::secrets::SecretStore;
use crate::story::{compose_description, is_setup_problem, StoryFailure};
use crate::tasks::{Task, TaskStore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::State;

pub const MAX_COUNT: usize = 20;
/// Reading comments costs one Jira request per defect, so only the most promising are read.
const MAX_DEFECTS_READ: usize = 30;
const COMMENTS_KEPT: usize = 5;
const COMMENTS_SCORED: usize = 10;
const GIVE_UP_AFTER_FAILURES: usize = 3;
const RELATED_PREFIX: &str = "Related Jira issue: ";

#[derive(Debug, Clone, Deserialize)]
pub struct SuggestOptions {
    /// How many stories the person needs.
    pub count: usize,
    pub merge_requests: bool,
    pub defects: bool,
    pub other_tasks: bool,
    /// Only consider merge requests and tasks from the last N days (0 = no limit).
    /// Defects are already limited by "How far back to fetch" in Settings.
    pub since_days: u32,
}

/// A piece of work that could become a story.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub task_id: i64,
    /// "merge_request", "defect" or "task".
    pub kind: &'static str,
    pub jira_key: Option<String>,
    pub title: String,
    pub score: i64,
    /// Short, human-readable reasons, shown on the card.
    pub evidence: Vec<String>,
    /// What the AI gets to read.
    pub context: String,
    comments: usize,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct Suggestion {
    /// The task this came from. Adopting a "defect" suggestion creates a NEW task instead.
    pub task_id: i64,
    pub kind: String,
    pub jira_key: Option<String>,
    pub source_title: String,
    pub score: i64,
    pub evidence: Vec<String>,
    pub title: String,
    pub description: String,
    /// "ai" or "template".
    pub source: String,
}

#[derive(Debug, Serialize, PartialEq, Default)]
pub struct SuggestReport {
    pub suggestions: Vec<Suggestion>,
    /// How many candidates there were before keeping the best N.
    pub available: usize,
    /// Set when the AI was unavailable: the rest was drafted from the template.
    pub stopped: Option<String>,
    /// Things worth telling the person (e.g. Jira comments couldn't be read).
    pub notes: Vec<String>,
}

struct MrStat {
    mrs: i64,
    merged: bool,
    commits: i64,
}

impl TaskStore {
    fn mr_stats(&self) -> Result<HashMap<i64, MrStat>, AppError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT task_id, COUNT(*), MAX(CASE WHEN state = 'merged' THEN 1 ELSE 0 END), GROUP_CONCAT(commit_notes, char(10))
             FROM merge_requests WHERE task_id IS NOT NULL GROUP BY task_id",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, Option<String>>(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|(id, mrs, merged, notes)| {
                let commits = notes.unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).count() as i64;
                (id, MrStat { mrs, merged: merged == 1, commits })
            })
            .collect())
    }
}

fn is_defect(task: &Task) -> bool {
    task.issue_type.as_deref().is_some_and(|t| {
        let t = t.to_lowercase();
        t.contains("bug") || t.contains("defect")
    })
}

/// Already turned into a story by an earlier "adopt".
fn already_adopted(tasks: &[Task], key: &str) -> bool {
    let line = format!("{RELATED_PREFIX}{key}");
    tasks.iter().any(|t| t.story_description.as_deref().is_some_and(|d| d.lines().any(|l| l.trim() == line)))
}

fn duration(seconds: i64) -> String {
    let minutes = (seconds + 30) / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn plural(n: i64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Every piece of work that could become a story, scored. No limit is applied here.
pub fn collect_candidates(store: &TaskStore, opts: &SuggestOptions, now: i64) -> Result<Vec<Candidate>, AppError> {
    let tasks = store.list()?;
    let mr = store.mr_stats()?;
    let cutoff = (opts.since_days > 0).then(|| now - i64::from(opts.since_days) * 86_400);
    let recent = |t: &Task| cutoff.map_or(true, |c| t.created_at >= c);
    let mut out = Vec::new();

    for t in tasks.iter().filter(|t| !t.context_only) {
        let tracked = t.tracked_seconds + t.running_since.map_or(0, |s| (now - s).max(0));
        match &t.jira_key {
            Some(key) => {
                if opts.defects && is_defect(t) && !already_adopted(&tasks, key) {
                    let logged = t.jira_logged_seconds.unwrap_or(0);
                    let mut evidence = vec![format!("Defect {key}{}", t.jira_status.as_deref().map(|s| format!(" ({s})")).unwrap_or_default())];
                    if tracked > 0 {
                        evidence.push(format!("{} tracked here", duration(tracked)));
                    }
                    if logged > 0 {
                        evidence.push(format!("{} logged in Jira", duration(logged)));
                    }
                    if t.done {
                        evidence.push("Resolved".into());
                    }
                    out.push(Candidate {
                        task_id: t.id,
                        kind: "defect",
                        jira_key: Some(key.clone()),
                        title: t.title.clone(),
                        score: tracked + logged + if t.done { 1200 } else { 0 } + 600,
                        evidence,
                        context: format!("Defect {key}: {}\nStatus: {}", t.title, t.jira_status.as_deref().unwrap_or("unknown")),
                        comments: 0,
                    });
                }
            }
            None => {
                if t.story_title.is_some() || t.story_description.is_some() || !recent(t) {
                    continue; // already has a draft, or too old
                }
                if let Some(stat) = mr.get(&t.id) {
                    if !opts.merge_requests {
                        continue;
                    }
                    let mut evidence = vec![format!(
                        "{}{}",
                        plural(stat.mrs, "merge request", "merge requests"),
                        if stat.merged { " (merged)" } else { "" }
                    )];
                    if stat.commits > 0 {
                        evidence.push(plural(stat.commits, "commit", "commits"));
                    }
                    if tracked > 0 {
                        evidence.push(format!("{} tracked", duration(tracked)));
                    }
                    out.push(Candidate {
                        task_id: t.id,
                        kind: "merge_request",
                        jira_key: None,
                        title: t.title.clone(),
                        score: tracked + 900 * stat.commits + if stat.merged { 1200 } else { 0 } + 300 * stat.mrs,
                        evidence,
                        context: store.mr_context(t.id)?,
                        comments: 0,
                    });
                } else if opts.other_tasks && tracked > 0 {
                    out.push(Candidate {
                        task_id: t.id,
                        kind: "task",
                        jira_key: None,
                        title: t.title.clone(),
                        score: tracked,
                        evidence: vec![format!("{} tracked", duration(tracked))],
                        context: String::new(),
                        comments: 0,
                    });
                }
            }
        }
    }
    Ok(out)
}

/// Best first; equal scores keep the older task first so the result is stable.
pub fn rank(mut candidates: Vec<Candidate>, count: usize) -> Vec<Candidate> {
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.task_id.cmp(&b.task_id)));
    candidates.truncate(count);
    candidates
}

fn clip(s: &str, max: usize) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// Read each promising defect's description and comments from Jira. They go into the AI's context
/// and add to the score (a defect people discussed is a defect that took real work).
/// Returns a note when Jira could not be read, instead of failing the whole suggestion.
async fn read_defect_comments(client: &JiraClient, candidates: &mut [Candidate]) -> Option<String> {
    let mut order: Vec<usize> = (0..candidates.len()).filter(|&i| candidates[i].kind == "defect").collect();
    order.sort_by_key(|&i| std::cmp::Reverse(candidates[i].score));
    let mut failures = 0;
    let mut first_error = None;

    for i in order.into_iter().take(MAX_DEFECTS_READ) {
        let key = candidates[i].jira_key.clone().unwrap_or_default();
        match client.issue_detail(&key).await {
            Ok(detail) => {
                let c = &mut candidates[i];
                if !detail.description.is_empty() {
                    c.context.push_str(&format!("\nDescription: {}", clip(&detail.description, 800)));
                }
                if !detail.comments.is_empty() {
                    c.context.push_str("\nComments:");
                    let skip = detail.comments.len().saturating_sub(COMMENTS_KEPT);
                    for comment in &detail.comments[skip..] {
                        c.context.push_str(&format!("\n- {}: {}", comment.author, clip(&comment.text, 300)));
                    }
                    c.comments = detail.comments.len();
                    c.score += 150 * detail.comments.len().min(COMMENTS_SCORED) as i64;
                    c.evidence.push(plural(detail.comments.len() as i64, "Jira comment", "Jira comments"));
                }
            }
            Err(e) => {
                failures += 1;
                first_error.get_or_insert_with(|| e.to_string());
                if failures >= GIVE_UP_AFTER_FAILURES {
                    break;
                }
            }
        }
    }
    first_error.map(|e| format!("Couldn't read some Jira comments ({e}). Those defects were ranked and drafted without them."))
}

/// The whole job. `jira` is only used to read defect comments; `ai` None means "use the template".
pub async fn suggest_impl(
    store: &TaskStore,
    jira: Option<&JiraClient>,
    ai: Option<&OllamaClient>,
    opts: &SuggestOptions,
    now: i64,
) -> Result<SuggestReport, AppError> {
    if opts.count == 0 || opts.count > MAX_COUNT {
        return Err(AppError::Invalid(format!("Ask for between 1 and {MAX_COUNT} stories")));
    }
    if !(opts.merge_requests || opts.defects || opts.other_tasks) {
        return Err(AppError::Invalid("Pick at least one place to look for work".into()));
    }

    let mut report = SuggestReport::default();
    let mut candidates = collect_candidates(store, opts, now)?;
    report.available = candidates.len();

    if candidates.iter().any(|c| c.kind == "defect") {
        match jira {
            Some(client) => report.notes.extend(read_defect_comments(client, &mut candidates).await),
            None => report.notes.push("Jira isn't connected, so defects were ranked and drafted without their comments.".into()),
        }
    }

    let mut ai = ai;
    for c in rank(candidates, opts.count) {
        let improvement = match ai {
            Some(client) => match client.improve(&c.title, &c.context).await {
                Ok(i) => i,
                Err(e) if is_setup_problem(&e) => {
                    report.stopped = Some(e.to_string());
                    ai = None;
                    template_improvement(&c.title, &c.context)
                }
                Err(e) => {
                    report.notes.push(format!("The AI couldn't draft \"{}\" ({e}); the template was used instead.", c.title));
                    template_improvement(&c.title, &c.context)
                }
            },
            None => template_improvement(&c.title, &c.context),
        };
        let mut description = compose_description(&improvement);
        if let (true, Some(key)) = (c.kind == "defect", &c.jira_key) {
            description.push_str(&format!("\n\n{RELATED_PREFIX}{key}"));
        }
        report.suggestions.push(Suggestion {
            task_id: c.task_id,
            kind: c.kind.to_string(),
            jira_key: c.jira_key,
            source_title: c.title.clone(),
            score: c.score,
            evidence: c.evidence,
            title: if improvement.title.is_empty() { c.title } else { improvement.title },
            description,
            source: improvement.source,
        });
    }
    Ok(report)
}

#[derive(Debug, Deserialize)]
pub struct AdoptItem {
    pub task_id: i64,
    pub kind: String,
    pub title: String,
    pub description: String,
    pub source: String,
}

#[derive(Debug, Serialize, PartialEq, Default)]
pub struct AdoptReport {
    /// Tasks that now carry a story draft (ready to select and push).
    pub task_ids: Vec<i64>,
    pub failed: Vec<StoryFailure>,
}

/// Save the chosen suggestions as story drafts. A defect already exists in Jira, so its story goes
/// on a NEW task that mentions it; everything else is saved on the task it came from.
pub fn adopt_impl(store: &TaskStore, items: &[AdoptItem]) -> AdoptReport {
    let mut report = AdoptReport::default();
    for item in items {
        let source = if item.source == "template" { "template" } else { "ai" };
        let result = if item.kind == "defect" {
            store.add(&item.title).and_then(|task| {
                store.save_story(task.id, Some(&item.title), Some(&item.description), source).map(|t| t.id).map_err(|e| {
                    let _ = store.remove(task.id); // don't leave an empty task behind
                    e
                })
            })
        } else {
            store.save_story(item.task_id, Some(&item.title), Some(&item.description), source).map(|t| t.id)
        };
        match result {
            Ok(id) => report.task_ids.push(id),
            Err(e) => report.failed.push(StoryFailure { task_id: item.task_id, message: e.to_string() }),
        }
    }
    report
}

// --- commands ----------------------------------------------------------------------------

#[tauri::command]
pub async fn suggest_stories(
    options: SuggestOptions,
    use_template: bool,
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<SuggestReport, AppError> {
    let jira = if options.defects { load_client(&store, &secrets).ok() } else { None };
    let (ai, ai_problem) = if use_template {
        (None, None)
    } else {
        match llm::client_from_settings(&store) {
            Ok(client) => (Some(client), None),
            Err(e) => (None, Some(e.to_string())),
        }
    };
    let mut report = suggest_impl(&store, jira.as_ref(), ai.as_ref(), &options, chrono::Utc::now().timestamp()).await?;
    if report.stopped.is_none() {
        report.stopped = ai_problem;
    }
    Ok(report)
}

#[tauri::command]
pub fn adopt_suggestions(items: Vec<AdoptItem>, store: State<'_, TaskStore>) -> Result<AdoptReport, AppError> {
    Ok(adopt_impl(&store, &items))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::GitChange;
    use crate::jira::JiraIssue;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn now() -> i64 {
        chrono::Utc::now().timestamp()
    }

    fn opts(count: usize) -> SuggestOptions {
        SuggestOptions { count, merge_requests: true, defects: true, other_tasks: true, since_days: 0 }
    }

    fn track(store: &TaskStore, task_id: i64, seconds: i64) {
        store.start_timer(task_id, 1_000).unwrap();
        store.stop_timer(1_000 + seconds, 1_000 + seconds).unwrap();
    }

    fn change(url: &str, title: &str, state: &str, commits: &str, created_at: i64) -> GitChange {
        GitChange {
            url: url.into(), reference: format!("g/p!{url}"), title: title.into(), description: format!("About {title}"),
            state: state.into(), source_branch: "b".into(), created_at, merged_at: None, role: "author".into(),
            commit_notes: Some(commits.into()),
        }
    }

    fn import(store: &TaskStore, changes: &[GitChange]) {
        let connection = store.add_connection("Work", "gitlab", "https://gitlab.example", None, None).unwrap().id;
        store.import_changes(connection, "gitlab", changes).unwrap();
    }

    fn task_id(store: &TaskStore, title: &str) -> i64 {
        store.list().unwrap().into_iter().find(|t| t.title == title).unwrap().id
    }

    fn defect(key: &str, title: &str, status: &str, resolved: bool) -> JiraIssue {
        JiraIssue { key: key.into(), summary: title.into(), status: Some(status.into()), issue_type: Some("Bug".into()), resolved, ..Default::default() }
    }

    #[test]
    fn the_best_n_are_kept_ranked_by_the_evidence_of_work() {
        let store = TaskStore::in_memory();
        import(&store, &[
            change("u1", "Build global search", "merged", "- add index\n- add ui\n- fix bug", now()),
            change("u2", "Tweak footer", "opened", "- move link", now()),
        ]);
        let spent = store.add("Investigate flaky build").unwrap();
        track(&store, spent.id, 3_600);
        let idle = store.add("No time spent here").unwrap(); // nothing tracked, nothing imported: not a candidate
        let _ = idle;

        let all = collect_candidates(&store, &opts(5), now()).unwrap();
        assert_eq!(all.len(), 3, "the idle task is not a candidate");

        let top = rank(all, 2);
        assert_eq!(top.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["Build global search", "Investigate flaky build"]);
        assert_eq!(top[0].kind, "merge_request");
        assert_eq!(top[0].evidence, ["1 merge request (merged)", "3 commits"]);
        assert_eq!(top[1].kind, "task");
        assert_eq!(top[1].evidence, ["1 h tracked"]);
    }

    #[test]
    fn sources_can_be_switched_off() {
        let store = TaskStore::in_memory();
        import(&store, &[change("u1", "An MR", "merged", "- x", now())]);
        let t = store.add("tracked").unwrap();
        track(&store, t.id, 600);
        store.upsert_jira_issues(&[defect("ABC-1", "Crash", "To Do", false)]).unwrap();

        let kinds = |o: SuggestOptions| {
            let mut k: Vec<&str> = collect_candidates(&store, &o, now()).unwrap().into_iter().map(|c| c.kind).collect();
            k.sort();
            k
        };
        assert_eq!(kinds(opts(5)), ["defect", "merge_request", "task"]);
        assert_eq!(kinds(SuggestOptions { defects: false, other_tasks: false, ..opts(5) }), ["merge_request"]);
        assert_eq!(kinds(SuggestOptions { merge_requests: false, other_tasks: false, ..opts(5) }), ["defect"]);
    }

    #[test]
    fn work_that_already_has_a_story_or_an_issue_is_not_suggested() {
        let store = TaskStore::in_memory();
        let drafted = store.add("drafted already").unwrap();
        track(&store, drafted.id, 3_600);
        store.save_story(drafted.id, Some("A story"), None, "manual").unwrap();
        // A Jira task that is not a defect already has its issue; a heading-only parent is never work.
        store.upsert_jira_issues(&[
            JiraIssue { key: "ABC-2".into(), summary: "Existing story".into(), issue_type: Some("Story".into()), ..Default::default() },
            JiraIssue { key: "ABC-3".into(), summary: "Child".into(), issue_type: Some("Story".into()),
                parent: Some(crate::jira::JiraParent { key: "ABC-1".into(), summary: "Epic".into(), issue_type: Some("Epic".into()) }), ..Default::default() },
        ]).unwrap();
        assert!(collect_candidates(&store, &opts(5), now()).unwrap().is_empty());
    }

    #[test]
    fn the_time_window_limits_merge_requests_and_tasks_but_not_defects() {
        let store = TaskStore::in_memory();
        import(&store, &[change("old", "Ancient MR", "merged", "- x", 1), change("new", "Fresh MR", "merged", "- x", now())]);
        store.upsert_jira_issues(&[defect("ABC-1", "Old defect", "Done", true)]).unwrap();

        let o = SuggestOptions { since_days: 30, ..opts(5) };
        let titles: Vec<String> = collect_candidates(&store, &o, now()).unwrap().into_iter().map(|c| c.title).collect();
        assert!(titles.contains(&"Fresh MR".to_string()) && titles.contains(&"Old defect".to_string()));
        assert!(!titles.contains(&"Ancient MR".to_string()));
    }

    #[tokio::test]
    async fn defect_comments_are_read_scored_and_given_to_the_ai() {
        let jira = MockServer::start().await;
        let adf = |t: &str| json!({ "type": "doc", "version": 1, "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": t }] }] });
        Mock::given(method("GET")).and(path("/rest/api/3/issue/ABC-1")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "key": "ABC-1", "fields": { "description": adf("Refund totals are wrong."), "comment": { "comments": [
                { "author": { "displayName": "Grace" }, "body": adf("Reproduced on staging.") },
                { "author": { "displayName": "Ada" }, "body": adf("Root cause is rounding.") }
            ] } }
        }))).mount(&jira).await;
        // ABC-2 is not readable (permissions): it must not break anything.
        Mock::given(method("GET")).and(path("/rest/api/3/issue/ABC-2")).respond_with(ResponseTemplate::new(403)).mount(&jira).await;
        let jira_client = JiraClient::new(&jira.uri(), "me@example.com", "tok").unwrap();

        let ollama = MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/chat")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "message": { "role": "assistant", "content": json!({
                "title": "Fix refund totals", "description": "Totals are wrong after a refund.", "acceptance_criteria": ["Totals match"]
            }).to_string() }
        }))).mount(&ollama).await;
        let ai = OllamaClient::new(&ollama.uri(), "m").unwrap();

        let store = TaskStore::in_memory();
        store.upsert_jira_issues(&[defect("ABC-1", "Totals wrong after refund", "Done", true), defect("ABC-2", "Unreadable", "To Do", false)]).unwrap();

        let report = suggest_impl(&store, Some(&jira_client), Some(&ai), &opts(2), now()).await.unwrap();
        assert_eq!(report.available, 2);
        let first = &report.suggestions[0];
        assert_eq!(first.jira_key.as_deref(), Some("ABC-1"), "the discussed, resolved defect ranks first");
        assert_eq!((first.title.as_str(), first.source.as_str()), ("Fix refund totals", "ai"));
        assert!(first.evidence.contains(&"2 Jira comments".to_string()) && first.evidence.contains(&"Resolved".to_string()));
        assert!(first.description.ends_with("Related Jira issue: ABC-1"));
        assert!(report.notes.iter().any(|n| n.contains("Couldn't read some Jira comments")));

        // What the model saw: the description and the comments, with their authors.
        let bodies: Vec<String> = ollama.received_requests().await.unwrap().iter().map(|r| String::from_utf8_lossy(&r.body).to_string()).collect();
        assert!(bodies[0].contains("Refund totals are wrong") && bodies[0].contains("Grace: Reproduced on staging") && bodies[0].contains("Root cause is rounding"));
    }

    #[tokio::test]
    async fn without_ai_the_template_drafts_and_a_missing_jira_is_mentioned() {
        let store = TaskStore::in_memory();
        import(&store, &[change("u1", "build global search", "merged", "- add index", now())]);
        store.upsert_jira_issues(&[defect("ABC-1", "Crash on start", "To Do", false)]).unwrap();

        let report = suggest_impl(&store, None, None, &opts(5), now()).await.unwrap();
        assert_eq!(report.suggestions.len(), 2);
        assert!(report.suggestions.iter().all(|s| s.source == "template"));
        assert!(report.suggestions.iter().any(|s| s.title == "Build global search"));
        assert!(report.notes.iter().any(|n| n.contains("Jira isn't connected")));
        assert!(report.stopped.is_none());
    }

    #[tokio::test]
    async fn an_unavailable_ai_falls_back_to_the_template_and_says_so() {
        let ai = OllamaClient::new("http://127.0.0.1:1", "m").unwrap(); // nothing listens here
        let store = TaskStore::in_memory();
        for (i, title) in ["alpha work", "beta work"].iter().enumerate() {
            let t = store.add(title).unwrap();
            track(&store, t.id, 600 * (i as i64 + 1));
        }
        let report = suggest_impl(&store, None, Some(&ai), &opts(2), now()).await.unwrap();
        assert_eq!(report.suggestions.len(), 2);
        assert!(report.suggestions.iter().all(|s| s.source == "template"));
        assert!(report.stopped.unwrap().contains("Can't reach the AI server"));
    }

    #[tokio::test]
    async fn silly_requests_are_refused() {
        let store = TaskStore::in_memory();
        assert!(suggest_impl(&store, None, None, &opts(0), now()).await.is_err());
        assert!(suggest_impl(&store, None, None, &opts(MAX_COUNT + 1), now()).await.is_err());
        let none = SuggestOptions { merge_requests: false, defects: false, other_tasks: false, ..opts(3) };
        assert!(suggest_impl(&store, None, None, &none, now()).await.is_err());
    }

    #[test]
    fn adopting_saves_drafts_on_tasks_and_a_defect_gets_a_new_task_once() {
        let store = TaskStore::in_memory();
        import(&store, &[change("u1", "Build global search", "merged", "- x", now())]);
        store.upsert_jira_issues(&[defect("ABC-1", "Totals wrong", "Done", true)]).unwrap();
        let mr_task = task_id(&store, "Build global search");
        let defect_task = task_id(&store, "Totals wrong");

        let items = [
            AdoptItem { task_id: mr_task, kind: "merge_request".into(), title: "Add global search".into(), description: "Users can search.".into(), source: "ai".into() },
            AdoptItem { task_id: defect_task, kind: "defect".into(), title: "Fix refund totals".into(),
                description: "Totals are wrong.\n\nRelated Jira issue: ABC-1".into(), source: "template".into() },
        ];
        let report = adopt_impl(&store, &items);
        assert!(report.failed.is_empty(), "{:?}", report.failed);
        assert_eq!(report.task_ids[0], mr_task);

        let mr = store.find(mr_task).unwrap();
        assert_eq!((mr.story_title.as_deref(), mr.story_source.as_deref()), (Some("Add global search"), Some("ai")));
        // The defect itself is untouched; its story lives on a new task.
        assert!(store.find(defect_task).unwrap().story_title.is_none());
        let created = store.find(report.task_ids[1]).unwrap();
        assert_eq!((created.title.as_str(), created.story_source.as_deref()), ("Fix refund totals", Some("template")));
        assert!(created.jira_key.is_none());

        // Both are now drafted, so asking again does not suggest the same work twice.
        assert!(collect_candidates(&store, &opts(5), now()).unwrap().is_empty());
    }

    #[test]
    fn adopting_a_task_that_cannot_hold_a_draft_fails_that_item_only() {
        let store = TaskStore::in_memory();
        let ok = store.add("fine").unwrap();
        let linked = store.add("linked").unwrap();
        store.link_jira_key(linked.id, "ABC-9").unwrap();
        let item = |id, title: &str| AdoptItem { task_id: id, kind: "task".into(), title: title.into(), description: "d".into(), source: "ai".into() };
        let report = adopt_impl(&store, &[item(linked.id, "x"), item(ok.id, "y")]);
        assert_eq!(report.task_ids, vec![ok.id]);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].task_id, linked.id);
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration(0), "0 min");
        assert_eq!(duration(25 * 60), "25 min");
        assert_eq!(duration(3_600), "1 h");
        assert_eq!(duration(3_600 + 20 * 60), "1 h 20 min");
    }
}
