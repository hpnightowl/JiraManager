//! Start/stop timer. Every method takes the time as a parameter (`now`, `at`) instead of
//! reading the clock itself. That keeps the logic deterministic, so tests can say
//! "start at 1000, stop at 1090" without sleeping. Only the Tauri commands at the bottom
//! read the real clock.

use crate::error::AppError;
use crate::tasks::TaskStore;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use tauri::State;

/// If the heartbeat is older than this when the app starts, the app was closed or crashed
/// while a timer ran, and we ask the user what to do instead of silently counting the gap.
const STALE_AFTER_SECS: i64 = 60;

/// A timer that was still "running" when the app was last open.
#[derive(Debug, Serialize, PartialEq)]
pub struct StaleTimer {
    pub task_id: i64,
    pub title: String,
    pub started_at: i64,
    /// Last moment the app confirmed it was alive: the sensible time to stop the timer at.
    pub last_seen: i64,
}

// A second `impl` block for the same type, in another file: Rust allows this, and it keeps
// each feature in its own module.
impl TaskStore {
    /// Start timing `task_id`. Any timer already running on another task is stopped at `now`
    /// (all in one transaction, so there is never a moment with two timers or none).
    pub fn start_timer(&self, task_id: i64, now: i64) -> Result<(), AppError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;

        tx.query_row("SELECT 1 FROM tasks WHERE id = ?1", params![task_id], |_| Ok(()))
            .optional()?
            .ok_or(AppError::NotFound(task_id))?;

