use crate::error::AppError;
use rusqlite::Connection;

/// Schema history. Never edit a shipped entry; append a new one.
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
    -- only one row can be running at a time
    CREATE UNIQUE INDEX one_running_timer ON time_entries((1)) WHERE ended_at IS NULL;",
    // v3: settings (no tokens here, see secrets.rs) and the link from a task to its Jira issue.
    "CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    ALTER TABLE tasks ADD COLUMN jira_key    TEXT;
    ALTER TABLE tasks ADD COLUMN jira_status TEXT;
    CREATE UNIQUE INDEX tasks_jira_key ON tasks(jira_key) WHERE jira_key IS NOT NULL;",
    // v4: which time entries are already logged in Jira.
    "ALTER TABLE time_entries ADD COLUMN jira_worklog_id TEXT;
    ALTER TABLE time_entries ADD COLUMN pushed_at INTEGER;",
    // v5: Jira-aware tasks: creation date, type, parent, a parent chosen here but not pushed yet,
    // time already logged in Jira, and context_only for parents that are not mine (headings).
    "ALTER TABLE tasks ADD COLUMN created_at          INTEGER;
    ALTER TABLE tasks ADD COLUMN issue_type          TEXT;
    ALTER TABLE tasks ADD COLUMN parent_key          TEXT;
    ALTER TABLE tasks ADD COLUMN pending_parent_key  TEXT;
    ALTER TABLE tasks ADD COLUMN jira_logged_seconds INTEGER;
    ALTER TABLE tasks ADD COLUMN context_only        INTEGER NOT NULL DEFAULT 0;
    UPDATE tasks SET created_at = COALESCE(
        (SELECT MIN(started_at) FROM time_entries e WHERE e.task_id = tasks.id),
        CAST(strftime('%s', 'now') AS INTEGER));",
    // v6: story drafts on tasks, and imported merge requests. Deleting a task sets task_id to NULL
    // so the merge request doesn't look new on the next import.
    "ALTER TABLE tasks ADD COLUMN story_title       TEXT;
    ALTER TABLE tasks ADD COLUMN story_description TEXT;
    ALTER TABLE tasks ADD COLUMN story_source      TEXT;
    CREATE TABLE merge_requests (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        gitlab_id     INTEGER NOT NULL UNIQUE,
        task_id       INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
        reference     TEXT    NOT NULL,
        title         TEXT    NOT NULL,
        description   TEXT    NOT NULL DEFAULT '',
        state         TEXT    NOT NULL,
        web_url       TEXT    NOT NULL,
        source_branch TEXT    NOT NULL DEFAULT '',
        created_at    INTEGER NOT NULL,
        merged_at     INTEGER,
        role          TEXT    NOT NULL,
        jira_key      TEXT,
        commit_notes  TEXT    NOT NULL DEFAULT ''
    );
    CREATE INDEX merge_requests_task ON merge_requests(task_id);",
    // v7: several git sources. The old single GitLab setup becomes the first connection (same token
    // name). Merge requests are keyed by web address (`local://...` for branches) instead of
    // GitLab's id, which is only unique per server; SQLite needs the table rebuilt for that.
    "CREATE TABLE git_connections (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        name        TEXT NOT NULL,
        kind        TEXT NOT NULL,
        base_url    TEXT NOT NULL,
        secret_name TEXT UNIQUE,
        author      TEXT
    );
    INSERT INTO git_connections (name, kind, base_url, secret_name)
        SELECT 'GitLab', 'gitlab', value, 'gitlab-api-token' FROM settings WHERE key = 'gitlab.base_url';
    INSERT INTO git_connections (name, kind, base_url, secret_name)
        SELECT 'GitLab', 'gitlab', 'https://gitlab.com', 'gitlab-api-token'
        WHERE NOT EXISTS (SELECT 1 FROM git_connections) AND EXISTS (SELECT 1 FROM merge_requests);

    CREATE TABLE merge_requests_v7 (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        connection_id INTEGER REFERENCES git_connections(id) ON DELETE SET NULL,
        source        TEXT    NOT NULL DEFAULT 'gitlab',
        task_id       INTEGER REFERENCES tasks(id) ON DELETE SET NULL,
        reference     TEXT    NOT NULL,
        title         TEXT    NOT NULL,
        description   TEXT    NOT NULL DEFAULT '',
        state         TEXT    NOT NULL,
        web_url       TEXT    NOT NULL UNIQUE,
        source_branch TEXT    NOT NULL DEFAULT '',
        created_at    INTEGER NOT NULL,
        merged_at     INTEGER,
        role          TEXT    NOT NULL,
        jira_key      TEXT,
        commit_notes  TEXT    NOT NULL DEFAULT ''
    );
    INSERT INTO merge_requests_v7 (id, connection_id, source, task_id, reference, title, description, state,
            web_url, source_branch, created_at, merged_at, role, jira_key, commit_notes)
        SELECT id, (SELECT MIN(id) FROM git_connections), 'gitlab', task_id, reference, title, description, state,
            web_url, source_branch, created_at, merged_at, role, jira_key, commit_notes
        FROM merge_requests;
    DROP TABLE merge_requests;
    ALTER TABLE merge_requests_v7 RENAME TO merge_requests;
    CREATE INDEX merge_requests_task ON merge_requests(task_id);",
    // v8: issues removed from the list, so a Refresh doesn't bring them back.
    "CREATE TABLE hidden_issues (
        jira_key  TEXT PRIMARY KEY,
        title     TEXT NOT NULL,
        hidden_at INTEGER NOT NULL
    );",
    // v9: issues created by this app, and a copy of what Jira says I reported.
    "CREATE TABLE created_issues (
        jira_key   TEXT PRIMARY KEY,
        title      TEXT NOT NULL,
        issue_type TEXT,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE reported_issues (
        jira_key   TEXT PRIMARY KEY,
        title      TEXT NOT NULL,
        issue_type TEXT,
        status     TEXT,
        created_at INTEGER NOT NULL
    );",
    // v10: a copy of my Done issues from Jira, replaced on each read.
    "CREATE TABLE done_issues (
        jira_key   TEXT PRIMARY KEY,
        title      TEXT NOT NULL,
        issue_type TEXT,
        status     TEXT,
        done_at    INTEGER NOT NULL
    );",
];

