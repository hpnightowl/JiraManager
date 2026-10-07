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
}

/// Task storage backed by a SQLite file.
/// A `rusqlite::Connection` can't be used from two threads at once,
/// so we keep it behind a `Mutex` (same idea as milestone 2).
pub struct TaskStore {
    conn: Mutex<Connection>,
}

/// Tasks plus their time totals in one query: a LEFT JOIN keeps tasks that have no
/// time entries yet, and GROUP BY folds each task's entries into one row.
const TASK_SELECT: &str = "SELECT t.id, t.title, t.done,
        COALESCE(SUM(CASE WHEN e.ended_at IS NOT NULL THEN e.ended_at - e.started_at END), 0),
        MAX(CASE WHEN e.ended_at IS NULL THEN e.started_at END)
    FROM tasks t LEFT JOIN time_entries e ON e.task_id = t.id";

fn row_to_task(row: &Row<'_>) -> rusqlite::Result<Task> {
    Ok(Task {
        id: row.get(0)?,
        title: row.get(1)?,
        done: row.get(2)?, // SQLite stores 0/1, rusqlite converts to bool
        tracked_seconds: row.get(3)?,
        running_since: row.get(4)?, // NULL becomes None
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
        conn.execute("INSERT INTO tasks (title) VALUES (?1)", params![title])?;
        get(&conn, conn.last_insert_rowid())
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

    pub fn remove(&self, id: i64) -> Result<(), AppError> {
        let conn = self.conn()?;
        // Its time entries go too: `ON DELETE CASCADE` in the schema.
        let changed = conn.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
        if changed == 0 {
            return Err(AppError::NotFound(id));
        }
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
}
