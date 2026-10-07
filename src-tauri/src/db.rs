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
}
