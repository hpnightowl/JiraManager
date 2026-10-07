use crate::error::AppError;
use rusqlite::Connection;

/// Schema history. NEVER edit an entry that has shipped: append a new one.
/// The position in this list is the schema version (1 = first entry).
const MIGRATIONS: &[&str] = &[
    // v1: tasks
    "CREATE TABLE tasks (
        id    INTEGER PRIMARY KEY AUTOINCREMENT,
        title TEXT    NOT NULL,
        done  INTEGER NOT NULL DEFAULT 0
    );",
    // v2: time tracking. Times are Unix seconds (UTC). `ended_at` NULL = still running.
    // `heartbeat_at` is refreshed while the app runs, so after a crash we know when it died.
    "CREATE TABLE time_entries (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        task_id      INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
        started_at   INTEGER NOT NULL,
        ended_at     INTEGER,
        heartbeat_at INTEGER NOT NULL
    );
    CREATE INDEX time_entries_task ON time_entries(task_id);
    -- At most ONE row may have ended_at IS NULL: every running row indexes the constant 1,
    -- and a unique index rejects a second '1'. The database enforces the rule, not our code.
    CREATE UNIQUE INDEX one_running_timer ON time_entries((1)) WHERE ended_at IS NULL;",
];

/// Bring the database up to the latest schema.
/// SQLite keeps a free integer `user_version` in the file header; we use it
/// to remember how many migrations have already run.
pub fn migrate(conn: &mut Connection) -> Result<(), AppError> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    for (index, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        // One transaction per migration: either it fully applies, or not at all.
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", index as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_sets_version_and_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        migrate(&mut conn).unwrap(); // running again must be a no-op, not an error
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
    }

    #[test]
    fn upgrading_a_v1_database_keeps_its_data() {
        let mut conn = Connection::open_in_memory().unwrap();
        // Simulate a database created by the milestone-3 app (only migration 1 applied).
        conn.execute_batch(MIGRATIONS[0]).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute("INSERT INTO tasks (title) VALUES ('old task')", [])
            .unwrap();

        migrate(&mut conn).unwrap();

        let title: String = conn
            .query_row("SELECT title FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "old task");
        let entries: i64 = conn // the v2 table now exists
            .query_row("SELECT COUNT(*) FROM time_entries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(entries, 0);
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, MIGRATIONS.len() as i64);
    }
}
