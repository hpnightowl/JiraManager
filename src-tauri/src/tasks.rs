use crate::db;
use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use tauri::State;

/// One task in the list. `Serialize` lets serde turn it into JSON for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub done: bool,
    /// Seconds from FINISHED time entries. The UI adds the live part itself.
    pub tracked_seconds: i64,
    /// Unix time the currently running timer started; `None` = not running.
    pub running_since: Option<i64>,
    /// Set when this task mirrors a Jira issue, e.g. "ABC-123".
    pub jira_key: Option<String>,
    pub jira_status: Option<String>,
    /// Finished time not yet logged to Jira (the UI uses it to offer "Push").
    pub unpushed_seconds: i64,
    /// Unix time the task was created (for Jira issues: when Jira says it was created).
    pub created_at: i64,
    /// "Epic", "Story", "Task", "Bug"... (known for Jira issues).
    pub issue_type: Option<String>,
    /// The parent Jira reports for this issue.
    pub parent_key: Option<String>,
    /// A parent chosen in this app that Jira doesn't know about yet. Sent on Push.
    pub pending_parent_key: Option<String>,
    /// Time Jira already has logged on the issue (as of the last refresh / push).
    pub jira_logged_seconds: Option<i64>,
    /// True for a parent (e.g. an epic) that isn't assigned to me: shown as a heading only.
    pub context_only: bool,
    /// A drafted story kept on the task. Push uses it; "Improve" / "Generate story" fill it.
    pub story_title: Option<String>,
    /// Description including an "Acceptance criteria:" bullet list, as plain text.
    pub story_description: Option<String>,
    /// "ai", "template" or "manual".
    pub story_source: Option<String>,
}

/// "ABC-123": letters/digits/underscore, a dash, digits. Anything else is not a Jira key.
pub(crate) fn is_issue_key(s: &str) -> bool {
    match s.rsplit_once('-') {
        Some((project, number)) => {
            project.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                && project.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !number.is_empty()
                && number.chars().all(|c| c.is_ascii_digit())
        }
        None => false,
    }
}

/// Task storage backed by a SQLite file. The connection sits behind a `Mutex` because it can't be
/// shared between threads.
pub struct TaskStore {
    conn: Mutex<Connection>,
}

/// Tasks plus their time totals in one query: a LEFT JOIN keeps tasks that have no
/// time entries yet, and GROUP BY folds each task's entries into one row.
const TASK_SELECT: &str = "SELECT t.id, t.title, t.done,
        COALESCE(SUM(CASE WHEN e.ended_at IS NOT NULL THEN e.ended_at - e.started_at END), 0),
        MAX(CASE WHEN e.ended_at IS NULL THEN e.started_at END),
        t.jira_key, t.jira_status,
        COALESCE(SUM(CASE WHEN e.ended_at IS NOT NULL AND e.jira_worklog_id IS NULL
                          THEN e.ended_at - e.started_at END), 0),
        COALESCE(t.created_at, 0), t.issue_type, t.parent_key, t.pending_parent_key,
        t.jira_logged_seconds, t.context_only,
        t.story_title, t.story_description, t.story_source
    FROM tasks t LEFT JOIN time_entries e ON e.task_id = t.id";

fn row_to_task(row: &Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        done: row.get(2)?, // SQLite stores 0/1, rusqlite converts to bool
        tracked_seconds: row.get(3)?,
        running_since: row.get(4)?, // NULL becomes None
        jira_key: row.get(5)?,
        jira_status: row.get(6)?,
        unpushed_seconds: row.get(7)?,
        created_at: row.get(8)?,
        issue_type: row.get(9)?,
        parent_key: row.get(10)?,
        pending_parent_key: row.get(11)?,
        jira_logged_seconds: row.get(12)?,
        context_only: row.get(13)?,
        story_title: row.get(14)?,
        story_description: row.get(15)?,
        story_source: row.get(16)?,
    })
}

fn get(conn: &Connection, id: i64) -> Result<Task, AppError> {
    conn.query_row(
        &format!("{TASK_SELECT} WHERE t.id = ?1 GROUP BY t.id"),
        params![id],
        row_to_task,
    )
    .optional()? // "no row" becomes None instead of an error
    .ok_or(AppError::NotFound(id))
}

