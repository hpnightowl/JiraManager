//! Reads your commits from a local repository with the `git` command, so it works with any host.
//! Each branch (other than the main one) with commits by you in the period becomes one change.
//! Only branches that still exist can be found. `git` runs without a shell and read-only.

use crate::error::AppError;
use crate::git::GitChange;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

const MAX_BRANCHES: usize = 100;
const MAX_COMMITS_PER_BRANCH: usize = 200;
const MAX_NOTES: usize = 2500;

/// Branch names that are "the main line", never a piece of work on their own.
const MAIN_LINES: &[&str] = &["main", "master", "develop", "dev", "trunk", "HEAD"];

/// Run `git -C <repo> <args>` and return its output. Errors say what to do about them.
fn git(repo: &Path, args: &[&str]) -> Result<String, AppError> {
    let mut cmd = Command::new("git");
    // We only ever READ. `GIT_OPTIONAL_LOCKS=0` stops git from touching the repository (it can
    // otherwise refresh the index behind your back), fsmonitor off means a repository's own config
    // can't make git run a program of its choosing, and no prompt can ever wait for a password.
    cmd.arg("-C").arg(repo).args(["-c", "core.fsmonitor=false"]).args(args);
    cmd.env("GIT_OPTIONAL_LOCKS", "0").env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: don't flash a console window
    }
    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AppError::Config("`git` isn't installed (or isn't on your PATH), so local repositories can't be read.".into())
        } else {
            AppError::Config(format!("Couldn't run git: {e}"))
        }
    })?;
    if !output.status.success() {
        let why = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AppError::Config(format!("git said: {}", if why.is_empty() { "an unknown error".into() } else { why })));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Is this folder a git repository? (Also used when adding one.)
pub fn check_repo(repo: &Path) -> Result<(), AppError> {
    if !repo.is_dir() {
        return Err(AppError::Config(format!("{} is not a folder", repo.display())));
    }
    git(repo, &["rev-parse", "--git-dir"]).map(|_| ()).map_err(|_| {
        AppError::Config(format!("{} is not a git repository (no .git inside)", repo.display()))
    })
}

