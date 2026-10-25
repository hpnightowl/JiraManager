//! Jira Cloud: settings, the HTTP client, and syncing assigned issues into local tasks.
//! The token is read just before a request and never returned to the UI, redirects are not followed,
//! and changing the site URL drops the old token.

use crate::error::AppError;
use crate::fields::{self, FieldSpec};
use crate::secrets::SecretStore;
use crate::tasks::TaskStore;
use reqwest::{redirect, Client, RequestBuilder, Url};
use rusqlite::{params, OptionalExtension};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::time::Duration;
use tauri::State;

const TOKEN_NAME: &str = "jira-api-token";
pub(crate) const KEY_URL: &str = "jira.base_url";
const KEY_EMAIL: &str = "jira.email";

const KEY_FETCH_DAYS: &str = "jira.fetch_days";
/// How far back to look for issues that are already resolved. 0 = only open ones.
pub const FETCH_DAYS_CHOICES: &[u32] = &[0, 14, 30, 60, 90];
pub const DEFAULT_FETCH_DAYS: u32 = 30;

/// The issues assigned to me. By default also the ones I RESOLVED recently: at month end you log
/// time and write stories for work that is already finished, and an "unresolved only" search
/// would never show those (which is how fixed defects went missing).
pub fn build_jql(fetch_days: u32) -> String {
    if fetch_days == 0 {
        "assignee = currentUser() AND resolution = Unresolved ORDER BY updated DESC".to_string()
    } else {
        format!(
            "assignee = currentUser() AND (resolution = Unresolved OR resolved >= -{fetch_days}d) ORDER BY updated DESC"
        )
    }
}

/// Everything I created, of the normal issue types (`standardIssueTypes()` leaves out sub-tasks).
const REPORTED_JQL: &str = "reporter = currentUser() AND issuetype in standardIssueTypes() ORDER BY created DESC";

/// Everything assigned to me that is in a "Done" status (whatever the workflow calls it), most
/// recently finished first. Sub-tasks are left out, like in "reported".
const DONE_JQL: &str =
    "assignee = currentUser() AND statusCategory = Done AND issuetype in standardIssueTypes() ORDER BY resolutiondate DESC, updated DESC";

/// A Jira issue's description and latest comments, as plain text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IssueDetail {
    pub description: String,
    pub comments: Vec<IssueComment>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IssueComment {
    pub author: String,
    pub text: String,
}

#[derive(Deserialize)]
struct RawIssueDetail {
    fields: RawDetailFields,
}

#[derive(Deserialize)]
struct RawDetailFields {
    description: Option<Value>,
    comment: Option<RawComments>,
}

#[derive(Deserialize)]
struct RawComments {
    #[serde(default)]
    comments: Vec<RawComment>,
}

#[derive(Deserialize)]
struct RawComment {
    author: Option<RawAuthor>,
    #[serde(default)]
    body: Value,
}

#[derive(Deserialize)]
struct RawAuthor {
    #[serde(rename = "displayName", default)]
    display_name: String,
}

/// What we ask Jira to include for each issue (it only returns the issue id otherwise).
const ISSUE_FIELDS: &str = "summary,status,issuetype,parent,created,timespent,resolutiondate,statuscategorychangedate";
const PAGE_SIZE: &str = "50";
const MAX_PAGES: usize = 10; // at most 500 issues per fetch

// ---------------------------------------------------------------------------------------
// URL handling
// ---------------------------------------------------------------------------------------

/// "yourcompany.atlassian.net/jira/..." -> "https://yourcompany.atlassian.net".
/// Plain `http://` is only accepted for localhost (used by tests and local mock servers).
pub fn normalize_base_url(input: &str) -> Result<String, AppError> {
    let bad = |msg: &str| AppError::Config(msg.to_string());
    let input = input.trim();
    if input.is_empty() {
        return Err(bad("Enter your Jira site URL, e.g. https://yourcompany.atlassian.net"));
    }
    let with_scheme = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
    let url = Url::parse(&with_scheme).map_err(|_| bad("That doesn't look like a valid URL"))?;
    let host = url.host_str().ok_or_else(|| bad("The URL has no host name"))?;
    let is_local = matches!(host, "localhost" | "127.0.0.1" | "[::1]");

    match url.scheme() {
        "https" => {}
        "http" if is_local => {}
        _ => return Err(bad("The Jira URL must use https:// (http is only allowed for localhost)")),
    }
    // Keep scheme + host + port only: paths like /jira/software/... are dropped.
    Ok(url.origin().ascii_serialization())
}

// ---------------------------------------------------------------------------------------
// HTTP client
// ---------------------------------------------------------------------------------------

/// The parent Jira reports for an issue (an epic, or a story for a sub-task).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct JiraParent {
    pub key: String,
    pub summary: String,
    pub issue_type: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct JiraIssue {
    pub key: String,
    pub summary: String,
    pub status: Option<String>,
    pub issue_type: Option<String>,
    /// Unix seconds; when Jira says the issue was created.
    pub created_at: Option<i64>,
    /// Time already logged in Jira, in seconds (None if time tracking is off).
    pub logged_seconds: Option<i64>,
    pub parent: Option<JiraParent>,
    /// Jira has a resolution date for it: the work is finished.
    pub resolved: bool,
    /// Unix seconds; when it reached Done (its resolution date, else when its status last moved
    /// to a different category such as "In Progress" -> "Done").
    pub done_at: Option<i64>,
}

pub struct JiraClient {
    http: Client,
    base: String,
    email: String,
    token: String,
}

// Shapes of Jira's JSON. Deserialize ignores every field we don't list.
#[derive(Deserialize)]
struct MyselfResponse {
    #[serde(rename = "accountId")]
    account_id: String,
    #[serde(rename = "displayName")]
    display_name: String,
}

#[derive(Deserialize)]
struct SearchPage {
    #[serde(default)]
    issues: Vec<RawIssue>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
    #[serde(rename = "isLast", default)]
    is_last: bool,
}

#[derive(Deserialize)]
struct RawIssue {
    key: String,
    fields: RawFields,
}

#[derive(Deserialize)]
struct RawFields {
    #[serde(default)]
    summary: String,
    status: Option<RawNamed>,
    issuetype: Option<RawNamed>,
    parent: Option<RawParent>,
    /// e.g. "2026-09-03T10:15:30.123+0530"
    created: Option<String>,
    /// Seconds already logged on this issue.
    timespent: Option<i64>,
    /// Set when the issue was resolved.
    resolutiondate: Option<String>,
    /// When the status last moved to another category (used when there is no resolution date).
    statuscategorychangedate: Option<String>,
}