impl TaskStore {
    /// Open (or create) the database file and bring its schema up to date.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, AppError> {
        Self::from_connection(Connection::open(path)?)
    }

    pub(crate) fn from_connection(mut conn: Connection) -> Result<Self, AppError> {
        // SQLite ignores FOREIGN KEY rules unless you switch them on for each connection.
        conn.pragma_update(None, "foreign_keys", true)?;
        db::migrate(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// `pub(crate)` = visible to other modules of this app (like `timer.rs`), not outside it.
    pub(crate) fn conn(&self) -> Result<MutexGuard<'_, Connection>, AppError> {
        self.conn.lock().map_err(|_| AppError::Poisoned)
    }

    pub fn add(&self, title: &str) -> Result<Task, AppError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::Invalid("Task title can't be empty".into()));
        }
        let conn = self.conn()?;
        // `?1` is a bound parameter: never build SQL by pasting user text into it.
        conn.execute(
            "INSERT INTO tasks (title, created_at) VALUES (?1, CAST(strftime('%s', 'now') AS INTEGER))",
            params![title],
        )?;
        get(&conn, conn.last_insert_rowid())
    }

    /// Rename a task that only exists here. A task linked to Jira keeps Jira's title
    /// (changing it would need a second kind of push, which we don't have yet).
    pub fn rename(&self, id: i64, title: &str) -> Result<Task, AppError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(AppError::Invalid("Task title can't be empty".into()));
        }
        let conn = self.conn()?;
        let task = get(&conn, id)?;
        if task.jira_key.is_some() {
            return Err(AppError::Invalid(
                "This task is linked to Jira. Rename the issue in Jira, then Refresh.".into(),
            ));
        }
        conn.execute("UPDATE tasks SET title = ?2 WHERE id = ?1", params![id, title])?;
        get(&conn, id)
    }

    /// Stage a parent (e.g. an epic) for a task. Nothing goes to Jira until Push.
    /// `None` clears a staged parent.
    pub fn set_pending_parent(&self, id: i64, parent_key: Option<&str>) -> Result<Task, AppError> {
        let conn = self.conn()?;
        let task = get(&conn, id)?;
        if task.context_only {
            return Err(AppError::Invalid("This is a parent heading from Jira, not one of your tasks".into()));
        }
        let parent_key = parent_key.map(str::trim).filter(|k| !k.is_empty());
        if let Some(key) = parent_key {
            if !is_issue_key(key) {
                return Err(AppError::Invalid(format!("\"{key}\" is not a Jira issue key")));
            }
            if task.jira_key.as_deref() == Some(key) {
                return Err(AppError::Invalid("A task can't be its own parent".into()));
            }
            if task.issue_type.as_deref().is_some_and(|t| t.eq_ignore_ascii_case("epic")) {
                return Err(AppError::Invalid("An epic can't have a parent".into()));
            }
        }
        // Choosing the parent Jira already has is not a change.
        let staged = parent_key.filter(|k| task.parent_key.as_deref() != Some(*k));
        conn.execute("UPDATE tasks SET pending_parent_key = ?2 WHERE id = ?1", params![id, staged])?;
        get(&conn, id)
    }

    pub fn find(&self, id: i64) -> Result<Task, AppError> {
        let conn = self.conn()?;
        get(&conn, id)
    }

    pub fn list(&self) -> Result<Vec<Task>, AppError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("{TASK_SELECT} GROUP BY t.id ORDER BY t.id"))?;
        let tasks = stmt
            .query_map([], row_to_task)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(tasks)
    }

    pub fn toggle(&self, id: i64) -> Result<Task, AppError> {
        let conn = self.conn()?;
        let changed = conn.execute("UPDATE tasks SET done = 1 - done WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(AppError::NotFound(id));
        }
        get(&conn, id)
    }

    /// Remove a task from the list. A task that mirrors a Jira issue is also remembered as
    /// *hidden*, so the next Refresh doesn't simply bring it back (restore it from "Hidden").
    /// The Jira issue itself is never touched.
    pub fn remove(&self, id: i64) -> Result<(), AppError> {
        let conn = self.conn()?;
        let jira: Option<(String, String)> = conn
            .query_row(
                "SELECT jira_key, title FROM tasks WHERE id = ?1 AND jira_key IS NOT NULL",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        // Its time entries go too: `ON DELETE CASCADE` in the schema.
        let changed = conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(AppError::NotFound(id));
        }
        if let Some((key, title)) = jira {
            conn.execute(
                "INSERT OR REPLACE INTO hidden_issues (jira_key, title, hidden_at)
                 VALUES (?1, ?2, CAST(strftime('%s', 'now') AS INTEGER))",
                params![key, title],
            )?;
        }
        Ok(())
    }

    /// Small key/value settings (Jira URL, email...). Secrets do not belong here.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, AppError> {
        let conn = self.conn()?;
        Ok(conn
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), AppError> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self::from_connection(Connection::open_in_memory().unwrap()).unwrap()
    }
}

// --- Tauri commands -------------------------------------------------------------------

#[tauri::command]
pub fn add_task(title: String, store: State<'_, TaskStore>) -> Result<Task, AppError> {
    store.add(&title)
}

#[tauri::command]
pub fn list_tasks(store: State<'_, TaskStore>) -> Result<Vec<Task>, AppError> {
    store.list()
}

#[tauri::command]
pub fn toggle_task(id: i64, store: State<'_, TaskStore>) -> Result<Task, AppError> {
    store.toggle(id)
}