/// The commit email configured in this repository (or globally).
pub fn configured_email(repo: &Path) -> Option<String> {
    git(repo, &["config", "user.email"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// The branch the others are compared against: origin's default if known, else a usual name.
fn default_branch(repo: &Path) -> Result<String, AppError> {
    if let Ok(out) = git(repo, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"]) {
        let name = out.trim().to_string();
        if !name.is_empty() {
            return Ok(name);
        }
    }
    for candidate in ["main", "master", "develop", "trunk"] {
        if git(repo, &["rev-parse", "--verify", "--quiet", candidate]).is_ok() {
            return Ok(candidate.to_string());
        }
    }
    Err(AppError::Config("Couldn't tell which branch is the main one (looked for origin's default, main, master, develop, trunk).".into()))
}

/// "feature/abc-12-add-apple-pay" -> "Add apple pay". Falls back to `fallback` (a commit subject)
/// when nothing readable is left.
pub fn humanize_branch(branch: &str, fallback: &str) -> String {
    let mut name = branch.rsplit_once('/').map_or(branch, |(_, last)| last).to_string();
    // A leading Jira-style key ("abc-12-") is noise in a title; the key is still found from the branch name.
    let lower = name.to_ascii_lowercase();
    if let Some(dash) = lower.find('-') {
        let (project, rest) = lower.split_at(dash);
        let digits: String = rest[1..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if !project.is_empty() && project.chars().all(|c| c.is_ascii_alphabetic()) && !digits.is_empty() {
            name = name[dash + 1 + digits.len()..].trim_start_matches(['-', '_', ' ']).to_string();
        }
    }
    let words = name.replace(['-', '_'], " ");
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.chars().count() < 4 {
        return fallback.trim().to_string();
    }
    let mut chars = words.chars();
    chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// Escape the characters that mean something in a (basic) regular expression.
fn escape_regex(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    for c in text.chars() {
        if matches!(c, '.' | '*' | '[' | ']' | '^' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn parse_git_date(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s.trim()).map(|d| d.timestamp()).unwrap_or(0)
}

/// Feature branches (local and `origin/...`, de-duplicated) other than the main line.
fn feature_branches(repo: &Path, default: &str) -> Result<Vec<String>, AppError> {
    let remotes: HashSet<String> = git(repo, &["remote"])?.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    let refs = git(repo, &["for-each-ref", "--format=%(refname:short)", "refs/heads", "refs/remotes"])?;

    let strip_remote = |name: &str| -> String {
        match name.split_once('/') {
            Some((first, rest)) if remotes.contains(first) => rest.to_string(),
            _ => name.to_string(),
        }
    };
    let default_plain = strip_remote(default);

    let mut seen = HashSet::new();
    let mut branches = Vec::new();
    for line in refs.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let plain = strip_remote(line);
        if plain == default_plain || MAIN_LINES.contains(&plain.as_str()) {
            continue;
        }
        if seen.insert(plain.clone()) {
            branches.push(line.to_string()); // keep the form git understands (local name or origin/name)
        }
        if branches.len() >= MAX_BRANCHES {
            break;
        }
    }
    Ok(branches)
}

/// Work done by `author` since `since` (Unix s), one change per feature branch.
pub fn fetch_changes(repo: &Path, author: Option<&str>, since: i64) -> Result<Vec<GitChange>, AppError> {
    check_repo(repo)?;
    let author = author
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(String::from)
        .or_else(|| configured_email(repo))
        .ok_or_else(|| AppError::Config("Set your commit email for this repository (git config user.email, or enter it in Settings).".into()))?;
    let default = default_branch(repo)?;
    let repo_name = repo.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "repo".into());
    let repo_id = repo.to_string_lossy().replace('\\', "/");
    let since_arg = format!(
        "--since={}",
        chrono::DateTime::from_timestamp(since, 0).unwrap_or_default().format("%Y-%m-%d %H:%M:%S +0000")
    );
    // `--author` is a regular expression matched against "Name <email>". Anchoring on the angle
    // brackets and escaping the email means "me@x.com" can't also match "pyme@x.com".
    let author_arg = format!("--author=<{}>", escape_regex(&author));

    let mut changes = Vec::new();
    for branch in feature_branches(repo, &default)? {
        let range = format!("{default}..{branch}");
        let log = git(
            repo,
            &["log", &range, "--no-merges", &author_arg, &since_arg, "--reverse", "--format=%H%x1f%aI%x1f%s", "--"],
        )?;
        // oldest first: (date, subject)
        let commits: Vec<(i64, String)> = log
            .lines()
            .filter_map(|line| {
                let mut parts = line.split('\u{1f}');
                let (_hash, date, subject) = (parts.next()?, parts.next()?, parts.next()?);
                Some((parse_git_date(date), subject.trim().to_string()))
            })
            .filter(|(_, subject)| !subject.is_empty())
            .take(MAX_COMMITS_PER_BRANCH)
            .collect();
        let Some((first_date, first_subject)) = commits.first().cloned() else { continue };

        let plain = branch.split_once('/').filter(|(first, _)| *first == "origin").map_or(branch.as_str(), |(_, rest)| rest);
        let notes: String = commits.iter().map(|(_, s)| format!("- {s}")).collect::<Vec<_>>().join("\n").chars().take(MAX_NOTES).collect();
        changes.push(GitChange {
            url: format!("local://{repo_id}#{plain}"),
            reference: format!("{repo_name}@{plain}"),
            title: humanize_branch(plain, &first_subject),
            description: String::new(),
            state: "local".into(),
            source_branch: plain.to_string(),
            created_at: first_date,
            merged_at: None,
            role: "author".into(),
            commit_notes: Some(notes),
        });
    }
    Ok(changes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn run(dir: &Path, who: &str, date: &str, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", who)
            .env("GIT_AUTHOR_EMAIL", format!("{who}@example.com"))
            .env("GIT_COMMITTER_NAME", who)
            .env("GIT_COMMITTER_EMAIL", format!("{who}@example.com"))
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .status()
            .expect("git must be installed to run these tests");
        assert!(status.success(), "git {args:?} failed");
    }

    fn commit(dir: &Path, who: &str, date: &str, file: &str, message: &str) {
        std::fs::write(dir.join(file), message).unwrap();
        run(dir, who, date, &["add", "."]);
        run(dir, who, date, &["commit", "-q", "-m", message]);
    }

    /// main (initial commit) + feature branches by me / by someone else / too old.
    fn sample_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pj-localgit-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        run(&dir, "me", "2026-09-01T10:00:00Z", &["init", "-q", "-b", "main"]);
        commit(&dir, "me", "2026-09-01T10:00:00Z", "a.txt", "Initial commit");

        run(&dir, "me", "2026-10-02T09:00:00Z", &["checkout", "-q", "-b", "feature/abc-12-add-apple-pay"]);
        commit(&dir, "me", "2026-10-02T09:00:00Z", "pay.txt", "Add Apple Pay button");
        commit(&dir, "me", "2026-10-03T09:00:00Z", "pay2.txt", "Handle declined cards");

        run(&dir, "me", "2026-10-04T09:00:00Z", &["checkout", "-q", "main"]);
        run(&dir, "me", "2026-10-04T09:00:00Z", &["checkout", "-q", "-b", "feature/theirs"]);
        commit(&dir, "colleague", "2026-10-04T09:00:00Z", "t.txt", "Someone else's work");

        run(&dir, "me", "2026-10-05T09:00:00Z", &["checkout", "-q", "main"]);
        run(&dir, "me", "2026-08-01T09:00:00Z", &["checkout", "-q", "-b", "old-experiment"]);
        commit(&dir, "me", "2026-08-01T09:00:00Z", "o.txt", "Old experiment");
        run(&dir, "me", "2026-10-05T09:00:00Z", &["checkout", "-q", "main"]);
        dir
    }

    fn since_oct_1() -> i64 {
        chrono::DateTime::parse_from_rfc3339("2026-10-01T00:00:00Z").unwrap().timestamp()
    }

    #[test]
    fn branches_become_changes_only_for_my_commits_in_the_period() {
        let repo = sample_repo("fetch");
        let changes = fetch_changes(&repo, Some("me@example.com"), since_oct_1()).unwrap();

        assert_eq!(changes.len(), 1, "{changes:?}"); // not 'theirs' (other author), not 'old-experiment' (too old)
        let c = &changes[0];
        assert_eq!(c.source_branch, "feature/abc-12-add-apple-pay");
        assert_eq!(c.title, "Add apple pay"); // readable, without the key noise
        assert_eq!(c.commit_notes.as_deref(), Some("- Add Apple Pay button\n- Handle declined cards"));
        assert_eq!(c.created_at, chrono::DateTime::parse_from_rfc3339("2026-10-02T09:00:00Z").unwrap().timestamp());
        assert!(c.url.starts_with("local://") && c.url.ends_with("#feature/abc-12-add-apple-pay"));
        assert!(c.reference.ends_with("@feature/abc-12-add-apple-pay"));
        assert_eq!((c.state.as_str(), c.role.as_str()), ("local", "author"));
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn a_wider_period_also_finds_older_branches() {
        let repo = sample_repo("wide");
        let all = fetch_changes(&repo, Some("me@example.com"), 0).unwrap();
        let branches: Vec<_> = all.iter().map(|c| c.source_branch.as_str()).collect();
        assert!(branches.contains(&"old-experiment") && branches.contains(&"feature/abc-12-add-apple-pay"));
        assert!(!branches.contains(&"feature/theirs"));
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn an_email_only_matches_itself_not_longer_addresses() {
        let repo = sample_repo("anchor");
        // "colleague@example.com" must not be matched by a shorter email that is a suffix of it.
        let suffix = fetch_changes(&repo, Some("e@example.com"), 0).unwrap();
        assert!(suffix.is_empty(), "{suffix:?}");
        assert_eq!(escape_regex("a.b+c@x.com"), "a\\.b+c@x\\.com");
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn the_author_defaults_to_the_repository_email() {
        let repo = sample_repo("author");
        run(&repo, "me", "2026-10-05T09:00:00Z", &["config", "user.email", "me@example.com"]);
        assert_eq!(fetch_changes(&repo, None, since_oct_1()).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(repo);
    }

    #[test]
    fn folders_that_are_not_repositories_are_refused_with_a_clear_message() {
        let dir = std::env::temp_dir().join(format!("pj-localgit-plain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(check_repo(&dir).unwrap_err().to_string().contains("not a git repository"));
        assert!(check_repo(&dir.join("does-not-exist")).unwrap_err().to_string().contains("not a folder"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn branch_names_become_readable_titles() {
        assert_eq!(humanize_branch("feature/abc-12-add-apple-pay", "x"), "Add apple pay");
        assert_eq!(humanize_branch("fix/login_redirect_loop", "x"), "Login redirect loop");
        assert_eq!(humanize_branch("ABC-7", "Fallback subject"), "Fallback subject"); // nothing readable left
        assert_eq!(humanize_branch("wip", "Real commit subject"), "Real commit subject");
        assert_eq!(humanize_branch("improve-search-speed", "x"), "Improve search speed");
    }
}
