//! Read-only GitLab client (`read_api` token): merge requests you authored or are assigned to.
//! One client per server. It reads title, description, branch, state, link and commit subjects, never diffs.

use crate::error::AppError;
use crate::git::GitChange;
use crate::jira::normalize_base_url;
use reqwest::{redirect, Client, RequestBuilder};
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration;

pub const DEFAULT_URL: &str = "https://gitlab.com";

const PER_PAGE: &str = "100";
const MAX_PAGES: usize = 5; // at most 500 MRs per scope
const MAX_COMMITS: usize = 30;
const MAX_NOTES: usize = 2500;

/// What GitLab calls an MR, plus the two numbers needed to ask for its commits.
#[derive(Debug, Clone, PartialEq)]
pub struct GitlabMr {
    pub project_id: i64,
    pub iid: i64,
    pub change: GitChange,
}

#[derive(Deserialize)]
struct RawUser {
    #[serde(default)]
    username: String,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct RawRefs {
    #[serde(default)]
    full: String,
}

#[derive(Deserialize)]
struct RawMr {
    iid: i64,
    project_id: i64,
    #[serde(default)]
    title: String,
    description: Option<String>,
    #[serde(default)]
    state: String,
    #[serde(default)]
    web_url: String,
    #[serde(default)]
    source_branch: String,
    created_at: String,
    merged_at: Option<String>,
    references: Option<RawRefs>,
}

#[derive(Deserialize)]
struct RawCommit {
    #[serde(default)]
    title: String,
}

fn parse_time(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp())
}

impl RawMr {
    fn into_mr(self, role: &str) -> GitlabMr {
        GitlabMr {
            project_id: self.project_id,
            iid: self.iid,
            change: GitChange {
                url: self.web_url,
                reference: self.references.map(|r| r.full).filter(|r| !r.is_empty()).unwrap_or_else(|| format!("!{}", self.iid)),
                title: self.title,
                description: self.description.unwrap_or_default(),
                state: self.state,
                source_branch: self.source_branch,
                created_at: parse_time(&self.created_at).unwrap_or(0),
                merged_at: self.merged_at.as_deref().and_then(parse_time),
                role: role.to_string(),
                commit_notes: None,
            },
        }
    }
}

pub struct GitlabClient {
    http: Client,
    base: String,
    token: String,
}