#[tauri::command]
pub fn delete_task(id: i64, store: State<'_, TaskStore>) -> Result<(), AppError> {
    store.remove(id)
}

#[tauri::command]
pub fn rename_task(id: i64, title: String, store: State<'_, TaskStore>) -> Result<Task, AppError> {
    store.rename(id, &title)
}

#[tauri::command]
pub fn set_task_parent(id: i64, parent_key: Option<String>, store: State<'_, TaskStore>) -> Result<Task, AppError> {
    store.set_pending_parent(id, parent_key.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_trims_and_assigns_increasing_ids() {
        let store = TaskStore::in_memory();
        let a = store.add("  fix login  ").unwrap();
        let b = store.add("write tests").unwrap();
        assert_eq!(a.title, "fix login");
        assert!(b.id > a.id);
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn add_rejects_blank_title() {
        let store = TaskStore::in_memory();
        assert!(matches!(store.add("   "), Err(AppError::Invalid(_))));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn toggle_flips_done_and_unknown_id_errors() {
        let store = TaskStore::in_memory();
        let t = store.add("x").unwrap();
        assert!(store.toggle(t.id).unwrap().done);
        assert!(!store.toggle(t.id).unwrap().done);
        assert!(matches!(store.toggle(999), Err(AppError::NotFound(999))));
    }

    #[test]
    fn remove_deletes_and_unknown_id_errors() {
        let store = TaskStore::in_memory();
        let t = store.add("x").unwrap();
        store.remove(t.id).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(matches!(store.remove(t.id), Err(AppError::NotFound(_))));
    }

    #[test]
    fn data_survives_reopening_the_file() {
        let path = std::env::temp_dir().join(format!("projectjira-test-{}.db", std::process::id()));
        {
            let store = TaskStore::open(&path).unwrap();
            store.add("persist me").unwrap();
        } // store dropped here -> file closed
        let reopened = TaskStore::open(&path).unwrap();
        let tasks = reopened.list().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "persist me");
        drop(reopened);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn titles_with_sql_characters_are_stored_literally() {
        let store = TaskStore::in_memory();
        let nasty = "x'); DROP TABLE tasks; --";
        let t = store.add(nasty).unwrap();
        assert_eq!(t.title, nasty);
        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn new_tasks_remember_when_they_were_created() {
        let t = TaskStore::in_memory().add("x").unwrap();
        assert!(t.created_at > 1_700_000_000, "created_at = {}", t.created_at);
        assert!(!t.context_only && t.parent_key.is_none() && t.jira_logged_seconds.is_none());
    }

    #[test]
    fn issue_keys_are_recognised() {
        for good in ["ABC-1", "my_proj-42", "A1-100"] {
            assert!(is_issue_key(good), "{good}");
        }
        for bad in ["", "ABC", "ABC-", "-1", "1ABC-2", "ABC-x", "AB C-1", "ABC-1; DROP"] {
            assert!(!is_issue_key(bad), "{bad}");
        }
    }

    #[test]
    fn only_local_tasks_can_be_renamed() {
        let store = TaskStore::in_memory();
        let local = store.add("old").unwrap();
        assert_eq!(store.rename(local.id, "  new  ").unwrap().title, "new");
        assert!(store.rename(local.id, "  ").is_err());
        store.link_jira_key(local.id, "ABC-1").unwrap();
        assert!(store.rename(local.id, "again").is_err());
    }

    #[test]
    fn staging_a_parent_follows_the_rules() {
        let store = TaskStore::in_memory();
        let t = store.add("story").unwrap();
        assert_eq!(store.set_pending_parent(t.id, Some(" EPIC-1 ")).unwrap().pending_parent_key.as_deref(), Some("EPIC-1"));
        assert_eq!(store.set_pending_parent(t.id, None).unwrap().pending_parent_key, None);
        assert!(store.set_pending_parent(t.id, Some("not a key")).is_err());

        // A task can't be its own parent; an epic can't have one.
        store.link_jira_key(t.id, "ABC-5").unwrap();
        assert!(store.set_pending_parent(t.id, Some("ABC-5")).is_err());
        store.conn().unwrap().execute("UPDATE tasks SET issue_type = 'Epic' WHERE id = ?1", params![t.id]).unwrap();
        assert!(store.set_pending_parent(t.id, Some("ABC-1")).is_err());
    }

    #[test]
    fn choosing_the_parent_jira_already_has_is_not_a_change() {
        let store = TaskStore::in_memory();
        let t = store.add("story").unwrap();
        store.conn().unwrap().execute("UPDATE tasks SET parent_key = 'EPIC-1' WHERE id = ?1", params![t.id]).unwrap();
        assert_eq!(store.set_pending_parent(t.id, Some("EPIC-1")).unwrap().pending_parent_key, None);
        assert_eq!(store.set_pending_parent(t.id, Some("EPIC-2")).unwrap().pending_parent_key.as_deref(), Some("EPIC-2"));
    }
}