/// Applies any migrations not yet run; the count is kept in SQLite's `user_version`.
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
    fn old_gitlab_setup_becomes_first_connection() {
        let mut conn = Connection::open_in_memory().unwrap();
        // A database as the previous version left it (migrations 1..=6 applied).
        for (i, sql) in MIGRATIONS[..6].iter().enumerate() {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", i as i64 + 1).unwrap();
        }
        conn.execute("INSERT INTO settings (key, value) VALUES ('gitlab.base_url', 'https://git.example.com')", []).unwrap();
        conn.execute("INSERT INTO tasks (title) VALUES ('from an MR')", []).unwrap();
        conn.execute(
            "INSERT INTO merge_requests (gitlab_id, task_id, reference, title, state, web_url, created_at, role)
             VALUES (42, 1, 'g/p!7', 'Speed up search', 'merged', 'https://git.example.com/g/p/-/merge_requests/7', 100, 'author')",
            [],
        )
        .unwrap();

        migrate(&mut conn).unwrap();

        let (name, kind, url, secret): (String, String, String, String) = conn
            .query_row("SELECT name, kind, base_url, secret_name FROM git_connections", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap();
        assert_eq!((name.as_str(), kind.as_str(), url.as_str()), ("GitLab", "gitlab", "https://git.example.com"));
        assert_eq!(secret, "gitlab-api-token"); // the token is found under its old name: nothing to re-enter

        let (title, source, task, connection): (String, String, i64, i64) = conn
            .query_row("SELECT title, source, task_id, connection_id FROM merge_requests", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap();
        assert_eq!((title.as_str(), source.as_str(), task, connection), ("Speed up search", "gitlab", 1, 1));

        // The same web address can't be stored twice.
        let dup = conn.execute(
            "INSERT INTO merge_requests (task_id, reference, title, state, web_url, created_at, role)
             VALUES (NULL, 'x', 'x', 'opened', 'https://git.example.com/g/p/-/merge_requests/7', 1, 'author')",
            [],
        );
        assert!(dup.is_err());
    }

    #[test]
    fn a_fresh_database_has_no_connections() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM git_connections", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn upgrading_a_v1_database_keeps_its_data() {
        let mut conn = Connection::open_in_memory().unwrap();
        // A database from an early version (only migration 1 applied).
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
