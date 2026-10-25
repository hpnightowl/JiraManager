//! Git sources, whatever they are. A *connection* is one place your work lives:
//!   * a GitLab server (gitlab.com or self-hosted) with ITS OWN address and ITS OWN token, or
//!   * a repository folder on this computer (works with any git host, or none).
//! Every source produces the same neutral record, `GitChange` (a merge request, or a feature
//! branch), and everything else in the app (tasks, story drafts, bulk push) only sees that.
//! To support another host later, write one more reader that returns `GitChange`s.

use crate::error::AppError;
use crate::gitlab::{self, GitlabClient};
use crate::jira::normalize_base_url;
use crate::localgit;
use crate::secrets::SecretStore;
use crate::tasks::TaskStore;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use std::collections::HashSet;
use std::path::PathBuf;
use tauri::State;

// ---------------------------------------------------------------------------------------
// Finding the Jira issue a change belongs to
// ---------------------------------------------------------------------------------------

/// Things that look like issue keys but aren't ("UTF-8", "SHA-256"...). Only used when we don't
/// yet know any real Jira project, because with known projects we only accept those.
const NOT_JIRA: &[&str] = &["UTF", "ISO", "SHA", "MD", "RFC", "CVE", "PEP", "TLS", "SSL", "AES", "RSA", "USB", "HTTP", "IPV", "X", "V"];

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// First `ABC-123` found in `text`. With `known` projects, only those are accepted (any case, so
/// branch names like `feature/abc-123-fix` work); without, only upper-case keys not on the
/// `NOT_JIRA` list.
fn scan_for_key(text: &str, known: &HashSet<String>) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let starts_here = chars[i].is_ascii_alphabetic() && (i == 0 || !is_word(chars[i - 1]));
        if starts_here {
            let mut j = i;
            while j < chars.len() && is_word(chars[j]) {
                j += 1;
            }
            // `j` is the end of the word (underscores belong to the project name). A key is the
            // word followed by '-' and digits.
            if j < chars.len() && chars[j] == '-' {
                let mut k = j + 1;
                while k < chars.len() && chars[k].is_ascii_digit() {
                    k += 1;
                }
                let digits_ok = k > j + 1 && (k == chars.len() || !chars[k].is_ascii_alphabetic());
                let project: String = chars[i..j].iter().collect();
                let upper = project.to_ascii_uppercase();
                let length_ok = (2..=10).contains(&upper.len());
                let accepted = if known.is_empty() {
                    project == upper && !NOT_JIRA.contains(&upper.as_str())
                } else {
                    known.contains(&upper)
                };
                if digits_ok && length_ok && accepted {
                    let digits: String = chars[j + 1..k].iter().collect();
                    return Some(format!("{upper}-{digits}"));
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

/// Look through the given texts (title first, then branch name) for a Jira key.
pub fn find_jira_key(texts: &[&str], known_projects: &HashSet<String>) -> Option<String> {
    texts.iter().find_map(|t| scan_for_key(t, known_projects))
}

// ---------------------------------------------------------------------------------------
// The neutral record
// ---------------------------------------------------------------------------------------

/// A merge request or a feature branch, from any source.
#[derive(Debug, Clone, PartialEq)]
pub struct GitChange {
    /// The identity: a web address (GitLab) or `local://<repo path>#<branch>`. Unique everywhere,
    /// so two servers (or a server and a local clone) can never be confused with each other.
    pub url: String,
    /// Short human reference: `group/project!12` or `repo@branch`.
    pub reference: String,
    pub title: String,
    pub description: String,
    /// `opened` / `merged` / `closed` for GitLab, `local` for branches.
    pub state: String,
    pub source_branch: String,
    pub created_at: i64,
    pub merged_at: Option<i64>,
    /// "author" or "assignee"
    pub role: String,
    /// "- subject" lines. `None` = "don't touch what is stored" (commits are only read for new changes).
    pub commit_notes: Option<String>,
}

// ---------------------------------------------------------------------------------------
// Connections (stored)
// ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ConnRow {
    pub id: i64,
    pub name: String,
    /// "gitlab" or "local"
    pub kind: String,
    /// A server address, or a folder path for a local repository.
    pub base_url: String,
    /// Where this connection's token lives in the keychain (GitLab only).
    pub secret_name: Option<String>,
    /// Commit email to look for (local repositories; defaults to the repository's own).
    pub author: Option<String>,
}

fn row_to_conn(r: &rusqlite::Row<'_>) -> rusqlite::Result<ConnRow> {
    Ok(ConnRow { id: r.get(0)?, name: r.get(1)?, kind: r.get(2)?, base_url: r.get(3)?, secret_name: r.get(4)?, author: r.get(5)? })
}

const CONN_COLUMNS: &str = "id, name, kind, base_url, secret_name, author";

impl TaskStore {
    pub fn connections(&self) -> Result<Vec<ConnRow>, AppError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("SELECT {CONN_COLUMNS} FROM git_connections ORDER BY id"))?;
        let rows = stmt.query_map([], row_to_conn)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn connection(&self, id: i64) -> Result<ConnRow, AppError> {
        let conn = self.conn()?;
        conn.query_row(&format!("SELECT {CONN_COLUMNS} FROM git_connections WHERE id = ?1"), params![id], row_to_conn)
            .optional()?
            .ok_or_else(|| AppError::Invalid("That connection no longer exists".into()))
    }

    pub fn add_connection(&self, name: &str, kind: &str, base_url: &str, secret_name: Option<&str>, author: Option<&str>) -> Result<ConnRow, AppError> {
        let id = {
            let conn = self.conn()?;
            conn.execute(
                "INSERT INTO git_connections (name, kind, base_url, secret_name, author) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![name, kind, base_url, secret_name, author],
            )?;
            conn.last_insert_rowid()
        };
        self.connection(id)
    }

    pub fn update_connection(&self, id: i64, name: &str, base_url: &str, author: Option<&str>) -> Result<ConnRow, AppError> {
        {
            let conn = self.conn()?;
            conn.execute(
                "UPDATE git_connections SET name = ?2, base_url = ?3, author = ?4 WHERE id = ?1",
                params![id, name, base_url, author],
            )?;
        }
        self.connection(id)
    }

    /// Removes the connection only. Merge requests already imported stay (they keep their tasks).
    pub fn delete_connection(&self, id: i64) -> Result<(), AppError> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM git_connections WHERE id = ?1", params![id])?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------
// Storing changes and creating tasks
// ---------------------------------------------------------------------------------------

#[derive(Debug, Serialize, PartialEq, Default, Clone)]
pub struct MrImportReport {
    pub fetched: usize,
    /// Tasks created because a change had no matching task.
    pub new_tasks: usize,
    /// Changes attached to a task that already existed (found through the Jira key).
    pub linked_to_existing: usize,
    /// Changes imported before; only their state/title were refreshed.
    pub refreshed: usize,
    /// The new tasks, so the UI can offer "generate stories for these".
    pub new_task_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MergeRequest {
    pub id: i64,
    pub connection_id: Option<i64>,
    /// "gitlab" or "local"
    pub source: String,
    pub task_id: Option<i64>,
    pub reference: String,
    pub title: String,
    pub state: String,
    pub web_url: String,
    pub created_at: i64,
    pub merged_at: Option<i64>,
    pub role: String,
    pub jira_key: Option<String>,
}

impl TaskStore {
    /// Addresses of everything already imported, from every source.
    pub fn known_mr_urls(&self) -> Result<HashSet<String>, AppError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT web_url FROM merge_requests")?;
        let urls = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<HashSet<String>>>()?;
        Ok(urls)
    }

    /// Jira project keys we already know ("ABC" from "ABC-12"), to recognise keys in titles.
    fn known_project_keys(conn: &rusqlite::Connection) -> Result<HashSet<String>, AppError> {
        let mut stmt = conn.prepare("SELECT DISTINCT substr(jira_key, 1, instr(jira_key, '-') - 1) FROM tasks WHERE jira_key IS NOT NULL")?;
        let keys = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<HashSet<String>>>()?;
        Ok(keys.into_iter().map(|k| k.to_ascii_uppercase()).collect())
    }

    /// One task per change. A change that mentions a Jira key we already have attaches to that
    /// task instead of creating a duplicate. Re-importing never creates anything twice.
    pub fn import_changes(&self, connection_id: i64, source: &str, changes: &[GitChange]) -> Result<MrImportReport, AppError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let mut report = MrImportReport { fetched: changes.len(), ..Default::default() };
        let mut known_projects = Self::known_project_keys(&tx)?;

        for ch in changes {
            let already = tx
                .query_row("SELECT 1 FROM merge_requests WHERE web_url = ?1", params![ch.url], |_| Ok(()))
                .optional()?
                .is_some();
            if already {
                tx.execute(
                    "UPDATE merge_requests SET title = ?2, description = ?3, state = ?4, merged_at = ?5,
                            commit_notes = COALESCE(?6, commit_notes), connection_id = COALESCE(connection_id, ?7)
                     WHERE web_url = ?1",
                    params![ch.url, ch.title, ch.description, ch.state, ch.merged_at, ch.commit_notes, connection_id],
                )?;
                report.refreshed += 1;
                continue;
            }

            let key = find_jira_key(&[&ch.title, &ch.source_branch], &known_projects);
            let existing_task: Option<i64> = match &key {
                Some(k) => tx.query_row("SELECT id FROM tasks WHERE jira_key = ?1", params![k], |r| r.get(0)).optional()?,
                None => None,
            };
            let task_id = match existing_task {
                Some(id) => {
                    report.linked_to_existing += 1;
                    id
                }
                None => {
                    let title: String = ch.title.trim().chars().take(255).collect();
                    tx.execute(
                        "INSERT INTO tasks (title, jira_key, created_at) VALUES (?1, ?2, ?3)",
                        params![if title.is_empty() { ch.reference.clone() } else { title }, key, ch.created_at],
                    )?;
                    let id = tx.last_insert_rowid();
                    report.new_tasks += 1;
                    report.new_task_ids.push(id);
                    if let Some((project, _)) = key.as_deref().and_then(|k| k.rsplit_once('-')) {
                        known_projects.insert(project.to_string());
                    }
                    id
                }
            };
            tx.execute(
                "INSERT INTO merge_requests (connection_id, source, task_id, reference, title, description, state, web_url,
                        source_branch, created_at, merged_at, role, jira_key, commit_notes)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, COALESCE(?14, ''))",
                params![
                    connection_id, source, task_id, ch.reference, ch.title, ch.description, ch.state, ch.url,
                    ch.source_branch, ch.created_at, ch.merged_at, ch.role, key, ch.commit_notes
                ],
            )?;
        }
        tx.commit()?;
        Ok(report)
    }

    pub fn list_merge_requests(&self) -> Result<Vec<MergeRequest>, AppError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, connection_id, source, task_id, reference, title, state, web_url, created_at, merged_at, role, jira_key
             FROM merge_requests ORDER BY created_at DESC",
        )?;
        let list = stmt
            .query_map([], |r| {
                Ok(MergeRequest {
                    id: r.get(0)?,
                    connection_id: r.get(1)?,
                    source: r.get(2)?,
                    task_id: r.get(3)?,
                    reference: r.get(4)?,
                    title: r.get(5)?,
                    state: r.get(6)?,
                    web_url: r.get(7)?,
                    created_at: r.get(8)?,
                    merged_at: r.get(9)?,
                    role: r.get(10)?,
                    jira_key: r.get(11)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(list)
    }

    /// Text describing the work behind a task (its changes and commits), for story generation.
    pub fn mr_context(&self, task_id: i64) -> Result<String, AppError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT title, description, commit_notes FROM merge_requests WHERE task_id = ?1 ORDER BY created_at",
        )?;
        let parts = stmt
            .query_map(params![task_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let text = parts
            .iter()
            .map(|(title, description, commits)| {
                let mut s = format!("Merge request: {title}");
                if !description.trim().is_empty() {
                    s.push_str(&format!("\n{}", description.trim().chars().take(800).collect::<String>()));
                }
                if !commits.trim().is_empty() {
                    s.push_str(&format!("\nCommits:\n{}", commits.trim()));
                }
                s
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        Ok(text.chars().take(1800).collect())
    }
}

// ---------------------------------------------------------------------------------------
// What the Settings screen sees
// ---------------------------------------------------------------------------------------

/// A connection as the UI sees it. Note: no token, only "is there one".
#[derive(Debug, Serialize, PartialEq)]
pub struct GitConnection {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub base_url: String,
    pub author: Option<String>,
    pub has_token: bool,
    pub token_storage: Option<String>,
}

fn view(row: ConnRow, secrets: &SecretStore) -> Result<GitConnection, AppError> {
    let token = match &row.secret_name {
        Some(name) => secrets.get(name)?,
        None => None,
    };
    Ok(GitConnection {
        id: row.id,
        name: row.name,
        kind: row.kind,
        base_url: row.base_url,
        author: row.author,
        has_token: token.is_some(),
        token_storage: token.map(|(_, backend)| backend.label().to_string()),
    })
}

fn clean(s: Option<&str>) -> Option<String> {
    s.map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn default_name(kind: &str, base_url: &str) -> String {
    if kind == "local" {
        std::path::Path::new(base_url).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "Repository".into())
    } else {
        base_url.split("://").nth(1).unwrap_or(base_url).to_string()
    }
}

pub fn add_connection_impl(
    store: &TaskStore,
    secrets: &SecretStore,
    kind: &str,
    name: &str,
    base_url: &str,
    token: Option<&str>,
    author: Option<&str>,
) -> Result<GitConnection, AppError> {
    let (base_url, secret_name, author) = match kind {
        "gitlab" => {
            let url = normalize_base_url(if base_url.trim().is_empty() { gitlab::DEFAULT_URL } else { base_url })?;
            // A name that can't clash with another connection's, so each server keeps its own token.
            let secret = format!("git-token-{}", chrono::Utc::now().timestamp_micros());
            (url, Some(secret), None)
        }
        "local" => {
            let path = base_url.trim();
            if path.is_empty() {
                return Err(AppError::Config("Enter the folder of the repository".into()));
            }
            localgit::check_repo(std::path::Path::new(path))?;
            (path.to_string(), None, clean(author))
        }
        other => return Err(AppError::Invalid(format!("Unknown connection type: {other}"))),
    };
    let name = clean(Some(name)).unwrap_or_else(|| default_name(kind, &base_url));

    let row = store.add_connection(&name, kind, &base_url, secret_name.as_deref(), author.as_deref())?;
    if let (Some(secret), Some(token)) = (&row.secret_name, clean(token)) {
        if let Err(e) = secrets.set(secret, &token) {
            let _ = store.delete_connection(row.id); // don't leave a half-made connection behind
            return Err(e);
        }
    }
    view(row, secrets)
}

pub fn update_connection_impl(
    store: &TaskStore,
    secrets: &SecretStore,
    id: i64,
    name: &str,
    base_url: &str,
    token: Option<&str>,
    author: Option<&str>,
) -> Result<GitConnection, AppError> {
    let old = store.connection(id)?;
    let new_url = match old.kind.as_str() {
        "gitlab" => normalize_base_url(if base_url.trim().is_empty() { gitlab::DEFAULT_URL } else { base_url })?,
        _ => {
            let path = base_url.trim().to_string();
            if path != old.base_url {
                localgit::check_repo(std::path::Path::new(&path))?;
            }
            path
        }
    };
    if let Some(secret) = &old.secret_name {
        let new_token = clean(token);
        // Pointing a connection at a different server must not send the old token there.
        if new_url != old.base_url && new_token.is_none() {
            secrets.delete(secret)?;
        }
        if let Some(t) = new_token {
            secrets.set(secret, &t)?;
        }
    }
    let name = clean(Some(name)).unwrap_or(old.name);
    let row = store.update_connection(id, &name, &new_url, clean(author).as_deref())?;
    view(row, secrets)
}

pub fn remove_connection_impl(store: &TaskStore, secrets: &SecretStore, id: i64) -> Result<(), AppError> {
    let row = store.connection(id)?;
    if let Some(secret) = &row.secret_name {
        secrets.delete(secret)?;
    }
    store.delete_connection(id)
}

// ---------------------------------------------------------------------------------------
// Importing from many sources
// ---------------------------------------------------------------------------------------

#[derive(Debug, Serialize, PartialEq)]
pub struct ConnectionImport {
    pub connection_id: i64,
    pub name: String,
    pub error: Option<String>,
    pub report: Option<MrImportReport>,
}

#[derive(Debug, Serialize, PartialEq, Default)]
pub struct ImportSummary {
    pub connections: Vec<ConnectionImport>,
    pub fetched: usize,
    pub new_tasks: usize,
    pub linked_to_existing: usize,
    pub refreshed: usize,
    pub new_task_ids: Vec<i64>,
}

/// Read one source and store what it returned.
pub async fn import_one(
    store: &TaskStore,
    row: &ConnRow,
    token: Option<&str>,
    since: i64,
    include_assigned: bool,
    state: &str,
) -> Result<MrImportReport, AppError> {
    let known = store.known_mr_urls()?;
    let (source, changes) = match row.kind.as_str() {
        "gitlab" => {
            let token = token.ok_or_else(|| AppError::Config(format!("Add an access token for \"{}\" in Settings first", row.name)))?;
            let client = GitlabClient::new(&row.base_url, token)?;
            ("gitlab", gitlab::fetch_changes(&client, since, include_assigned, state, &known).await?)
        }
        "local" => {
            // git can take a while on a big repository: keep it off the async threads.
            let path = PathBuf::from(&row.base_url);
            let author = row.author.clone();
            let changes = tauri::async_runtime::spawn_blocking(move || localgit::fetch_changes(&path, author.as_deref(), since))
                .await
                .map_err(|e| AppError::Config(format!("Reading the repository failed: {e}")))??;
            ("local", changes)
        }
        other => return Err(AppError::Invalid(format!("Unknown connection type: {other}"))),
    };
    store.import_changes(row.id, source, &changes)
}

/// Import from each chosen connection in turn. One failing server (wrong token, offline) is
/// reported on its own line and never stops the others.
pub async fn import_many(
    store: &TaskStore,
    secrets: &SecretStore,
    connection_ids: &[i64],
    since: i64,
    include_assigned: bool,
    state: &str,
) -> Result<ImportSummary, AppError> {
    if !["all", "merged", "opened", "closed"].contains(&state) {
        return Err(AppError::Invalid(format!("Unknown merge request state: {state}")));
    }
    if connection_ids.is_empty() {
        return Err(AppError::Invalid("Choose at least one connection to import from".into()));
    }

    let mut summary = ImportSummary::default();
    for &id in connection_ids {
        let row = match store.connection(id) {
            Ok(r) => r,
            Err(e) => {
                summary.connections.push(ConnectionImport { connection_id: id, name: String::new(), error: Some(e.to_string()), report: None });
                continue;
            }
        };
        let token = match &row.secret_name {
            Some(name) => secrets.get(name).ok().flatten().map(|(t, _)| t),
            None => None,
        };
        match import_one(store, &row, token.as_deref(), since, include_assigned, state).await {
            Ok(report) => {
                summary.fetched += report.fetched;
                summary.new_tasks += report.new_tasks;
                summary.linked_to_existing += report.linked_to_existing;
                summary.refreshed += report.refreshed;
                summary.new_task_ids.extend(report.new_task_ids.iter().copied());
                summary.connections.push(ConnectionImport { connection_id: id, name: row.name, error: None, report: Some(report) });
            }
            Err(e) => summary.connections.push(ConnectionImport { connection_id: id, name: row.name, error: Some(e.to_string()), report: None }),
        }
    }
    Ok(summary)
}

// ---------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------

#[tauri::command]
pub fn list_git_connections(store: State<'_, TaskStore>, secrets: State<'_, SecretStore>) -> Result<Vec<GitConnection>, AppError> {
    store.connections()?.into_iter().map(|row| view(row, &secrets)).collect()
}

/// `kind` is "gitlab" or "local". For GitLab, `base_url` is the server address; for a local
/// repository it is the folder path.
#[tauri::command]
pub fn add_git_connection(
    kind: String,
    name: String,
    base_url: String,
    token: Option<String>,
    author: Option<String>,
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<GitConnection, AppError> {
    add_connection_impl(&store, &secrets, &kind, &name, &base_url, token.as_deref(), author.as_deref())
}

/// `token: None` or empty means "keep the token I already saved".
#[tauri::command]
pub fn update_git_connection(
    id: i64,
    name: String,
    base_url: String,
    token: Option<String>,
    author: Option<String>,
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<GitConnection, AppError> {
    update_connection_impl(&store, &secrets, id, &name, &base_url, token.as_deref(), author.as_deref())
}

#[tauri::command]
pub fn remove_git_connection(id: i64, store: State<'_, TaskStore>, secrets: State<'_, SecretStore>) -> Result<(), AppError> {
    remove_connection_impl(&store, &secrets, id)
}

#[tauri::command]
pub fn clear_git_token(id: i64, store: State<'_, TaskStore>, secrets: State<'_, SecretStore>) -> Result<GitConnection, AppError> {
    let row = store.connection(id)?;
    if let Some(secret) = &row.secret_name {
        secrets.delete(secret)?;
    }
    view(row, &secrets)
}

/// Does this connection work? GitLab: who the token belongs to. Local: which commit email is used.
#[tauri::command]
pub async fn test_git_connection(id: i64, store: State<'_, TaskStore>, secrets: State<'_, SecretStore>) -> Result<String, AppError> {
    let row = store.connection(id)?;
    match row.kind.as_str() {
        "gitlab" => {
            let (token, _) = row
                .secret_name
                .as_deref()
                .map(|n| secrets.get(n))
                .transpose()?
                .flatten()
                .ok_or_else(|| AppError::Config(format!("Add an access token for \"{}\" first", row.name)))?;
            let who = GitlabClient::new(&row.base_url, &token)?.current_user().await?;
            Ok(format!("Connected to {} as {who}", row.name))
        }
        _ => {
            let path = PathBuf::from(&row.base_url);
            localgit::check_repo(&path)?;
            let author = row.author.clone().or_else(|| localgit::configured_email(&path));
            Ok(match author {
                Some(a) => format!("{} is a git repository. Looking for commits by {a}", row.name),
                None => format!("{} is a git repository, but no commit email is set: enter yours in its settings", row.name),
            })
        }
    }
}

#[tauri::command]
pub async fn import_git_changes(
    connection_ids: Vec<i64>,
    since: i64,
    include_assigned: bool,
    state: String,
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<ImportSummary, AppError> {
    import_many(&store, &secrets, &connection_ids, since, include_assigned, &state).await
}

#[tauri::command]
pub fn list_merge_requests(store: State<'_, TaskStore>) -> Result<Vec<MergeRequest>, AppError> {
    store.list_merge_requests()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn known(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pj-git-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    // --- finding Jira keys ---

    #[test]
    fn keys_are_found_in_titles_and_branches() {
        let k = known(&["ABC"]);
        let find = |t: &str| find_jira_key(&[t], &k);
        assert_eq!(find("ABC-123: fix login"), Some("ABC-123".into()));
        assert_eq!(find("[ABC-7] Add Apple Pay"), Some("ABC-7".into()));
        assert_eq!(find("feature/abc-123-apple-pay"), Some("ABC-123".into())); // lower case, known project
        assert_eq!(find("Fixes XYZ-9 and ABC-1"), Some("ABC-1".into())); // XYZ isn't one of our projects
        assert_eq!(find("ABC-123abc"), None);
        assert_eq!(find("xABC-1"), None); // glued to a letter: part of another word
        assert_eq!(find("no key here"), None);
        assert_eq!(find_jira_key(&["fix typo", "ABC-5-branch"], &k), Some("ABC-5".into())); // later text is searched too
    }

    #[test]
    fn without_known_projects_only_plausible_upper_case_keys_count() {
        let none = known(&[]);
        assert_eq!(find_jira_key(&["ABC-123 fix"], &none), Some("ABC-123".into()));
        assert_eq!(find_jira_key(&["feature/abc-123"], &none), None); // lower case: too risky to guess
        assert_eq!(find_jira_key(&["Support UTF-8 and SHA-256"], &none), None);
    }

    // --- importing into tasks ---

    fn change(n: i64, title: &str, branch: &str) -> GitChange {
        GitChange {
            url: format!("https://gitlab.example/mr/{n}"),
            reference: format!("g/p!{n}"),
            title: title.into(),
            description: "Details".into(),
            state: "opened".into(),
            source_branch: branch.into(),
            created_at: 1_790_000_000 + n,
            merged_at: None,
            role: "author".into(),
            commit_notes: Some("- first commit".into()),
        }
    }

    fn with_connection(store: &TaskStore) -> i64 {
        store.add_connection("Work GitLab", "gitlab", "https://gitlab.example", Some("tok-1"), None).unwrap().id
    }

    #[test]
    fn a_change_without_a_key_becomes_one_new_task_and_reimport_adds_nothing() {
        let store = TaskStore::in_memory();
        let c = with_connection(&store);
        let report = store.import_changes(c, "gitlab", &[change(1, "Speed up search", "feature/search")]).unwrap();
        assert_eq!((report.new_tasks, report.linked_to_existing, report.refreshed), (1, 0, 0));

        let tasks = store.list().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!((tasks[0].title.as_str(), tasks[0].created_at), ("Speed up search", 1_790_000_001));
        assert_eq!(tasks[0].jira_key, None);

        // Same change again, now merged: refreshed, nothing new.
        let mut merged = change(1, "Speed up search", "feature/search");
        merged.state = "merged".into();
        merged.commit_notes = None;
        let again = store.import_changes(c, "gitlab", &[merged]).unwrap();
        assert_eq!((again.new_tasks, again.refreshed), (0, 1));
        assert_eq!(store.list().unwrap().len(), 1);
        let mrs = store.list_merge_requests().unwrap();
        assert_eq!((mrs[0].state.as_str(), mrs[0].task_id, mrs[0].connection_id), ("merged", Some(tasks[0].id), Some(c)));
        assert!(store.mr_context(tasks[0].id).unwrap().contains("- first commit")); // old notes kept
    }

    #[test]
    fn a_change_that_mentions_an_existing_issue_attaches_to_it() {
        let store = TaskStore::in_memory();
        let c = with_connection(&store);
        let existing = store.add("Fix login").unwrap();
        store.link_jira_key(existing.id, "ABC-12").unwrap();

        let report = store.import_changes(c, "gitlab", &[change(1, "ABC-12: fix redirect loop", "feature/x")]).unwrap();
        assert_eq!((report.new_tasks, report.linked_to_existing), (0, 1));
        assert_eq!(store.list().unwrap().len(), 1); // no duplicate task
        assert_eq!(store.list_merge_requests().unwrap()[0].task_id, Some(existing.id));
        assert_eq!(store.list_merge_requests().unwrap()[0].jira_key.as_deref(), Some("ABC-12"));
    }

    #[test]
    fn a_change_with_an_unknown_issue_key_creates_a_task_linked_to_that_issue() {
        let store = TaskStore::in_memory();
        let c = with_connection(&store);
        let seed = store.add("seed").unwrap();
        store.link_jira_key(seed.id, "ABC-1").unwrap(); // makes "ABC" a known project
        store.import_changes(c, "gitlab", &[change(1, "Add Apple Pay", "feature/abc-99-apple-pay")]).unwrap();

        let task = store.list().unwrap().into_iter().find(|t| t.jira_key.as_deref() == Some("ABC-99")).unwrap();
        assert_eq!(task.title, "Add Apple Pay");
    }

    #[test]
    fn a_deleted_task_does_not_come_back_on_the_next_import() {
        let store = TaskStore::in_memory();
        let c = with_connection(&store);
        store.import_changes(c, "gitlab", &[change(1, "Temp work", "b")]).unwrap();
        let id = store.list().unwrap()[0].id;
        store.remove(id).unwrap();

        let report = store.import_changes(c, "gitlab", &[change(1, "Temp work", "b")]).unwrap();
        assert_eq!((report.new_tasks, report.refreshed), (0, 1));
        assert!(store.list().unwrap().is_empty());
        assert_eq!(store.list_merge_requests().unwrap()[0].task_id, None);
    }

    #[test]
    fn two_servers_with_the_same_merge_request_number_do_not_collide() {
        let store = TaskStore::in_memory();
        let work = with_connection(&store);
        let client = store.add_connection("Client GitLab", "gitlab", "https://git.client.example", Some("tok-2"), None).unwrap().id;
        let mut theirs = change(1, "Their MR number one", "b");
        theirs.url = "https://git.client.example/mr/1".into();

        store.import_changes(work, "gitlab", &[change(1, "My MR number one", "b")]).unwrap();
        store.import_changes(client, "gitlab", &[theirs]).unwrap();

        assert_eq!(store.list().unwrap().len(), 2); // both exist: same number "1", different servers
        let mrs = store.list_merge_requests().unwrap();
        assert_eq!(mrs.iter().filter(|m| m.connection_id == Some(work)).count(), 1);
        assert_eq!(mrs.iter().filter(|m| m.connection_id == Some(client)).count(), 1);
    }

    #[test]
    fn story_context_collects_title_description_and_commits() {
        let store = TaskStore::in_memory();
        let c = with_connection(&store);
        store.import_changes(c, "gitlab", &[change(1, "Add search", "b")]).unwrap();
        let ctx = store.mr_context(store.list().unwrap()[0].id).unwrap();
        assert!(ctx.contains("Merge request: Add search") && ctx.contains("Details") && ctx.contains("- first commit"), "got: {ctx}");
    }

    #[test]
    fn removing_a_connection_keeps_what_was_imported() {
        let store = TaskStore::in_memory();
        let c = with_connection(&store);
        store.import_changes(c, "gitlab", &[change(1, "Keep me", "b")]).unwrap();
        store.delete_connection(c).unwrap();
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(store.list_merge_requests().unwrap()[0].connection_id, None);
    }

    // --- managing connections ---

    #[test]
    fn each_gitlab_server_gets_its_own_token_and_changing_the_address_forgets_it() {
        let dir = temp_dir("conns");
        let store = TaskStore::in_memory();
        let secrets = SecretStore::file_only(dir.clone());

        let a = add_connection_impl(&store, &secrets, "gitlab", "Work", "gitlab.com", Some("glpat-A"), None).unwrap();
        let b = add_connection_impl(&store, &secrets, "gitlab", "", "https://git.client.example/some/path", Some("glpat-B"), None).unwrap();
        assert_eq!((a.base_url.as_str(), b.base_url.as_str()), ("https://gitlab.com", "https://git.client.example"));
        assert_eq!(b.name, "git.client.example"); // a sensible default name
        assert!(a.has_token && b.has_token && !format!("{a:?}{b:?}").contains("glpat-"));

        let (ra, rb) = (store.connection(a.id).unwrap(), store.connection(b.id).unwrap());
        assert_ne!(ra.secret_name, rb.secret_name);
        assert_eq!(secrets.get(ra.secret_name.as_deref().unwrap()).unwrap().unwrap().0, "glpat-A");
        assert_eq!(secrets.get(rb.secret_name.as_deref().unwrap()).unwrap().unwrap().0, "glpat-B");

        // Same server: the token is kept. A different server without a new token: it is forgotten.
        assert!(update_connection_impl(&store, &secrets, a.id, "Work", "https://gitlab.com", None, None).unwrap().has_token);
        assert!(!update_connection_impl(&store, &secrets, a.id, "Work", "https://other.example", None, None).unwrap().has_token);
        assert!(secrets.get(rb.secret_name.as_deref().unwrap()).unwrap().is_some()); // the other server's token is untouched

        // http is refused, and removing deletes the token.
        assert!(add_connection_impl(&store, &secrets, "gitlab", "x", "http://gitlab.example.com", None, None).is_err());
        remove_connection_impl(&store, &secrets, b.id).unwrap();
        assert!(secrets.get(rb.secret_name.as_deref().unwrap()).unwrap().is_none());
        assert_eq!(store.connections().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn local_repositories_must_be_git_folders_and_unknown_kinds_are_refused() {
        let dir = temp_dir("local");
        std::fs::create_dir_all(&dir).unwrap();
        let store = TaskStore::in_memory();
        let secrets = SecretStore::file_only(dir.join("secrets"));
        assert!(add_connection_impl(&store, &secrets, "local", "x", dir.to_str().unwrap(), None, None).is_err()); // a folder, but no .git
        assert!(add_connection_impl(&store, &secrets, "local", "x", "", None, None).is_err());
        assert!(add_connection_impl(&store, &secrets, "svn", "x", "y", None, None).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    // --- importing from several sources at once ---

    fn raw_mr(iid: i64, title: &str) -> serde_json::Value {
        json!({
            "id": iid, "iid": iid, "project_id": 7, "title": title, "description": "d", "state": "merged",
            "web_url": format!("PLACEHOLDER/mr/{iid}"), "source_branch": "b", "created_at": "2026-10-03T10:15:30.000Z",
            "references": { "full": format!("g/p!{iid}") }
        })
    }

    async fn gitlab_with(mrs: Vec<serde_json::Value>, token: &str) -> MockServer {
        let server = MockServer::start().await;
        let uri = server.uri();
        let body: Vec<_> = mrs
            .into_iter()
            .map(|mut m| {
                let url = m["web_url"].as_str().unwrap().replace("PLACEHOLDER", &uri);
                m["web_url"] = json!(url);
                m
            })
            .collect();
        Mock::given(method("GET"))
            .and(path("/api/v4/merge_requests"))
            .and(header("PRIVATE-TOKEN", token))
            .and(query_param("scope", "created_by_me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(wiremock::matchers::path_regex(r"^/api/v4/projects/7/merge_requests/\d+/commits$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "title": "a commit" }])))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn each_server_is_read_with_its_own_token_and_one_failure_does_not_stop_the_rest() {
        let dir = temp_dir("many");
        let store = TaskStore::in_memory();
        let secrets = SecretStore::file_only(dir.clone());
        let work = gitlab_with(vec![raw_mr(1, "Work MR")], "tok-work").await;
        let client = gitlab_with(vec![raw_mr(1, "Client MR")], "tok-client").await;

        let a = add_connection_impl(&store, &secrets, "gitlab", "Work", &work.uri(), Some("tok-work"), None).unwrap();
        let b = add_connection_impl(&store, &secrets, "gitlab", "Client", &client.uri(), Some("tok-client"), None).unwrap();
        let broken = add_connection_impl(&store, &secrets, "gitlab", "No token", &client.uri(), None, None).unwrap();

        let summary = import_many(&store, &secrets, &[a.id, broken.id, b.id], 0, false, "all").await.unwrap();

        assert_eq!(summary.connections.len(), 3);
        assert!(summary.connections[0].error.is_none() && summary.connections[2].error.is_none());
        assert!(summary.connections[1].error.as_deref().unwrap().contains("access token for \"No token\""));
        assert_eq!((summary.fetched, summary.new_tasks), (2, 2));
        let titles: HashSet<_> = store.list().unwrap().into_iter().map(|t| t.title).collect();
        assert_eq!(titles, HashSet::from(["Work MR".to_string(), "Client MR".to_string()]));

        // Importing again changes nothing.
        let again = import_many(&store, &secrets, &[a.id, b.id], 0, false, "all").await.unwrap();
        assert_eq!((again.new_tasks, again.refreshed), (0, 2));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_wrong_token_is_reported_for_that_server_only() {
        let dir = temp_dir("wrongtoken");
        let store = TaskStore::in_memory();
        let secrets = SecretStore::file_only(dir.clone());
        let server = gitlab_with(vec![raw_mr(1, "Never seen")], "the-right-token").await; // only answers to this token
        let c = add_connection_impl(&store, &secrets, "gitlab", "Bad", &server.uri(), Some("the-wrong-token"), None).unwrap();

        let summary = import_many(&store, &secrets, &[c.id], 0, false, "all").await.unwrap();
        assert!(summary.connections[0].error.is_some()); // the fake answers 404 to an unknown token: reported, not fatal
        assert!(store.list().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_import_needs_a_selection_and_a_known_state() {
        let store = TaskStore::in_memory();
        let secrets = SecretStore::file_only(temp_dir("sel"));
        assert!(import_many(&store, &secrets, &[], 0, true, "all").await.is_err());
        assert!(import_many(&store, &secrets, &[1], 0, true, "bogus").await.unwrap_err().to_string().contains("Unknown merge request state"));
    }

    #[tokio::test]
    async fn local_repositories_import_alongside_servers() {
        use std::process::Command;
        let dir = temp_dir("localimport");
        std::fs::create_dir_all(&dir).unwrap();
        let run = |args: &[&str]| {
            let ok = Command::new("git")
                .args(args)
                .current_dir(&dir)
                .env("GIT_AUTHOR_NAME", "me").env("GIT_AUTHOR_EMAIL", "me@example.com")
                .env("GIT_COMMITTER_NAME", "me").env("GIT_COMMITTER_EMAIL", "me@example.com")
                .env("GIT_AUTHOR_DATE", "2026-10-02T09:00:00Z").env("GIT_COMMITTER_DATE", "2026-10-02T09:00:00Z")
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("a"), "1").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "Initial"]);
        run(&["checkout", "-q", "-b", "feature/speed-up-search"]);
        std::fs::write(dir.join("b"), "2").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "Add an index"]);

        let store = TaskStore::in_memory();
        let secrets = SecretStore::file_only(dir.join("secrets"));
        let repo = add_connection_impl(&store, &secrets, "local", "", dir.to_str().unwrap(), None, Some("me@example.com")).unwrap();
        assert_eq!(repo.kind, "local");

        let since = chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z").unwrap().timestamp();
        let summary = import_many(&store, &secrets, &[repo.id], since, false, "all").await.unwrap();
        assert_eq!((summary.new_tasks, summary.connections[0].error.clone()), (1, None));

        let task = &store.list().unwrap()[0];
        assert_eq!(task.title, "Speed up search");
        let mrs = store.list_merge_requests().unwrap();
        assert_eq!((mrs[0].source.as_str(), mrs[0].state.as_str()), ("local", "local"));
        assert!(store.mr_context(task.id).unwrap().contains("- Add an index"));

        // The same branch again is recognised, not duplicated.
        let again = import_many(&store, &secrets, &[repo.id], since, false, "all").await.unwrap();
        assert_eq!((again.new_tasks, again.refreshed), (0, 1));
        let _ = std::fs::remove_dir_all(dir);
    }
}