impl GitlabClient {
    pub fn new(base_url: &str, token: &str) -> Result<Self, AppError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(redirect::Policy::none()) // never forward the token to another host
            .user_agent("ProjectJira/0.1")
            .build()?;
        Ok(Self { http, base: normalize_base_url(base_url)?, token: token.trim().to_string() })
    }

    fn get(&self, path: &str) -> RequestBuilder {
        self.http
            .get(format!("{}/api/v4{path}", self.base))
            .header("PRIVATE-TOKEN", &self.token)
            .header("Accept", "application/json")
    }

    async fn execute(&self, req: RequestBuilder) -> Result<reqwest::Response, AppError> {
        let resp = req.send().await?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let server = &self.base;
        Err(AppError::Gitlab(match status.as_u16() {
            401 => format!(
                "{server} rejected the access token. Create a new one (User settings > Access tokens, scope read_api) \
                 and save it in Settings."
            ),
            403 => format!("{server} accepted the token but denied access (403). The token needs the read_api scope."),
            404 => format!("{server} answered 404 (not found). Check the GitLab address in Settings."),
            429 => format!("{server} is rate-limiting requests (429). Wait a minute and try again."),
            300..=399 => format!("{server} tried to redirect the request. Use the exact address of your GitLab."),
            code => format!("{server} returned HTTP {code}."),
        }))
    }

    /// Who does this token belong to? Also the cheapest "does everything work" check.
    pub async fn current_user(&self) -> Result<String, AppError> {
        let user: RawUser = self.execute(self.get("/user")).await?.json().await?;
        Ok(if user.name.is_empty() { user.username } else { format!("{} (@{})", user.name, user.username) })
    }

    /// MRs for one `scope` ("created_by_me" or "assigned_to_me"), created since `since` (Unix s).
    pub async fn merge_requests(&self, scope: &str, since: i64, state: &str, role: &str) -> Result<Vec<GitlabMr>, AppError> {
        let created_after = chrono::DateTime::from_timestamp(since, 0).unwrap_or_default().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let mut all = Vec::new();
        let mut page = 1usize;
        for _ in 0..MAX_PAGES {
            let resp = self
                .execute(self.get("/merge_requests").query(&[
                    ("scope", scope),
                    ("state", state),
                    ("created_after", created_after.as_str()),
                    ("order_by", "created_at"),
                    ("per_page", PER_PAGE),
                    ("page", page.to_string().as_str()),
                ]))
                .await?;
            // GitLab tells us the next page in a header; it is empty on the last page.
            let next = resp
                .headers()
                .get("x-next-page")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<usize>().ok());
            let batch: Vec<RawMr> = resp.json().await?;
            all.extend(batch.into_iter().map(|raw| raw.into_mr(role)));
            match next {
                Some(n) => page = n,
                None => break,
            }
        }
        Ok(all)
    }

    /// The first line of each commit message in an MR, as "- subject" lines.
    pub async fn commit_subjects(&self, project_id: i64, iid: i64) -> Result<String, AppError> {
        let commits: Vec<RawCommit> = self
            .execute(self.get(&format!("/projects/{project_id}/merge_requests/{iid}/commits")).query(&[("per_page", "50")]))
            .await?
            .json()
            .await?;
        let lines: Vec<String> = commits
            .into_iter()
            .map(|c| c.title.trim().to_string())
            .filter(|t| !t.is_empty() && !t.starts_with("Merge branch"))
            .take(MAX_COMMITS)
            .map(|t| format!("- {t}"))
            .collect();
        Ok(lines.join("\n"))
    }
}