/// Jira sends many objects of the shape `{ "name": "..." }` (status, issue type...).
#[derive(Deserialize)]
struct RawNamed {
    name: String,
}

/// Jira includes the parent's key AND its summary and type, so no second request is needed.
#[derive(Deserialize)]
struct RawParent {
    key: String,
    fields: Option<RawParentFields>,
}

#[derive(Deserialize)]
struct RawParentFields {
    #[serde(default)]
    summary: String,
    issuetype: Option<RawNamed>,
}

fn parse_jira_time(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.3f%z").ok().map(|d| d.timestamp())
}

impl JiraClient {
    pub fn new(base_url: &str, email: &str, token: &str) -> Result<Self, AppError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(redirect::Policy::none())
            .user_agent("ProjectJira/0.1")
            .build()?;
        Ok(Self {
            http,
            base: normalize_base_url(base_url)?,
            email: email.trim().to_string(),
            token: token.to_string(),
        })
    }

    fn get(&self, path: &str) -> RequestBuilder {
        self.http
            .get(format!("{}{path}", self.base))
            .basic_auth(&self.email, Some(&self.token))
            .header("Accept", "application/json")
    }

    fn post(&self, path: &str) -> RequestBuilder {
        self.http
            .post(format!("{}{path}", self.base))
            .basic_auth(&self.email, Some(&self.token))
            .header("Accept", "application/json")
    }

    fn put(&self, path: &str) -> RequestBuilder {
        self.http
            .put(format!("{}{path}", self.base))
            .basic_auth(&self.email, Some(&self.token))
            .header("Accept", "application/json")
    }

    /// Send, parse the JSON body.
    async fn send<T: DeserializeOwned>(&self, req: RequestBuilder) -> Result<T, AppError> {
        Ok(self.execute(req).await?.json::<T>().await?)
    }

    /// Send and ignore the body (Jira answers "204 No Content" to updates).
    async fn send_unit(&self, req: RequestBuilder) -> Result<(), AppError> {
        self.execute(req).await?;
        Ok(())
    }

    /// Send, and turn HTTP failures into messages a person can act on.
    async fn execute(&self, req: RequestBuilder) -> Result<reqwest::Response, AppError> {
        let resp = req.send().await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let message = match status.as_u16() {
            401 => "Jira rejected the email or API token. Check both in Settings \
                    (create a token at id.atlassian.com > Security > API tokens)."
                .to_string(),
            403 => "Jira accepted the login but denied access to this resource (403).".to_string(),
            404 => "Jira answered 404 (not found). Check the site URL, and that the project or issue exists \
                    and you have permission to see it."
                .to_string(),
            429 => "Jira is rate-limiting requests (429). Wait a minute and try again.".to_string(),
            300..=399 => "Jira tried to redirect the request. Use the exact site URL, \
                          e.g. https://yourcompany.atlassian.net."
                .to_string(),
            400 => {
                let body = resp.text().await.unwrap_or_default();
                let short: String = body.chars().take(600).collect();
                format!("Jira rejected the request (400): {short}")
            }
            code => format!("Jira returned HTTP {code}."),
        };
        Err(AppError::Jira(message))
    }

    /// Who am I? Also the cheapest way to check that URL + email + token all work.
    pub async fn me(&self) -> Result<JiraUser, AppError> {
        let me: MyselfResponse = self.send(self.get("/rest/api/3/myself")).await?;
        Ok(JiraUser { account_id: me.account_id, display_name: me.display_name })
    }

    pub async fn myself(&self) -> Result<String, AppError> {
        Ok(self.me().await?.display_name)
    }

    /// Set (or change) the parent of an existing issue, e.g. put a story under an epic.
    pub async fn set_parent(&self, issue_key: &str, parent_key: &str) -> Result<(), AppError> {
        let path = format!("/rest/api/3/issue/{}", segment(issue_key)?);
        let body = json!({ "fields": { "parent": { "key": segment(parent_key)? } } });
        self.send_unit(self.put(&path).json(&body)).await
    }

    /// Issues assigned to me: the open ones, plus (when `fetch_days` > 0) those I resolved in the
    /// last `fetch_days` days. Uses the current search endpoint (`/search/jql`, paged with
    /// `nextPageToken`); the old `/search` endpoint was removed from Jira Cloud.
    pub async fn my_issues(&self, fetch_days: u32) -> Result<Vec<JiraIssue>, AppError> {
        self.search(&build_jql(fetch_days)).await
    }

    /// Issues I REPORTED (created), of the normal issue types (no sub-tasks), newest first.
    /// The basis of "how many stories have I created": it includes history from before this app.
    pub async fn my_reported(&self) -> Result<Vec<JiraIssue>, AppError> {
        self.search(REPORTED_JQL).await
    }

    /// Issues assigned to me that are Done: how much I have finished, with the date. History included,
    /// newest first, up to what one fetch holds (the latest 500).
    pub async fn my_done(&self) -> Result<Vec<JiraIssue>, AppError> {
        self.search(DONE_JQL).await
    }

    async fn search(&self, jql: &str) -> Result<Vec<JiraIssue>, AppError> {
        let mut issues = Vec::new();
        let mut page_token: Option<String> = None;

        for _ in 0..MAX_PAGES {
            let mut req = self.get("/rest/api/3/search/jql").query(&[
                ("jql", jql),
                ("fields", ISSUE_FIELDS),
                ("maxResults", PAGE_SIZE),
            ]);
            if let Some(token) = &page_token {
                req = req.query(&[("nextPageToken", token.as_str())]);
            }
            let page: SearchPage = self.send(req).await?;

            issues.extend(page.issues.into_iter().map(|raw| {
                let f = raw.fields;
                JiraIssue {
                    key: raw.key,
                    summary: f.summary,
                    status: f.status.map(|s| s.name),
                    issue_type: f.issuetype.map(|t| t.name),
                    created_at: f.created.as_deref().and_then(parse_jira_time),
                    logged_seconds: f.timespent,
                    resolved: f.resolutiondate.is_some(),
                    done_at: f.resolutiondate.as_deref().and_then(parse_jira_time).or_else(|| f.statuscategorychangedate.as_deref().and_then(parse_jira_time)),
                    parent: f.parent.map(|p| {
                        let pf = p.fields;
                        JiraParent {
                            key: p.key,
                            summary: pf.as_ref().map(|x| x.summary.clone()).unwrap_or_default(),
                            issue_type: pf.and_then(|x| x.issuetype).map(|t| t.name),
                        }
                    }),
                }
            }));

            match page.next_page_token {
                Some(next) if !page.is_last => page_token = Some(next),
                _ => break,
            }
        }
        Ok(issues)
    }

    /// An issue's description and its latest comments as plain text. This is what turns a bare title
    /// ("Totals wrong after a refund") into something the local AI can write a real story from.
    pub async fn issue_detail(&self, key: &str) -> Result<IssueDetail, AppError> {
        let path = format!("/rest/api/3/issue/{}", segment(key)?);
        let raw: RawIssueDetail = self.send(self.get(&path).query(&[("fields", "description,comment")])).await?;
        let comments = raw
            .fields
            .comment
            .map(|c| c.comments)
            .unwrap_or_default()
            .into_iter()
            .map(|c| IssueComment {
                author: c.author.map(|a| a.display_name).unwrap_or_else(|| "Someone".into()),
                text: crate::adf::adf_to_text(&c.body),
            })
            .filter(|c| !c.text.is_empty())
            .collect();
        Ok(IssueDetail { description: raw.fields.description.map(|d| crate::adf::adf_to_text(&d)).unwrap_or_default(), comments })
    }

    // --- creating issues and logging work ------------------------------------------------

    /// Projects I can see (for the "where should this go?" picker).
    pub async fn projects(&self) -> Result<Vec<JiraProject>, AppError> {
        let mut all = Vec::new();
        let mut start = 0usize;
        for _ in 0..10 {
            let page: ProjectPage = self
                .send(self.get("/rest/api/3/project/search").query(&[
                    ("maxResults", "100".to_string()),
                    ("startAt", start.to_string()),
                    ("orderBy", "name".to_string()),
                ]))
                .await?;
            let got = page.values.len();
            start += got;
            all.extend(page.values);
            if got == 0 || page.is_last {
                break;
            }
        }
        Ok(all)
    }

    pub async fn issue_types(&self, project_key: &str) -> Result<Vec<IssueType>, AppError> {
        let path = format!("/rest/api/3/issue/createmeta/{}/issuetypes", segment(project_key)?);
        let mut all: Vec<IssueType> = Vec::new();
        for _ in 0..10 {
            let page: IssueTypePage = self.send(self.get(&path).query(&[("startAt", all.len().to_string())])).await?;
            let got = page.issue_types.len();
            all.extend(page.issue_types);
            if got == 0 || all.len() >= page.total {
                break;
            }
        }
        Ok(all)
    }

    /// Every field Jira would show on the "create" screen for this project + issue type.
    pub async fn create_fields(&self, project_key: &str, issue_type_id: &str) -> Result<Vec<FieldSpec>, AppError> {
        let path = format!(
            "/rest/api/3/issue/createmeta/{}/issuetypes/{}",
            segment(project_key)?,
            segment(issue_type_id)?
        );
        let mut specs: Vec<FieldSpec> = Vec::new();
        let mut seen = 0usize;
        for _ in 0..10 {
            let page: FieldPage = self.send(self.get(&path).query(&[("startAt", seen.to_string())])).await?;
            let got = page.fields.len();
            seen += got;
            specs.extend(page.fields.into_iter().filter_map(fields::spec_from_raw));
            if got == 0 || seen >= page.total {
                break;
            }
        }
        Ok(specs)
    }

    /// People who can be assigned issues in this project (for user-picker fields).
    pub async fn search_users(&self, project_key: &str, query: &str) -> Result<Vec<JiraUser>, AppError> {
        let users: Vec<RawUser> = self
            .send(self.get("/rest/api/3/user/assignable/search").query(&[
                ("project", segment(project_key)?),
                ("query", query),
                ("maxResults", "10"),
            ]))
            .await?;
        Ok(users
            .into_iter()
            .filter(|u| u.account_type.as_deref().map_or(true, |t| t == "atlassian"))
            .map(|u| JiraUser { account_id: u.account_id, display_name: u.display_name })
            .collect())
    }

    /// Create an issue; returns its key (e.g. "ABC-123").
    /// `extra` holds the already-converted values of the required custom fields.
    pub async fn create_issue(
        &self,
        project_key: &str,
        issue_type_id: &str,
        summary: &str,
        description: Option<Value>,
        extra: Map<String, Value>,
    ) -> Result<String, AppError> {
        let mut fields = extra;
        // Inserted last, so nothing in `extra` can override them.
        fields.insert("project".into(), json!({ "key": segment(project_key)? }));
        fields.insert("issuetype".into(), json!({ "id": segment(issue_type_id)? }));
        fields.insert("summary".into(), json!(summary));
        if let Some(d) = description {
            fields.insert("description".into(), d);
        }
        let created: CreatedIssue = self.send(self.post("/rest/api/3/issue").json(&json!({ "fields": fields }))).await?;
        Ok(created.key)
    }

    /// Log work on an issue; returns the new worklog id.
    pub async fn add_worklog(&self, issue_key: &str, started_at: i64, seconds: i64) -> Result<String, AppError> {
        let path = format!("/rest/api/3/issue/{}/worklog", segment(issue_key)?);
        let body = json!({ "started": jira_timestamp(started_at), "timeSpentSeconds": seconds });
        let created: CreatedWorklog = self.send(self.post(&path).json(&body)).await?;
        Ok(created.id)
    }
}

