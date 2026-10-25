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
    // v3: Jira. Plain key/value settings (the API token is NOT here, see secrets.rs),
    // and a link from a local task to the Jira issue it mirrors.
    "CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    ALTER TABLE tasks ADD COLUMN jira_key    TEXT;
    ALTER TABLE tasks ADD COLUMN jira_status TEXT;
    CREATE UNIQUE INDEX tasks_jira_key ON tasks(jira_key) WHERE jira_key IS NOT NULL;",
    // v4: remember which time entries are already logged in Jira, so pushing twice
    // never logs the same work twice.
    "ALTER TABLE time_entries ADD COLUMN jira_worklog_id TEXT;
    ALTER TABLE time_entries ADD COLUMN pushed_at INTEGER;",
    // v5: a Jira-aware task list.
    //   created_at          when the story was created (Jira's date for fetched issues): month view
    //   issue_type          Epic / Story / Task / Bug ...
    //   parent_key          the parent Jira says this issue has
    //   pending_parent_key  a parent chosen here that is NOT in Jira yet (sent on Push)
    //   jira_logged_seconds time Jira already has logged on the issue
    //   context_only        1 = a parent (e.g. an epic) that is not mine; shown as a heading only
    "ALTER TABLE tasks ADD COLUMN created_at          INTEGER;
    ALTER TABLE tasks ADD COLUMN issue_type          TEXT;
    ALTER TABLE tasks ADD COLUMN parent_key          TEXT;
    ALTER TABLE tasks ADD COLUMN pending_parent_key  TEXT;
    ALTER TABLE tasks ADD COLUMN jira_logged_seconds INTEGER;
    ALTER TABLE tasks ADD COLUMN context_only        INTEGER NOT NULL DEFAULT 0;
    UPDATE tasks SET created_at = COALESCE(
        (SELECT MIN(started_at) FROM time_entries e WHERE e.task_id = tasks.id),
        CAST(strftime('%s', 'now') AS INTEGER));",
    // v6: GitLab merge requests and saved story drafts.
    //   story_*        a drafted story (title + description incl. acceptance criteria) kept on the
    //                  task, so it survives closing the dialog and is what Push sends
    //   merge_requests one row per MR we imported. `task_id` is the task that represents it
    //                  (SET NULL, not CASCADE: deleting a task must not make the MR look "new"
    //                  again on the next import, or it would come back from the dead)
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
    // v7: many git sources. A "connection" is either a GitLab server (its own address and its own
    // token, kept in the keychain under `secret_name`) or a local repository folder. The single
    // GitLab setup from v6 becomes the first connection, still using its old token name, so
    // nothing has to be re-entered.
    //
    // Merge requests were identified by GitLab's numeric id, which is only unique per SERVER.
    // They are now identified by their web address, which is unique everywhere (for local
    // branches it is a `local://...` address). SQLite can't change a column's constraints, so the
    // table is rebuilt and the rows copied over.
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
    // v8: issues you removed from the list on purpose. Without this, "remove" only lasted until
    // the next Refresh, which would bring the same issue straight back.
    "CREATE TABLE hidden_issues (
        jira_key  TEXT PRIMARY KEY,
        title     TEXT NOT NULL,
        hidden_at INTEGER NOT NULL
    );",
    // v9: "how many stories have I created?", two honest answers.
    //   created_issues   what THIS APP created in Jira (an exact ledger, counted from now on: earlier
    //                    pushes weren't recorded, so they can't be added afterwards)
    //   reported_issues  a copy of what Jira says you reported (all history, refreshed on request)
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
    fn the_single_gitlab_setup_becomes_the_first_connection_and_merge_requests_survive() {
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
