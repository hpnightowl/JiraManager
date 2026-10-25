# Security review

Last reviewed: 2026-10-25, before the first push to GitHub. This is a self-review of the code, the
dependencies and the git history, not an independent penetration test.

## What the app handles, and what it protects
| Asset | Where it lives | Protection |
|---|---|---|
| Jira API token, GitLab tokens | OS keychain; where none exists, a file in the app folder | Keychain, or file mode `0600` (also tightened if an older looser file exists). Never in the database, never in logs, never sent back to the window (tests assert this). |
| Tasks, time, story drafts | One SQLite file in the app folder | Local only. Contains no tokens. |
| Your work text sent to the AI | Local Ollama on this computer | The address must be this computer (enforced, with tests). Nothing goes to a cloud AI. |
| Jira/GitLab content | Read over https | Redirects are not followed (so a token can't be forwarded to another host); http is refused except for localhost. |

## Findings and what was done

### Fixed in this review
1. **No Content-Security-Policy.** Added a strict CSP (own files only, no remote scripts, no framing,
   no form posts). All network access already goes through Rust, so the window needs none.
2. **The window could open any web address.** It had the generic "opener" permission. Removed it; the
   window now asks an `open_link` command which opens only https addresses on your Jira site, your
   GitLab servers, and the Ollama download page. Refuses look-alike hosts (`acme.atlassian.net.evil.com`),
   `user@host` tricks, other ports, `file:`, `javascript:` (tests in `links.rs`).
3. **Six vulnerable TLS crates** (`rustls`, `rustls-webpki`, `webpki-root-certs`: RUSTSEC-2026-0346 to
   0349, 0382, 0383), on the path to Jira and GitLab. Updated to patched versions. `tokio` updated too
   (RUSTSEC-2026-0386). CI now fails on any new vulnerability.
4. **Secret file hardening**: an existing world-readable secret file is tightened to `0600` before a
   secret is written to it; secret names can no longer contain `../`.
5. **Reading local repositories is now strictly read-only**: `GIT_OPTIONAL_LOCKS=0` (git won't touch
   your repository), `core.fsmonitor=false` (a repository's own config can't launch a program),
   `GIT_TERMINAL_PROMPT=0` (no hidden password prompts).
6. **`.gitignore` was excluding `docs/`** and had no rules for keys. Fixed; added `*.pfx`, `*.pem`,
   `*.key`, `*.asc` (except the public release key), `secret-*`.
7. **No secrets in the repository**: working tree and the full git history were scanned (token
   patterns, private keys). The only hits are obviously fake test values.

### Checked and fine
- SQL is parameterised everywhere; the one `format!` into SQL uses compile-time constants.
- JQL is built from fixed text plus a number from a fixed list; Jira keys are validated before they go
  into a URL path.
- No `{@html}`, `innerHTML`, `eval` in the interface (Svelte escapes text).
- The development fake backend (`devmock.ts`) is only loaded when `import.meta.env.DEV` is true and is
  removed from production builds.
- Stopping Ollama only kills a process whose name is `ollama` and whose pid the app recorded.
- AI: prompts fence Jira/GitLab text as data, the model has no tools and no network, its answer is
  plain text you review before anything is saved or pushed.
- The model download tag is validated before it is passed on.

### Known and accepted
- **Unmaintained transitive crates** (`zvariant`, `zbus_names` via the keychain library; `glib`,
  `proc-macro-error` via Tauri's GTK layer). Not exploitable by anything the app does, cannot be fixed
  without upstream releases. CI ignores exactly these IDs and nothing else.
- **Three low npm advisories** (`cookie` inside `@sveltejs/kit`). Build/dev tooling only, not part of
  the shipped app. The fix is a major SvelteKit upgrade; scheduled with the next toolchain bump.
- **Token file fallback** (no keychain, e.g. WSL): protected by file permissions only. The Setup screen
  says so whenever it is in use.
- **"Nothing is written to Jira without confirmation" is enforced by the interface**, not re-checked in
  the backend. That is acceptable only because the window loads no remote content (strict CSP, no
  `@html`, no arbitrary navigation). Keep it that way.
- Installing Ollama uses the vendor's own installer; read it before running it.
- There is no auto-update, so a new version only arrives when you install it.

## Releases
See `docs/RELEASING.md`: tag-only, main-only, pinned actions, secrets only in a gated environment,
signatures verified in the pipeline, draft release you approve by hand.

## Reporting a problem
This is a private project; open an issue in the repository or contact the maintainer directly.