        let running: Option<i64> = tx
            .query_row(
                "SELECT task_id FROM time_entries WHERE ended_at IS NULL",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if running == Some(task_id) {
            return Ok(()); // already running: nothing to do (the transaction rolls back unused)
        }

        tx.execute(
            "UPDATE time_entries SET ended_at = ?1 WHERE ended_at IS NULL",
            params![now],
        )?;
        tx.execute(
            "INSERT INTO time_entries (task_id, started_at, heartbeat_at) VALUES (?1, ?2, ?2)",
            params![task_id, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Stop the running timer (if any) at time `at`, clamped into [started_at, now] so a bad
    /// value can never give negative or future time. No timer running is not an error.
    pub fn stop_timer(&self, at: i64, now: i64) -> Result<(), AppError> {
        let conn = self.conn()?;
        // With two arguments, SQLite's MIN/MAX are plain "smaller/larger of these" functions.
        conn.execute(
            "UPDATE time_entries SET ended_at = MIN(MAX(?1, started_at), ?2) WHERE ended_at IS NULL",
            params![at, now],
        )?;
        Ok(())
    }

    /// "I'm still alive": remember this moment on the running entry.
    pub fn heartbeat(&self, now: i64) -> Result<(), AppError> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE time_entries SET heartbeat_at = ?1 WHERE ended_at IS NULL",
            params![now],
        )?;
        Ok(())
    }

    /// Seconds worked inside [from, to): used for the "today" and "this month" totals.
    /// An entry that straddles the edge only counts the part inside; a running timer counts
    /// up to `now`. (SQLite's two-argument MIN/MAX are "smaller/larger of these".)
    pub fn tracked_between(&self, from: i64, to: i64, now: i64) -> Result<i64, AppError> {
        let conn = self.conn()?;
        Ok(conn.query_row(
            "SELECT COALESCE(SUM(MAX(0, MIN(COALESCE(ended_at, ?3), ?2) - MAX(started_at, ?1))), 0)
             FROM time_entries",
            params![from, to, now],
            |row| row.get(0),
        )?)
    }

    /// Is there a running timer whose heartbeat has gone quiet? (Call once at app start.)
    pub fn stale_timer(&self, now: i64) -> Result<Option<StaleTimer>, AppError> {
        let conn = self.conn()?;
        let stale = conn
            .query_row(
                "SELECT t.id, t.title, e.started_at, e.heartbeat_at
                 FROM time_entries e JOIN tasks t ON t.id = e.task_id
                 WHERE e.ended_at IS NULL AND e.heartbeat_at < ?1",
                params![now - STALE_AFTER_SECS],
                |row| {
                    Ok(StaleTimer {
                        task_id: row.get(0)?,
                        title: row.get(1)?,
                        started_at: row.get(2)?,
                        last_seen: row.get(3)?,
                    })
                },
            )
            .optional()?;
        Ok(stale)
    }
}

// --- Tauri commands: the only place that reads the real clock ----------------------------

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[tauri::command]
pub fn start_timer(task_id: i64, store: State<'_, TaskStore>) -> Result<(), AppError> {
    store.start_timer(task_id, now())
}

/// `at: None` means "stop right now"; the UI passes `Some(last_seen)` to stop a stale timer.
#[tauri::command]
pub fn stop_timer(at: Option<i64>, store: State<'_, TaskStore>) -> Result<(), AppError> {
    let now = now();
    store.stop_timer(at.unwrap_or(now), now)
}

#[tauri::command]
pub fn heartbeat(store: State<'_, TaskStore>) -> Result<(), AppError> {
    store.heartbeat(now())
}

/// The UI passes the start and end of "today" / "this month" in the user's own time zone.
#[tauri::command]
pub fn tracked_between(from: i64, to: i64, store: State<'_, TaskStore>) -> Result<i64, AppError> {
    store.tracked_between(from, to, now())
}

#[tauri::command]
pub fn stale_timer(store: State<'_, TaskStore>) -> Result<Option<StaleTimer>, AppError> {
    store.stale_timer(now())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only_task(store: &TaskStore) -> crate::tasks::Task {
        store.list().unwrap().remove(0)
    }

    #[test]
    fn start_then_stop_adds_up_seconds() {
        let store = TaskStore::in_memory();
        let t = store.add("a").unwrap();
        store.start_timer(t.id, 1000).unwrap();
        assert_eq!(only_task(&store).running_since, Some(1000));
        store.stop_timer(1090, 1090).unwrap();
        let t = only_task(&store);
        assert_eq!((t.tracked_seconds, t.running_since), (90, None));
        // A second session accumulates on top of the first.
        store.start_timer(t.id, 2000).unwrap();
        store.stop_timer(2010, 2010).unwrap();
        assert_eq!(only_task(&store).tracked_seconds, 100);
    }

    #[test]
    fn starting_another_task_stops_the_first() {
        let store = TaskStore::in_memory();
        let a = store.add("a").unwrap();
        let b = store.add("b").unwrap();
        store.start_timer(a.id, 100).unwrap();
        store.start_timer(b.id, 160).unwrap();
        let tasks = store.list().unwrap();
        assert_eq!((tasks[0].tracked_seconds, tasks[0].running_since), (60, None));
        assert_eq!(tasks[1].running_since, Some(160));
    }

    #[test]
    fn starting_the_running_task_again_changes_nothing() {
        let store = TaskStore::in_memory();
        let a = store.add("a").unwrap();
        store.start_timer(a.id, 100).unwrap();
        store.start_timer(a.id, 500).unwrap();
        assert_eq!(only_task(&store).running_since, Some(100));
    }

    #[test]
    fn stop_is_clamped_and_harmless_when_nothing_runs() {
        let store = TaskStore::in_memory();
        let a = store.add("a").unwrap();
        store.stop_timer(5, 5).unwrap(); // nothing running: fine
        store.start_timer(a.id, 1000).unwrap();
        store.stop_timer(9999, 1100).unwrap(); // "future" end is clamped to now
        assert_eq!(only_task(&store).tracked_seconds, 100);
        store.start_timer(a.id, 2000).unwrap();
        store.stop_timer(10, 2050).unwrap(); // end before start is clamped to start
        assert_eq!(only_task(&store).tracked_seconds, 100);
    }

    #[test]
    fn tracked_between_counts_only_the_part_inside_the_window() {
        let store = TaskStore::in_memory();
        let a = store.add("a").unwrap();
        store.start_timer(a.id, 100).unwrap();
        store.stop_timer(200, 200).unwrap(); // 100..200
        store.start_timer(a.id, 300).unwrap(); // running from 300
        assert_eq!(store.tracked_between(0, 1000, 400).unwrap(), 100 + 100); // second entry up to now=400
        assert_eq!(store.tracked_between(150, 350, 400).unwrap(), 50 + 50); // clipped on both edges
        assert_eq!(store.tracked_between(500, 600, 700).unwrap(), 100); // the running entry, clipped to 500..600
        assert_eq!(store.tracked_between(210, 290, 700).unwrap(), 0); // a gap with no work
    }

    #[test]
    fn unknown_task_cannot_be_started() {
        let store = TaskStore::in_memory();
        assert!(matches!(store.start_timer(42, 0), Err(AppError::NotFound(42))));
    }

    #[test]
    fn silent_timer_is_reported_as_stale_until_a_heartbeat() {
        let store = TaskStore::in_memory();
        let a = store.add("a").unwrap();
        store.start_timer(a.id, 1000).unwrap();
        assert_eq!(store.stale_timer(1030).unwrap(), None); // quiet for only 30 s
        let stale = store.stale_timer(2000).unwrap().unwrap();
        assert_eq!((stale.task_id, stale.last_seen), (a.id, 1000));
        store.heartbeat(1990).unwrap();
        assert_eq!(store.stale_timer(2000).unwrap(), None);
    }

    #[test]
    fn deleting_a_task_deletes_its_time_entries() {
        let store = TaskStore::in_memory();
        let a = store.add("a").unwrap();
        store.start_timer(a.id, 10).unwrap();
        store.remove(a.id).unwrap();
        let count: i64 = store
            .conn()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM time_entries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn database_itself_refuses_two_running_timers() {
        let store = TaskStore::in_memory();
        let a = store.add("a").unwrap();
        let b = store.add("b").unwrap();
        let conn = store.conn().unwrap();
        let insert = "INSERT INTO time_entries (task_id, started_at, heartbeat_at) VALUES (?1, 1, 1)";
        conn.execute(insert, params![a.id]).unwrap();
        assert!(conn.execute(insert, params![b.id]).is_err());
    }
}