/// Everything an import needs from one GitLab server. Commits are only fetched for MRs whose
/// address isn't in `known_urls` (already stored).
pub async fn fetch_changes(
    client: &GitlabClient,
    since: i64,
    include_assigned: bool,
    state: &str,
    known_urls: &HashSet<String>,
) -> Result<Vec<GitChange>, AppError> {
    let mut ordered: Vec<GitlabMr> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Authored first: if an MR is both authored and assigned to you, it counts as authored.
    let mut scopes = vec![("created_by_me", "author")];
    if include_assigned {
        scopes.push(("assigned_to_me", "assignee"));
    }
    for (scope, role) in scopes {
        for mr in client.merge_requests(scope, since, state, role).await? {
            if seen.insert(mr.change.url.clone()) {
                ordered.push(mr);
            }
        }
    }

    for mr in ordered.iter_mut().filter(|m| !known_urls.contains(&m.change.url)) {
        // A commit list we can't read isn't worth failing the whole import for.
        let notes = client.commit_subjects(mr.project_id, mr.iid).await.unwrap_or_default();
        mr.change.commit_notes = Some(notes.chars().take(MAX_NOTES).collect());
    }
    Ok(ordered.into_iter().map(|m| m.change).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn raw_mr(id: i64, iid: i64, title: &str, branch: &str) -> Value {
        json!({
            "id": id, "iid": iid, "project_id": 7, "title": title, "description": "Does the thing.",
            "state": "merged", "web_url": format!("https://gitlab.example/g/p/-/merge_requests/{iid}"),
            "source_branch": branch, "created_at": "2026-10-03T10:15:30.123Z", "merged_at": "2026-10-04T08:00:00.000Z",
            "references": { "full": format!("g/p!{iid}") }
        })
    }

    fn client(server: &MockServer) -> GitlabClient {
        GitlabClient::new(&server.uri(), "glpat-secret").unwrap()
    }

    #[tokio::test]
    async fn current_user_sends_the_private_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .and(header("PRIVATE-TOKEN", "glpat-secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "username": "ada", "name": "Ada L" })))
            .mount(&server)
            .await;
        assert_eq!(client(&server).current_user().await.unwrap(), "Ada L (@ada)");
    }

    #[tokio::test]
    async fn rejected_token_message_never_echoes_the_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
        let err = client(&server).current_user().await.unwrap_err().to_string();
        assert!(err.contains("read_api") && err.contains(&server.uri()), "got: {err}");
        assert!(!err.contains("glpat-secret"));
    }

    #[tokio::test]
    async fn merge_requests_are_read_page_by_page_and_parsed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/merge_requests"))
            .and(query_param("page", "2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([raw_mr(3, 3, "Third", "b3")])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/merge_requests"))
            .and(query_param("scope", "created_by_me"))
            .and(query_param("state", "merged"))
            .and(query_param("created_after", "2026-10-01T00:00:00Z"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("X-Next-Page", "2")
                    .set_body_json(json!([raw_mr(1, 1, "First", "b1"), raw_mr(2, 2, "Second", "b2")])),
            )
            .mount(&server)
            .await;

        let since = chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z").unwrap().timestamp();
        let mrs = client(&server).merge_requests("created_by_me", since, "merged", "author").await.unwrap();
        assert_eq!(mrs.iter().map(|m| m.change.title.as_str()).collect::<Vec<_>>(), ["First", "Second", "Third"]);
        let first = &mrs[0].change;
        assert_eq!(first.reference, "g/p!1");
        assert_eq!(first.url, "https://gitlab.example/g/p/-/merge_requests/1");
        assert_eq!(first.created_at, 1_791_022_530); // 2026-10-03T10:15:30Z
        assert_eq!(first.merged_at, Some(1_791_100_800));
        assert_eq!(first.role, "author");
        assert_eq!((mrs[0].project_id, mrs[0].iid), (7, 1));
    }

    #[tokio::test]
    async fn commit_subjects_skip_merge_commits() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/merge_requests/3/commits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                { "title": "Add Apple Pay button" }, { "title": "Merge branch 'main' into feature" }, { "title": "Handle declined cards" }
            ])))
            .mount(&server)
            .await;
        assert_eq!(client(&server).commit_subjects(7, 3).await.unwrap(), "- Add Apple Pay button\n- Handle declined cards");
    }

    #[tokio::test]
    async fn authored_and_assigned_merge_without_duplicates() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("scope", "created_by_me"))
            .and(path("/api/v4/merge_requests"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([raw_mr(1, 1, "Mine", "b1")])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("scope", "assigned_to_me"))
            .and(path("/api/v4/merge_requests"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([raw_mr(1, 1, "Mine", "b1"), raw_mr(2, 2, "Theirs", "b2")])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/merge_requests/2/commits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "title": "Do it" }])))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/7/merge_requests/1/commits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{ "title": "Old" }])))
            .mount(&server)
            .await;

        // MR 1 was imported earlier (its address is known): only MR 2 should have its commits fetched.
        let known = HashSet::from(["https://gitlab.example/g/p/-/merge_requests/1".to_string()]);
        let changes = fetch_changes(&client(&server), 0, true, "all", &known).await.unwrap();
        assert_eq!(changes.iter().map(|m| (m.title.as_str(), m.role.as_str())).collect::<Vec<_>>(), [("Mine", "author"), ("Theirs", "assignee")]);
        assert_eq!(changes[0].commit_notes, None);
        assert_eq!(changes[1].commit_notes.as_deref(), Some("- Do it"));

        // And with include_assigned = false the assigned scope is never asked.
        let only_mine = fetch_changes(&client(&server), 0, false, "all", &known).await.unwrap();
        assert_eq!(only_mine.len(), 1);
    }
}
