# Milestone 5 — Connect to Jira and fetch my issues

Goal: save Jira URL + email + API token, test the connection, pull "my open issues" into the task list (no duplicates on re-fetch).

## Files
| File | Role |
|---|---|
| `secrets.rs` | Stores the API token: OS keychain, or a 0600 file when none exists (WSL) |
| `jira.rs` | URL normalising, `JiraClient` (async HTTP), settings, sync into SQLite, Tauri commands |
| `db.rs` | Migration **v3**: `settings` table, `tasks.jira_key` / `jira_status`, unique index on `jira_key` |
| `tasks.rs` | `Task` gains `jira_key`, `jira_status`; `get_setting` / `set_setting` |
| `+page.svelte` | Settings card, "Fetch from Jira", key chip that opens the issue |
| `Cargo.toml` | + `reqwest` (HTTP), `keyring`; dev: `wiremock`, `tokio` |

## Getting an API token (you do this, in the browser)
id.atlassian.com → Security → **API tokens** → Create. Paste it into the app's Settings. Don't paste it anywhere else (chat, notes, git).

## Ideas

**1. Secrets are not settings.** URL and email go in SQLite (`settings` table). The token goes through `SecretStore`: OS keychain first (Windows Credential Manager / Linux Secret Service), else a file with mode `0600` created *with that mode from the start*. The UI is told *where* it landed (`token_storage`) so a downgrade is never silent. The token is never sent back to the UI: `JiraSettings` only has `has_token: bool`. A test checks the token can't appear in the settings' `Debug` output.

**2. Async Rust.** `async fn` returns a *future*; nothing runs until `.await`. Tauri runs async commands on a tokio runtime. Rule you will hit: **don't hold a `MutexGuard` across an `.await`** (the guard isn't `Send`, so it won't compile). So commands do: read what they need from the DB (lock, copy, unlock) → build the client → `.await` the network → lock again to save. See `fetch_jira_issues`.

**3. Calling a REST API with `reqwest`.** Builder pattern: `.get(url).basic_auth(email, Some(token)).header(..).query(&[..]).send().await?`. Basic auth = `base64(email:token)` in the `Authorization` header, which is what Jira Cloud expects for API tokens. `resp.json::<T>()` parses straight into a struct.

**4. serde for foreign JSON.** We declare only the fields we need (`key`, `fields.summary`, `fields.status.name`); everything else in Jira's big response is ignored. `#[serde(rename = "nextPageToken")]` maps JSON names to Rust names; `#[serde(default)]` supplies a default when a field is missing.

**5. Turning HTTP failures into advice.** `send()` maps 401/403/404/429/3xx to sentences that tell the user what to check. Network failures (`reqwest::Error`) convert with `#[from]`, same trick as the database errors.

**6. Defence in depth for credentials.**
- `https://` required (plain http only for `localhost`, used by tests).
- Redirects **disabled**: a redirect can't smuggle your token to another host.
- Changing the Jira URL **forgets the saved token** unless you enter a new one.
- Error messages never include the token (tested).

**7. Jira's search API.** The old `/rest/api/3/search` was removed; we use `/rest/api/3/search/jql`, which pages with `nextPageToken` (no `startAt`). We loop until there's no token or `MAX_PAGES` is reached.

**8. Idempotent sync.** Re-fetching must not duplicate tasks: match on `jira_key` (backed by a *partial unique index*), update if present, insert otherwise, all in one transaction. Local-only tasks (no key) are never touched.

**9. Testing network code without a network.** `wiremock` starts a fake Jira on a random local port. Tests assert on what our client *sent* (the Basic header) and on how it handles 401, redirects and two pages of results. `#[tokio::test]` runs an async test.

## What is NOT tested here
- The real keychain paths (Windows Credential Manager, Linux Secret Service): this WSL has no keychain daemon, so only the file fallback was exercised.
- The real Jira Cloud: tests use a fake server. Your first "Test connection" is the real check.

## Try it
```bash
cd ~/ProjectJira && npm run tauri dev
```
1. ⚙ Settings → site URL, your Atlassian email, paste the token → **Save** → **Test connection** (should say "Connected to Jira as <your name>").
2. **Fetch from Jira** → your unresolved issues appear with a blue key chip and a status pill. Click the chip to open the issue in your browser.
3. Fetch again → "0 new, N updated" (no duplicates). Start a timer on a Jira task.
4. In WSL the settings page will say the token is in a *local file*; that's expected without a keychain.

## Exercises (optional)
1. Add a "last fetched" time to the `settings` table and show it next to the button.
2. Show a friendly message ("No open issues are assigned to you") when the search returns zero issues.
3. Add a test that a 429 response produces the rate-limit message.
