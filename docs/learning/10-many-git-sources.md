# Milestone 8b — Many git sources (several GitLab servers + local repositories)

Follow-up to `09-gitlab-and-bulk.md`. Your point: git servers are many, each needs its own token, and extraction must not be tied to one host. Decisions you made: **GitLab only** for now (gitlab.com and any number of self-hosted servers), **local repositories as a fallback for any host**, **everything your token can see**.

## The idea: a connection is a source, a change is the common record
```
GitLab server A (token A) ─┐
GitLab server B (token B) ─┼─▶ GitChange {url, reference, title, description, state, branch, dates, role, commit notes}
local repository folder   ─┘            │
                                        ▼
                         tasks · story drafts · bulk push   (none of these knows which source it came from)
```
To support another host later (GitHub, Bitbucket...), write one reader that returns `GitChange`s. Nothing else changes. That is also how it differs from tools tied to a few hosts: Rovo Dev's Git integrations were reported (community posts, unverified) as covering Bitbucket Cloud and GitHub only, not GitLab.

## Files
| File | Role |
|---|---|
| `git.rs` | `GitChange`, connections (add / edit / remove, one token each), importing changes into tasks, importing from many sources, Jira-key detection, commands |
| `gitlab.rs` | Only the GitLab client: `PRIVATE-TOKEN`, paging, commit subjects → `Vec<GitChange>` |
| `localgit.rs` | Reads your commits from a repository folder with the `git` command |
| `db.rs` | Migration **v7**: `git_connections`, `merge_requests` rebuilt around the web address |
| `GitConnectionsCard.svelte`, `ImportMrDialog.svelte` | Settings list and a multi-source import dialog |

## Ideas

**1. One token per server, stored separately.** Each GitLab connection has its own `secret_name` (`git-token-<id>`) in the OS keychain (or the 0600 file fallback). Editing a connection's address without a new token **deletes that connection's token** (it must not be sent to a different server); the other connections' tokens are untouched (tested).

**2. Identity by web address, not by number.** GitLab's numeric id is only unique *per server*, so two servers could both have "merge request 1". Changes are now identified by their web address (`local://<repo>#<branch>` for branches), which is unique everywhere. SQLite can't change a column's constraint, so migration v7 **rebuilds the table** and copies the rows; your old single GitLab setup becomes the first connection and keeps its token under its old name, so nothing has to be re-entered (tested).

**3. A failing source never stops the others.** Import runs each connection in turn and returns one result *per connection*: found/new/attached, or its own error ("Add an access token for "Client GitLab"", "rejected the token", "not a git repository"). The dialog shows each line.

**4. Local repositories, safely.** `git` is started with an argument list, never through a shell, so a path or branch name can't inject commands, and only read commands run (`rev-parse`, `config`, `for-each-ref`, `log`). Your commits are found with `--author=<your@email>`, anchored on the angle brackets and regex-escaped so `me@x.com` can't also match `pyme@x.com` (tested). A feature branch plays the part of a merge request: other than the main line, every branch with commits by you in the period becomes one task, titled from its name ("feature/abc-12-add-apple-pay" → "Add apple pay"), with the commit subjects as its context for the AI story.

**5. Know the limits.** A local repository only finds branches that still exist (locally or as `origin/...`); a branch merged and deleted leaves nothing behind. Squash-merged work on `main` isn't attributed to a branch.

## Not done on purpose
GitHub, Bitbucket, Azure DevOps, Gitea (you chose GitLab-only for now); per-project limits (you chose "everything my token can see"); a native folder picker (you type or paste the path).

## Try it
1. ⚙ Settings → **Git sources** → *Add a GitLab server* (address + a `read_api` token) once per server; *Add a repository folder* for local clones.
2. **Test** on each. **⬇ Import from git** → tick the sources, pick a start date → Import.
3. A source with no token is greyed out in the dialog; a source that fails is listed on its own line without stopping the others.

## Exercises
1. Add a "Last imported" time per connection.
2. Let a local repository list several author emails.
3. Show the connection name on the merge-request chip's tooltip.