/// IDs and keys end up inside URL paths, so only plain characters are allowed (no `/`, `..`, `?`).
fn segment(s: &str) -> Result<&str, AppError> {
    let ok = !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(s)
    } else {
        Err(AppError::Invalid(format!("Not a valid Jira identifier: {s}")))
    }
}

/// Jira wants `2026-10-08T14:30:00.000+0000` (note: no colon in the offset).
pub fn jira_timestamp(unix_seconds: i64) -> String {
    chrono::DateTime::from_timestamp(unix_seconds, 0)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3f%z")
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JiraProject {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueType {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub subtask: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JiraUser {
    pub account_id: String,
    pub display_name: String,
}

#[derive(Deserialize)]
struct ProjectPage {
    #[serde(default)]
    values: Vec<JiraProject>,
    #[serde(rename = "isLast", default)]
    is_last: bool,
}

// `alias = "values"`: accept either spelling of the list, Jira has used both over the years.
#[derive(Deserialize)]
struct IssueTypePage {
    #[serde(rename = "issueTypes", alias = "values", default)]
    issue_types: Vec<IssueType>,
    #[serde(default)]
    total: usize,
}

#[derive(Deserialize)]
struct FieldPage {
    #[serde(alias = "values", default)]
    fields: Vec<fields::RawField>,
    #[serde(default)]
    total: usize,
}

#[derive(Deserialize)]
struct RawUser {
    #[serde(rename = "accountId")]
    account_id: String,
    #[serde(rename = "displayName", default)]
    display_name: String,
    #[serde(rename = "accountType")]
    account_type: Option<String>,
}

#[derive(Deserialize)]
struct CreatedIssue {
    key: String,
}

#[derive(Deserialize)]
struct CreatedWorklog {
    id: String,
}

// ---------------------------------------------------------------------------------------
// Saving Jira issues as local tasks
// ---------------------------------------------------------------------------------------

#[derive(Debug, Serialize, PartialEq, Clone)]
pub struct TypeCount {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct SyncReport {
    pub fetched: usize,
    pub added: usize,
    pub updated: usize,
    /// Issues Jira returned that you had removed on purpose, so they were left out.
    pub hidden_skipped: usize,
    /// What came back, by issue type (Bug, Story...): so you can SEE that defects were included.
    pub by_type: Vec<TypeCount>,
}

/// How many of each issue type, biggest group first.
fn count_by_type(issues: &[JiraIssue]) -> Vec<TypeCount> {
    let mut counts: Vec<TypeCount> = Vec::new();
    for issue in issues {
        let name = issue.issue_type.clone().unwrap_or_else(|| "Unknown type".into());
        match counts.iter_mut().find(|c| c.name == name) {
            Some(c) => c.count += 1,
            None => counts.push(TypeCount { name, count: 1 }),
        }
    }
    counts.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    counts
}

impl TaskStore {
    /// Insert issues we haven't seen, refresh title/status of the ones we have.
    /// Matching is by Jira key; purely local tasks are never touched.
    pub fn upsert_jira_issues(&self, issues: &[JiraIssue]) -> Result<SyncReport, AppError> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (mut added, mut updated, mut hidden_skipped) = (0, 0, 0);
        let hidden: std::collections::HashSet<String> = {
            let mut stmt = tx.prepare("SELECT jira_key FROM hidden_issues")?;
            let keys = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            keys
        };

        let title_of = |key: &str, summary: &str| match summary.trim() {
            "" => key.to_string(),
            s => s.to_string(),
        };
        let exists = |tx: &rusqlite::Transaction, key: &str| -> Result<Option<bool>, AppError> {
            // Some(is_context_only) when the key is known, None when it is new.
            Ok(tx
                .query_row("SELECT context_only FROM tasks WHERE jira_key = ?1", params![key], |r| r.get(0))
                .optional()?)
        };

        for issue in issues {
            // An issue you removed on purpose stays removed (restore it from "Hidden").
            if hidden.contains(&issue.key) {
                hidden_skipped += 1;
                continue;
            }
            // 1. The parent (e.g. an epic) may not be mine. Keep it as a heading-only row so the
            //    list can show "Epic -> stories". Jira sends its title and type with the issue.
            if let Some(parent) = &issue.parent {
                let title = title_of(&parent.key, &parent.summary);
                match exists(&tx, &parent.key)? {
                    None => {
                        tx.execute(
                            "INSERT INTO tasks (title, jira_key, issue_type, context_only, created_at)
                             VALUES (?1, ?2, ?3, 1, 0)",
                            params![title, parent.key, parent.issue_type],
                        )?;
                    }
                    Some(true) => {
                        tx.execute(
                            "UPDATE tasks SET title = ?2, issue_type = COALESCE(?3, issue_type) WHERE jira_key = ?1",
                            params![parent.key, title, parent.issue_type],
                        )?;
                    }
                    Some(false) => {} // it's one of my own tasks: its own fetch keeps it current
                }
            }

            // 2. The issue itself.
            let title = title_of(&issue.key, &issue.summary);
            let parent_key = issue.parent.as_ref().map(|p| p.key.as_str());
            if exists(&tx, &issue.key)?.is_some() {
                tx.execute(
                    // Finished in Jira => finished here too, but never the other way round: if you
                    // ticked it yourself, a refresh won't untick it.
                    "UPDATE tasks SET title = ?2, jira_status = ?3, issue_type = ?4, parent_key = ?5,
                            jira_logged_seconds = ?6, created_at = COALESCE(?7, created_at), context_only = 0,
                            done = CASE WHEN ?8 THEN 1 ELSE done END,
                            pending_parent_key = CASE WHEN pending_parent_key = ?5 THEN NULL ELSE pending_parent_key END
                     WHERE jira_key = ?1",
                    params![issue.key, title, issue.status, issue.issue_type, parent_key, issue.logged_seconds, issue.created_at, issue.resolved],
                )?;
                updated += 1;
            } else {
                tx.execute(
                    "INSERT INTO tasks (title, jira_key, jira_status, issue_type, parent_key, jira_logged_seconds, created_at, done)
                     VALUES (?2, ?1, ?3, ?4, ?5, ?6, COALESCE(?7, CAST(strftime('%s', 'now') AS INTEGER)), ?8)",
                    params![issue.key, title, issue.status, issue.issue_type, parent_key, issue.logged_seconds, issue.created_at, issue.resolved],
                )?;
                added += 1;
            }
        }
        tx.commit()?;
        Ok(SyncReport { fetched: issues.len(), added, updated, hidden_skipped, by_type: count_by_type(issues) })
    }
}

impl TaskStore {
    /// Issues you removed on purpose, newest first.
    pub fn hidden_issues(&self) -> Result<Vec<HiddenIssue>, AppError> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare("SELECT jira_key, title, hidden_at FROM hidden_issues ORDER BY hidden_at DESC, jira_key")?;
        let list = stmt
            .query_map([], |r| Ok(HiddenIssue { jira_key: r.get(0)?, title: r.get(1)?, hidden_at: r.get(2)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(list)
    }

    pub fn unhide_issue(&self, jira_key: &str) -> Result<(), AppError> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM hidden_issues WHERE jira_key = ?1", params![jira_key])?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------------------

/// What the Settings screen is allowed to know. Note: no token field, only "is there one".
#[derive(Debug, Serialize, PartialEq)]
pub struct JiraSettings {
    pub base_url: String,
    pub email: String,
    pub has_token: bool,
    pub token_storage: Option<String>,
    /// Also fetch issues resolved in the last N days (0 = only open ones).
    pub fetch_days: u32,
}

fn fetch_days(store: &TaskStore) -> u32 {
    store
        .get_setting(KEY_FETCH_DAYS)
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .filter(|d| FETCH_DAYS_CHOICES.contains(d))
        .unwrap_or(DEFAULT_FETCH_DAYS)
}

fn load_settings(store: &TaskStore, secrets: &SecretStore) -> Result<JiraSettings, AppError> {
    let token = secrets.get(TOKEN_NAME)?;
    Ok(JiraSettings {
        base_url: store.get_setting(KEY_URL)?.unwrap_or_default(),
        email: store.get_setting(KEY_EMAIL)?.unwrap_or_default(),
        has_token: token.is_some(),
        token_storage: token.map(|(_, backend)| backend.label().to_string()),
        fetch_days: fetch_days(store),
    })
}

fn save_fetch_days(store: &TaskStore, days: u32) -> Result<(), AppError> {
    if !FETCH_DAYS_CHOICES.contains(&days) {
        return Err(AppError::Invalid(format!("Choose one of {FETCH_DAYS_CHOICES:?} days")));
    }
    store.set_setting(KEY_FETCH_DAYS, &days.to_string())
}

fn save_settings(
    store: &TaskStore,
    secrets: &SecretStore,
    base_url: &str,
    email: &str,
    token: Option<&str>,
) -> Result<JiraSettings, AppError> {
    let base_url = normalize_base_url(base_url)?;
    let email = email.trim();
    if !email.contains('@') {
        return Err(AppError::Config("Enter the email address of your Atlassian account".into()));
    }

    let url_changed = store.get_setting(KEY_URL)?.is_some_and(|old| old != base_url);
    let new_token = token.map(str::trim).filter(|t| !t.is_empty());

    if url_changed && new_token.is_none() {
        secrets.delete(TOKEN_NAME)?; // never send an old token to a different site
    }
    if let Some(t) = new_token {
        secrets.set(TOKEN_NAME, t)?;
    }
    store.set_setting(KEY_URL, &base_url)?;
    store.set_setting(KEY_EMAIL, email)?;
    load_settings(store, secrets)
}

pub(crate) fn load_client(store: &TaskStore, secrets: &SecretStore) -> Result<JiraClient, AppError> {
    let url = store
        .get_setting(KEY_URL)?
        .ok_or_else(|| AppError::Config("Add your Jira URL in Settings first".into()))?;
    let email = store
        .get_setting(KEY_EMAIL)?
        .ok_or_else(|| AppError::Config("Add your Jira email in Settings first".into()))?;
    let (token, _) = secrets
        .get(TOKEN_NAME)?
        .ok_or_else(|| AppError::Config("Add your Jira API token in Settings first".into()))?;
    JiraClient::new(&url, &email, &token)
}

// ---------------------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------------------

#[tauri::command]
pub fn get_jira_settings(
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<JiraSettings, AppError> {
    load_settings(&store, &secrets)
}

/// `token: None` or empty means "keep the token I already saved".
#[tauri::command]
pub fn save_jira_settings(
    base_url: String,
    email: String,
    token: Option<String>,
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<JiraSettings, AppError> {
    save_settings(&store, &secrets, &base_url, &email, token.as_deref())
}

#[tauri::command]
pub fn clear_jira_token(
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<JiraSettings, AppError> {
    secrets.delete(TOKEN_NAME)?;
    load_settings(&store, &secrets)
}

/// Returns your Jira display name when everything works.
#[tauri::command]
pub async fn test_jira_connection(
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<String, AppError> {
    // Build the client first: the (non-Send) database lock is released before we `.await`.
    let client = load_client(&store, &secrets)?;
    client.myself().await
}

#[tauri::command]
pub async fn fetch_jira_issues(
    store: State<'_, TaskStore>,
    secrets: State<'_, SecretStore>,
) -> Result<SyncReport, AppError> {
    let client = load_client(&store, &secrets)?;
    let issues = client.my_issues(fetch_days(&store)).await?;
    store.upsert_jira_issues(&issues)
}

/// "Also fetch what I resolved in the last N days" (0 = only open issues).
#[tauri::command]
pub fn set_fetch_days(days: u32, store: State<'_, TaskStore>, secrets: State<'_, SecretStore>) -> Result<JiraSettings, AppError> {
    save_fetch_days(&store, days)?;
    load_settings(&store, &secrets)
}

#[derive(Debug, Serialize)]
pub struct HiddenIssue {
    pub jira_key: String,
    pub title: String,
    pub hidden_at: i64,
}

#[tauri::command]
pub fn list_hidden_issues(store: State<'_, TaskStore>) -> Result<Vec<HiddenIssue>, AppError> {
    store.hidden_issues()
}

/// Stop hiding an issue. It comes back on the next Refresh from Jira.
#[tauri::command]
pub fn unhide_issue(jira_key: String, store: State<'_, TaskStore>) -> Result<(), AppError> {
    store.unhide_issue(&jira_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> JiraClient {
        JiraClient::new(&server.uri(), "me@example.com", "tok").unwrap()
    }

    fn issue_json(key: &str, summary: &str) -> serde_json::Value {
        json!({ "id": "1", "key": key, "fields": { "summary": summary, "status": { "name": "In Progress" } } })
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pj-jira-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    // --- URL handling ---

    #[test]
    fn urls_are_normalized_to_scheme_and_host() {
        let ok = |s: &str| normalize_base_url(s).unwrap();
        assert_eq!(ok("https://acme.atlassian.net"), "https://acme.atlassian.net");
        assert_eq!(ok("  acme.atlassian.net/ "), "https://acme.atlassian.net");
        assert_eq!(ok("https://acme.atlassian.net/jira/software/projects/ABC"), "https://acme.atlassian.net");
        assert_eq!(ok("http://localhost:8080/x"), "http://localhost:8080");
    }

    #[test]
    fn insecure_or_empty_urls_are_rejected() {
        assert!(normalize_base_url("").is_err());
        assert!(normalize_base_url("http://acme.atlassian.net").is_err());
        assert!(normalize_base_url("ftp://acme.atlassian.net").is_err());
    }

    // --- HTTP client against a fake Jira ---

    #[tokio::test]
    async fn myself_returns_display_name_and_sends_basic_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/myself"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "accountId": "a1", "displayName": "Ada Lovelace" })))
            .mount(&server)
            .await;

        assert_eq!(client(&server).myself().await.unwrap(), "Ada Lovelace");

        let requests = server.received_requests().await.unwrap();
        let auth = requests[0].headers.get("authorization").unwrap().to_str().unwrap();
        assert!(auth.starts_with("Basic "));
    }

    #[tokio::test]
    async fn bad_credentials_give_an_actionable_message() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let secret_client = JiraClient::new(&server.uri(), "me@example.com", "s3cr3t-value").unwrap();
        let err = secret_client.myself().await.unwrap_err().to_string();
        assert!(err.contains("email or API token"), "got: {err}");
        assert!(!err.contains("s3cr3t-value"), "message must never echo the token");
    }

    #[tokio::test]
    async fn redirects_are_not_followed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/myself"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/elsewhere"))
            .mount(&server)
            .await;
        let err = client(&server).myself().await.unwrap_err().to_string();
        assert!(err.contains("redirect"), "got: {err}");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn issues_are_collected_across_pages() {
        let server = MockServer::start().await;
        // Registered first, so it wins when the request carries the page token.
        Mock::given(method("GET"))
            .and(path("/rest/api/3/search/jql"))
            .and(query_param("nextPageToken", "page2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issues": [issue_json("ABC-3", "Third")], "isLast": true
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/search/jql"))
            .and(query_param("fields", ISSUE_FIELDS))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issues": [issue_json("ABC-1", "First"), issue_json("ABC-2", "Second")],
                "nextPageToken": "page2", "isLast": false
            })))
            .mount(&server)
            .await;

        let issues = client(&server).my_issues(0).await.unwrap();
        let keys: Vec<_> = issues.iter().map(|i| i.key.as_str()).collect();
        assert_eq!(keys, ["ABC-1", "ABC-2", "ABC-3"]);
        assert_eq!(issues[0].status.as_deref(), Some("In Progress"));
    }

    #[tokio::test]
    async fn reported_issues_are_searched_by_reporter_not_assignee() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/search/jql"))
            .and(query_param("jql", REPORTED_JQL))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "issues": [issue_json("ABC-9", "Mine")], "isLast": true })))
            .mount(&server)
            .await;
        let issues = client(&server).my_reported().await.unwrap();
        assert_eq!(issues.len(), 1);
        assert!(REPORTED_JQL.contains("reporter = currentUser()") && !REPORTED_JQL.contains("assignee"));
    }

    #[tokio::test]
    async fn done_issues_come_with_the_date_they_were_finished() {
        let server = MockServer::start().await;
        let with = |key: &str, resolved: Option<&str>, changed: Option<&str>| {
            json!({ "key": key, "fields": { "summary": key, "status": { "name": "Done" }, "issuetype": { "name": "Story" },
                "resolutiondate": resolved, "statuscategorychangedate": changed } })
        };
        Mock::given(method("GET"))
            .and(path("/rest/api/3/search/jql"))
            .and(query_param("jql", DONE_JQL))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "issues": [
                with("ABC-1", Some("2026-10-02T09:00:00.000+0000"), Some("2026-10-05T09:00:00.000+0000")),
                with("ABC-2", None, Some("2026-09-10T09:00:00.000+0000")),
                with("ABC-3", None, None),
            ], "isLast": true })))
            .mount(&server)
            .await;

        let issues = client(&server).my_done().await.unwrap();
        // The resolution date wins; without one, the date the status moved to Done; without either, none.
        assert_eq!(issues[0].done_at, Some(1_790_931_600));
        assert_eq!(issues[1].done_at, Some(1_789_030_800));
        assert_eq!(issues[2].done_at, None);
        assert!(DONE_JQL.contains("assignee = currentUser()") && DONE_JQL.contains("statusCategory = Done"));
    }

    #[tokio::test]
    async fn issue_detail_returns_description_and_comments_as_plain_text() {
        let server = MockServer::start().await;
        let adf = |text: &str| json!({ "type": "doc", "version": 1, "content": [{ "type": "paragraph", "content": [{ "type": "text", "text": text }] }] });
        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/ABC-5"))
            .and(query_param("fields", "description,comment"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "key": "ABC-5", "fields": {
                "description": adf("Totals are wrong after a refund."),
                "comment": { "comments": [
                    { "author": { "displayName": "Grace" }, "body": adf("Reproduced on staging.") },
                    { "body": adf("Root cause: rounding.") },
                    { "author": { "displayName": "Bot" }, "body": { "type": "doc", "version": 1, "content": [] } }
                ] }
            } })))
            .mount(&server)
            .await;

        let detail = client(&server).issue_detail("ABC-5").await.unwrap();
        assert_eq!(detail.description, "Totals are wrong after a refund.");
        assert_eq!(detail.comments.len(), 2, "an empty comment is dropped");
        assert_eq!((detail.comments[0].author.as_str(), detail.comments[0].text.as_str()), ("Grace", "Reproduced on staging."));
        assert_eq!(detail.comments[1].author, "Someone");
    }

    #[tokio::test]
    async fn issue_detail_copes_with_no_description_and_no_comments() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/ABC-6"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "key": "ABC-6", "fields": { "description": null } })))
            .mount(&server)
            .await;
        assert_eq!(client(&server).issue_detail("ABC-6").await.unwrap(), IssueDetail::default());
    }

    // --- Saving into the local database ---

    #[test]
    fn upsert_adds_then_updates_and_leaves_local_tasks_alone() {
        let store = TaskStore::in_memory();
        let local = store.add("my own task").unwrap();
        let first = vec![JiraIssue { key: "ABC-1".into(), summary: "Fix login".into(), status: Some("To Do".into()), ..Default::default() }];

        let r = store.upsert_jira_issues(&first).unwrap();
        assert_eq!((r.fetched, r.added, r.updated), (1, 1, 0));

        let renamed = vec![JiraIssue { key: "ABC-1".into(), summary: "Fix login redirect".into(), status: Some("In Progress".into()), ..Default::default() }];
        let r = store.upsert_jira_issues(&renamed).unwrap();
        assert_eq!((r.fetched, r.added, r.updated), (1, 0, 1));

        let tasks = store.list().unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].id, local.id);
        assert_eq!(tasks[0].jira_key, None);
        assert_eq!(tasks[1].title, "Fix login redirect");
        assert_eq!(tasks[1].jira_status.as_deref(), Some("In Progress"));
    }

    fn story(key: &str, parent: Option<(&str, &str)>) -> JiraIssue {
        JiraIssue {
            key: key.into(),
            summary: format!("Story {key}"),
            issue_type: Some("Story".into()),
            created_at: Some(1_790_000_000),
            logged_seconds: Some(5_400),
            parent: parent.map(|(k, s)| JiraParent { key: k.into(), summary: s.into(), issue_type: Some("Epic".into()) }),
            ..Default::default()
        }
    }

    #[test]
    fn parents_become_heading_rows_and_stories_remember_their_details() {
        let store = TaskStore::in_memory();
        store.upsert_jira_issues(&[story("ABC-2", Some(("ABC-1", "Login revamp")))]).unwrap();

        let tasks = store.list().unwrap();
        assert_eq!(tasks.len(), 2);
        let epic = tasks.iter().find(|t| t.jira_key.as_deref() == Some("ABC-1")).unwrap();
        assert!(epic.context_only);
        assert_eq!((epic.title.as_str(), epic.issue_type.as_deref()), ("Login revamp", Some("Epic")));
        let s = tasks.iter().find(|t| t.jira_key.as_deref() == Some("ABC-2")).unwrap();
        assert!(!s.context_only);
        assert_eq!(s.parent_key.as_deref(), Some("ABC-1"));
        assert_eq!(s.created_at, 1_790_000_000);
        assert_eq!(s.jira_logged_seconds, Some(5_400));
    }

    #[test]
    fn refresh_updates_logged_time_and_clears_pending_parent() {
        let store = TaskStore::in_memory();
        store.upsert_jira_issues(&[story("ABC-2", None)]).unwrap();
        let id = store.list().unwrap()[0].id;
        store.set_pending_parent(id, Some("ABC-1")).unwrap(); // staged locally, not in Jira yet
        assert_eq!(store.find(id).unwrap().pending_parent_key.as_deref(), Some("ABC-1"));

        // Someone (or a push) set the parent in Jira, and 30 more minutes were logged:
        let mut refreshed = story("ABC-2", Some(("ABC-1", "Login revamp")));
        refreshed.logged_seconds = Some(7_200);
        store.upsert_jira_issues(&[refreshed]).unwrap();

        let t = store.find(id).unwrap();
        assert_eq!(t.pending_parent_key, None); // staged change is now in Jira, nothing left to push
        assert_eq!(t.parent_key.as_deref(), Some("ABC-1"));
        assert_eq!(t.jira_logged_seconds, Some(7_200));
    }

    #[test]
    fn a_refresh_never_overwrites_a_different_staged_parent() {
        let store = TaskStore::in_memory();
        store.upsert_jira_issues(&[story("ABC-2", Some(("ABC-1", "Old epic")))]).unwrap();
        let id = store.list().unwrap().into_iter().find(|t| !t.context_only).unwrap().id;
        store.set_pending_parent(id, Some("ABC-9")).unwrap();
        store.upsert_jira_issues(&[story("ABC-2", Some(("ABC-1", "Old epic")))]).unwrap();
        assert_eq!(store.find(id).unwrap().pending_parent_key.as_deref(), Some("ABC-9"));
    }

    #[tokio::test]
    async fn search_results_carry_type_parent_created_and_logged_time() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/search/jql"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "isLast": true,
                "issues": [{
                    "key": "ABC-2",
                    "fields": {
                        "summary": "Fix login", "status": { "name": "To Do" },
                        "issuetype": { "name": "Story" },
                        "parent": { "key": "ABC-1", "fields": { "summary": "Login revamp", "issuetype": { "name": "Epic" } } },
                        "created": "2026-09-03T10:15:30.123+0530",
                        "timespent": 5400
                    }
                }]
            })))
            .mount(&server)
            .await;

        let issue = client(&server).my_issues(0).await.unwrap().remove(0);
        assert_eq!(issue.issue_type.as_deref(), Some("Story"));
        assert_eq!(issue.logged_seconds, Some(5400));
        assert_eq!(issue.created_at, Some(1_788_410_730)); // 2026-09-03T04:45:30Z
        assert_eq!(
            issue.parent,
            Some(JiraParent { key: "ABC-1".into(), summary: "Login revamp".into(), issue_type: Some("Epic".into()) })
        );
    }

    #[tokio::test]
    async fn setting_a_parent_sends_a_put_with_the_parent_key() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/rest/api/3/issue/ABC-2"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&server)
            .await;
        client(&server).set_parent("ABC-2", "ABC-1").await.unwrap();
        let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(body, json!({ "fields": { "parent": { "key": "ABC-1" } } }));
    }

    #[tokio::test]
    async fn me_returns_account_id_and_name() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/myself"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "accountId": "abc123", "displayName": "Ada" })))
            .mount(&server)
            .await;
        let me = client(&server).me().await.unwrap();
        assert_eq!((me.account_id.as_str(), me.display_name.as_str()), ("abc123", "Ada"));
    }

    // --- which issues are fetched, and removing them ---

    #[test]
    fn search_includes_recently_resolved_work() {
        assert_eq!(build_jql(0), "assignee = currentUser() AND resolution = Unresolved ORDER BY updated DESC");
        let jql = build_jql(30);
        assert!(jql.contains("resolution = Unresolved OR resolved >= -30d"), "{jql}");
        assert!(jql.starts_with("assignee = currentUser()") && jql.ends_with("ORDER BY updated DESC"));
    }

    fn bug(key: &str, resolved: bool) -> JiraIssue {
        JiraIssue { key: key.into(), summary: format!("Defect {key}"), issue_type: Some("Bug".into()), resolved, ..Default::default() }
    }

    #[tokio::test]
    async fn defects_assigned_to_me_are_asked_for_and_counted_by_type() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/search/jql"))
            .and(query_param("jql", build_jql(30).as_str())) // the request really contains the "resolved recently" clause
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "isLast": true,
                "issues": [
                    { "key": "ABC-1", "fields": { "summary": "Crash on save", "issuetype": { "name": "Bug" }, "resolutiondate": "2026-10-10T09:00:00.000+0000" } },
                    { "key": "ABC-2", "fields": { "summary": "Slow search", "issuetype": { "name": "Bug" } } },
                    { "key": "ABC-3", "fields": { "summary": "Add export", "issuetype": { "name": "Story" } } }
                ]
            })))
            .mount(&server)
            .await;

        let issues = client(&server).my_issues(30).await.unwrap();
        assert_eq!(issues.iter().map(|i| i.resolved).collect::<Vec<_>>(), [true, false, false]);

        let store = TaskStore::in_memory();
        let report = store.upsert_jira_issues(&issues).unwrap();
        assert_eq!(report.by_type, vec![TypeCount { name: "Bug".into(), count: 2 }, TypeCount { name: "Story".into(), count: 1 }]);
        let tasks = store.list().unwrap();
        let done: Vec<_> = tasks.iter().map(|t| (t.jira_key.as_deref().unwrap(), t.done)).collect();
        assert_eq!(done, [("ABC-1", true), ("ABC-2", false), ("ABC-3", false)]); // the resolved defect arrives as done
    }

    #[test]
    fn refresh_marks_resolved_done_never_unticks() {
        let store = TaskStore::in_memory();
        store.upsert_jira_issues(&[bug("ABC-1", false)]).unwrap();
        let id = store.list().unwrap()[0].id;
        assert!(!store.find(id).unwrap().done);

        store.upsert_jira_issues(&[bug("ABC-1", true)]).unwrap(); // resolved in Jira
        assert!(store.find(id).unwrap().done);

        store.upsert_jira_issues(&[bug("ABC-1", false)]).unwrap(); // reopened: your tick stays
        assert!(store.find(id).unwrap().done);
    }

    #[test]
    fn removed_issue_stays_removed_until_restored() {
        let store = TaskStore::in_memory();
        store.upsert_jira_issues(&[bug("ABC-1", false), bug("ABC-2", false)]).unwrap();
        let first = store.list().unwrap().into_iter().find(|t| t.jira_key.as_deref() == Some("ABC-1")).unwrap();
        store.remove(first.id).unwrap();

        let hidden = store.hidden_issues().unwrap();
        assert_eq!((hidden.len(), hidden[0].jira_key.as_str(), hidden[0].title.as_str()), (1, "ABC-1", "Defect ABC-1"));

        // Jira still returns both: only the one you kept comes back in.
        let report = store.upsert_jira_issues(&[bug("ABC-1", false), bug("ABC-2", false)]).unwrap();
        assert_eq!((report.hidden_skipped, report.added, report.updated), (1, 0, 1));
        assert_eq!(store.list().unwrap().len(), 1);

        // Restore it: the next refresh brings it back.
        store.unhide_issue("ABC-1").unwrap();
        assert!(store.hidden_issues().unwrap().is_empty());
        assert_eq!(store.upsert_jira_issues(&[bug("ABC-1", false), bug("ABC-2", false)]).unwrap().added, 1);
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn removing_a_plain_local_task_remembers_nothing() {
        let store = TaskStore::in_memory();
        let t = store.add("just mine").unwrap();
        store.remove(t.id).unwrap();
        assert!(store.hidden_issues().unwrap().is_empty());
    }

    #[test]
    fn fetch_window_uses_known_choices() {
        let store = TaskStore::in_memory();
        assert_eq!(fetch_days(&store), 30);
        save_fetch_days(&store, 0).unwrap();
        assert_eq!(fetch_days(&store), 0);
        save_fetch_days(&store, 90).unwrap();
        assert_eq!(fetch_days(&store), 90);
        assert!(save_fetch_days(&store, 7).is_err());
        assert_eq!(fetch_days(&store), 90);
    }

    // --- Settings and the token ---

    #[test]
    fn settings_never_expose_the_token_and_url_change_forgets_it() {
        let store = TaskStore::in_memory();
        let dir = temp_dir("settings");
        let secrets = SecretStore::file_only(dir.clone());

        let saved = save_settings(&store, &secrets, "acme.atlassian.net", "me@example.com", Some("tok-123")).unwrap();
        assert_eq!(saved.base_url, "https://acme.atlassian.net");
        assert!(saved.has_token);
        assert!(!format!("{saved:?}").contains("tok-123"));

        // Saving again without a token keeps the one we have.
        let again = save_settings(&store, &secrets, "https://acme.atlassian.net", "me@example.com", None).unwrap();
        assert!(again.has_token);

        // Pointing at another site without a new token drops the old one.
        let moved = save_settings(&store, &secrets, "https://evil.example.com", "me@example.com", None).unwrap();
        assert!(!moved.has_token);
        assert!(secrets.get(TOKEN_NAME).unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saving_requires_a_plausible_email() {
        let store = TaskStore::in_memory();
        let secrets = SecretStore::file_only(temp_dir("email"));
        assert!(save_settings(&store, &secrets, "acme.atlassian.net", "not-an-email", Some("t")).is_err());
    }

    // --- creating issues and logging work ---

    #[test]
    fn timestamps_use_jiras_format() {
        assert_eq!(jira_timestamp(1_700_000_000), "2023-11-14T22:13:20.000+0000");
    }

    #[test]
    fn path_segments_reject_anything_that_could_change_the_url() {
        assert!(segment("ABC-123").is_ok() && segment("10001").is_ok() && segment("MY_KEY").is_ok());
        for bad in ["", "../x", "A/B", "A?b=1", "a b", "A#"] {
            assert!(segment(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[tokio::test]
    async fn projects_and_issue_types_are_listed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/project/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "isLast": true, "values": [{ "key": "ABC", "name": "Alpha", "id": "1" }]
            })))
            .mount(&server)
            .await;
        // The list has been called "issueTypes" and "values" by different Jira versions.
        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/createmeta/ABC/issuetypes"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "total": 2, "values": [
                    { "id": "10001", "name": "Story", "subtask": false },
                    { "id": "10002", "name": "Sub-task", "subtask": true }
                ]
            })))
            .mount(&server)
            .await;

        let c = client(&server);
        assert_eq!(c.projects().await.unwrap(), vec![JiraProject { key: "ABC".into(), name: "Alpha".into() }]);
        let types = c.issue_types("ABC").await.unwrap();
        assert_eq!(types.len(), 2);
        assert!(types[1].subtask);
        assert!(c.issue_types("../etc").await.is_err()); // rejected before any request is sent
    }

    #[tokio::test]
    async fn field_metadata_is_read_across_pages() {
        let server = MockServer::start().await;
        let field = |id: &str| json!({ "required": true, "name": id, "fieldId": id, "schema": { "type": "string" } });
        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/createmeta/ABC/issuetypes/10001"))
            .and(query_param("startAt", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "total": 3, "fields": [field("c")] })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/rest/api/3/issue/createmeta/ABC/issuetypes/10001"))
            .and(query_param("startAt", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "total": 3, "fields": [field("a"), field("b")] })))
            .mount(&server)
            .await;

        let specs = client(&server).create_fields("ABC", "10001").await.unwrap();
        let ids: Vec<_> = specs.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "c"]);
    }

    #[tokio::test]
    async fn create_issue_sends_fixed_fields_that_extras_cannot_override() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/rest/api/3/issue"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": "1", "key": "ABC-9" })))
            .mount(&server)
            .await;

        let mut extra = Map::new();
        extra.insert("customfield_1".into(), json!("x"));
        extra.insert("project".into(), json!({ "key": "EVIL" })); // must be overwritten
        let key = client(&server)
            .create_issue("ABC", "10001", "Title", None, extra)
            .await
            .unwrap();
        assert_eq!(key, "ABC-9");

        let body: Value = serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
        assert_eq!(body["fields"]["project"]["key"], "ABC");
        assert_eq!(body["fields"]["customfield_1"], "x");
        assert!(body["fields"].get("description").is_none());
    }

    #[tokio::test]
    async fn jira_validation_errors_are_shown_to_the_user() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/rest/api/3/issue"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "errorMessages": [], "errors": { "customfield_10050": "Team is required." }
            })))
            .mount(&server)
            .await;
        let err = client(&server)
            .create_issue("ABC", "10001", "T", None, Map::new())
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Team is required."), "got: {err}");
    }
}
