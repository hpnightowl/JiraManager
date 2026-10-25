//! "How many stories have I created?" Two answers, because each has a gap on its own:
//!   * **Created with this app**: an exact ledger of what ProjectJira itself created in Jira.
//!     It only knows about pushes made since the ledger existed; earlier ones can't be recovered.
//!   * **Reported in Jira**: a copy of what Jira says you created (reporter = you), history included,
//!     but it also counts things you made outside this app. Refreshed when you ask.

use crate::error::AppError;
use crate::jira::{load_client, JiraIssue};
use crate::secrets::SecretStore;
use crate::tasks::TaskStore;
use rusqlite::params;
use serde::Serialize;
use tauri::State;

const KEY_REPORTED_AT: &str = "stats.reported_at";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoryRecord {
    pub jira_key: String,
    pub title: String,
    pub issue_type: Option<String>,
    pub status: Option<String>,
    /// Unix seconds.
    pub created_at: i64,
}

#[derive(Debug, Serialize)]
pub struct StoryStats {
    pub created_with_app: Vec<StoryRecord>,
    pub reported: Vec<StoryRecord>,
    /// When `reported` was last refreshed from Jira (None = never).
    pub reported_refreshed_at: Option<i64>,
}

impl TaskStore {
    pub fn created_with_app(&self) -> Result<Vec<StoryRecord>, AppError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT jira_key, title, issue_type, created_at FROM created_issues ORDER BY created_at DESC, jira_key")?;
        let list = stmt
            .query_map([], |r| {
                Ok(StoryRecord { jira_key: r.get(0)?, title: r.get(1)?, issue_type: r.get(2)?, status: None, created_at: r.get(3)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(list)
    }

    pub fn reported_issues(&self) -> Result<Vec<StoryRecord>, AppError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT jira_key, title, issue_type, status, created_at FROM reported_issues ORDER BY created_at DESC, jira_key")?;
        let list = stmt
            .query_map([], |r| {
                Ok(StoryRecord { jira_key: r.get(0)?, title: r.get(1)?, issue_type: r.get(2)?, status: r.get(3)?, created_at: r.get(4)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(list)
    }

    /// Replace the copy of "what Jira says I reported" (all or nothing), and remember when.
    pub fn replace_reported(&self, issues: &[JiraIssue], now: i64) -> Result<usize, AppError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM reported_issues", [])?;
        for issue in issues {
            tx.execute(
                "INSERT OR REPLACE INTO reported_issues (jira_key, title, issue_type, status, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![issue.key, issue.summary, issue.issue_type, issue.status, issue.created_at.unwrap_or(0)],
            )?;
        }
        tx.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![KEY_REPORTED_AT, now.to_string()],
        )?;
        tx.commit()?;
        Ok(issues.len())
    }

    pub fn story_stats(&self) -> Result<StoryStats, AppError> {
        Ok(StoryStats {
            created_with_app: self.created_with_app()?,
            reported: self.reported_issues()?,
            reported_refreshed_at: self.get_setting(KEY_REPORTED_AT)?.and_then(|s| s.parse().ok()),
        })
    }
}

#[tauri::command]
pub fn get_story_stats(store: State<'_, TaskStore>) -> Result<StoryStats, AppError> {
    store.story_stats()
}

/// Ask Jira what I reported (read-only) and keep a copy.
#[tauri::command]
pub async fn refresh_reported_stories(store: State<'_, TaskStore>, secrets: State<'_, SecretStore>) -> Result<StoryStats, AppError> {
    let client = load_client(&store, &secrets)?;
    let issues = client.my_reported().await?;
    store.replace_reported(&issues, chrono::Utc::now().timestamp())?;
    store.story_stats()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_app_creates_is_recorded_and_survives_removing_the_task() {
        let store = TaskStore::in_memory();
        let t = store.add("Add Apple Pay").unwrap();
        store.record_created(t.id, "ABC-7", Some("Story"), None).unwrap();
        store.remove(t.id).unwrap(); // hiding/deleting the task must not change the count

        let ledger = store.created_with_app().unwrap();
        assert_eq!(ledger.len(), 1);
        assert_eq!((ledger[0].jira_key.as_str(), ledger[0].title.as_str(), ledger[0].issue_type.as_deref()), ("ABC-7", "Add Apple Pay", Some("Story")));
        assert!(ledger[0].created_at > 1_700_000_000);
    }

    #[test]
    fn a_task_that_only_mirrors_a_fetched_issue_is_not_counted_as_created_by_the_app() {
        let store = TaskStore::in_memory();
        store.upsert_jira_issues(&[JiraIssue { key: "ABC-1".into(), summary: "Existing".into(), ..Default::default() }]).unwrap();
        assert!(store.created_with_app().unwrap().is_empty());
    }

    fn issue(key: &str, ty: &str, created: i64) -> JiraIssue {
        JiraIssue { key: key.into(), summary: format!("Issue {key}"), issue_type: Some(ty.into()), status: Some("Done".into()), created_at: Some(created), ..Default::default() }
    }

    #[test]
    fn the_copy_of_what_jira_says_i_reported_is_replaced_wholesale_and_dated() {
        let store = TaskStore::in_memory();
        assert_eq!(store.story_stats().unwrap().reported_refreshed_at, None);

        store.replace_reported(&[issue("ABC-1", "Story", 100), issue("ABC-2", "Bug", 200)], 1_000).unwrap();
        let stats = store.story_stats().unwrap();
        assert_eq!(stats.reported.iter().map(|r| r.jira_key.as_str()).collect::<Vec<_>>(), ["ABC-2", "ABC-1"]); // newest first
        assert_eq!(stats.reported_refreshed_at, Some(1_000));

        // A later refresh REPLACES it: an issue that is no longer reported by me disappears.
        store.replace_reported(&[issue("ABC-1", "Story", 100)], 2_000).unwrap();
        let stats = store.story_stats().unwrap();
        assert_eq!(stats.reported.len(), 1);
        assert_eq!(stats.reported_refreshed_at, Some(2_000));
    }
}
