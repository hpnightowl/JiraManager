# Milestone 8 — GitLab merge requests, a story for every task, bulk push

You asked for: stories from the MRs you raised, a story + title + Push for every task you enter, and multi-select. Decisions you made: **authored + assigned MRs**, **one task per MR**, **one review screen for bulk push**, **no automatic time guessing**.

## The flow
```
GitLab (read-only token)                     local AI (Ollama)                        Jira
  MRs you authored / are assigned  ──▶  task per MR  ──▶  ✨ draft story per task  ──▶  Push / Push selected
  (title, description, commit subjects)    (or attached to           (saved on the task,         (only on your confirmation)
                                            an existing Jira task)    you can edit it)
```
Nothing is written to GitLab, ever. Nothing goes to Jira until you confirm in a Push dialog.

## Files
| File | Role |
|---|---|
| `gitlab.rs` | GitLab client (`PRIVATE-TOKEN`), paging, finding `ABC-123` in titles/branches, importing MRs into tasks, settings, commands |
| `story.rs` | Story drafts saved on tasks; generate one or many (AI or template) |
| `push.rs` | + `push_many_impl` / `push_tasks` / `get_bulk_preview` |
| `db.rs` | Migration **v6**: `tasks.story_*`, `merge_requests` table |
| `CreateFields.svelte` | The project / issue type / required + optional fields form, shared by both push dialogs |
| `BulkPushDialog.svelte`, `ImportMrDialog.svelte`, `GitlabSettingsCard.svelte` | New screens |
| `forms.ts` | `missingRequired()`: "Still needed before you can push: Team" (tested) |

## Ideas

**1. Reading MRs without reading code.** We ask GitLab for `scope=created_by_me` and, if you ticked it, `scope=assigned_to_me` (an MR in both counts once, as "author"). For each *new* MR we fetch only the commit **subjects** (one line each). Diffs are never downloaded, so they can't end up in an AI prompt. The token needs only the `read_api` scope.

**2. One task per MR, never twice.** `gitlab_id` is unique, so re-importing only refreshes state/title. If an MR's title or branch contains a Jira key we already know (`ABC-12: fix…`, `feature/abc-12-fix`), it attaches to that task instead of creating a duplicate. If you delete a task, its MR row stays (`ON DELETE SET NULL`), so the next import doesn't resurrect it.

**3. Finding Jira keys safely.** Text like `UTF-8`, `SHA-256` or `x-ABC-1` look like keys. The scanner requires a word boundary and digits not glued to letters; once you know your Jira projects (from fetched issues) it only accepts those, in any case; before that it only accepts upper-case keys not on a small deny-list.

**4. Story drafts live on the task.** `story_title` / `story_description` (description text includes an "Acceptance criteria:" bullet list) / `story_source` (`ai`, `template`, `manual`). Push starts from the draft; Cancel in the Push dialog **saves what you typed as the draft**, so nothing is lost. For an MR task the AI sees the MR description and commit subjects, so the story reflects the actual work.

**5. Every task can get a story.** Row button **✨ Draft story** (one task), or select tasks and **✨ Draft stories (N)**. If the AI is down, generation *stops* with a clear message and offers **simple templates** for the rest instead of failing N times. A task already in Jira needs no story and is skipped.

**6. Selection vs "done".** The leading checkbox now means *select*; "mark done" moved to a ✓ button. Select a few rows (or **Select all shown**) and a bar appears: **Draft stories** and **Push selected**.

**7. Bulk push = one set of answers, many issues.** You choose project, issue type, assignee, required fields **once**; each task is pushed with *its own* draft story and its own parent/time. The table shows, per task: new issue or existing key, story drafted or "title only", time. Failures are **per task**: a refused issue never blocks the others, never half-saves (the task just stays unpushed), and pushing again only does what's missing. The dialog won't enable the button while required fields are empty.

**8. Sharing UI by extraction.** The create-issue form used to live inside one dialog. Pulling it out into `CreateFields.svelte` (with `$bindable` props) let two dialogs use identical logic. Rule of thumb: the second time you need something, extract it.

**9. Don't shadow runes.** A variable called `state` hid Svelte's `$state` and caused confusing type errors. Names that start with `$` are reserved by Svelte; avoid variables that collide with the rune names.

## Not done on purpose
- **No time from commits** (your choice): MR open→merge time isn't work time.
- No pagination beyond 500 MRs per scope, and no group-the-MRs logic (one task per MR).
- A GitLab on a sub-path (`https://host/gitlab`) isn't supported yet (only scheme + host + port are kept).
- Real GitLab hasn't been tried: tests use a fake server built from GitLab's documented responses. The commits endpoint (`/projects/:id/merge_requests/:iid/commits`) is from memory, not verified against the docs.

## Try it
1. ⚙ Settings → **GitLab**: address (default `https://gitlab.com`), a token with only `read_api` → Save → Test connection.
2. **⬇ Import merge requests** → pick a start date → Import → **✨ Draft N stories**.
3. Tick several tasks → **⬆ Push selected** → choose a **test project**, fill the required fields → review the table → Push.
4. Try Cancel in a single Push dialog after editing the description: reopen it, your text is back.

## Exercises
1. Show the MR's description in a tooltip on the MR chip.
2. Add "Select group" on each epic heading.
3. In the bulk table, let each row have its own title edit.
