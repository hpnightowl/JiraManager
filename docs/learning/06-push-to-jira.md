# Milestone 6 — Push to Jira: create the issue, fill required fields, log time

This completes the MVP loop: capture a task → time it → push it to Jira (issue + worklogs).

> **Test in a sandbox project first.** Pushing creates real issues and real worklogs in your Jira. Use a test project (or one you can clean up) for the first runs.

## Files
| File | Role |
|---|---|
| `adf.rs` | Plain text → ADF (the JSON document format Jira Cloud v3 requires for descriptions) |
| `fields.rs` | Jira's "required fields" → `FieldSpec` (what to draw) → JSON payload (what to send). Pure functions |
| `jira.rs` | New client calls: projects, issue types, field metadata, user search, create issue, add worklog |
| `push.rs` | Dry-run preview, rounding, the push itself, "already logged" bookkeeping, Tauri commands |
| `db.rs` | Migration **v4**: `time_entries.jira_worklog_id`, `pushed_at` |
| `src/lib/PushDialog.svelte` | The dialog: project/type pickers, dynamic form, time table, summary |
| `src/lib/types.ts`, `format.ts` | Shared TypeScript types; duration/date helpers |

## The flow
```
Push ▶ dialog opens
  ├─ preview: finished, not-yet-logged entries (+ rounding)        get_push_preview
  ├─ (new issue) project → issue types → required fields           list_jira_projects, list_issue_types, get_create_form
  └─ "This will create 1 Story in ABC and log 2 worklogs totalling 1h 15m."   ← dry run
Confirm ▶ push_task
  1. no Jira key yet?  re-fetch field list → validate answers → POST /issue → SAVE the key at once
  2. each pending entry → POST /issue/{key}/worklog → mark that entry as logged
```

## Ideas

**1. Required fields are data, not code.** Each Jira project/issue type demands different fields. `GET /issue/createmeta/{project}/issuetypes/{type}` lists them with `required`, a `schema` (type), and `allowedValues`. `classify()` in `fields.rs` maps a schema to a `FieldKind` (`option`, `multi_option`, `date`, `user`…); the dialog draws a control per kind. New custom fields need no new code if their *type* is known.

**2. An enum that is also a contract.** `FieldKind` is a Rust `enum` serialised as snake_case strings, and `types.ts` mirrors it as a TypeScript union. Add a variant in Rust and the compiler forces you to handle it everywhere in Rust (`match` must be exhaustive); update the union and `svelte-check` flags unhandled cases in the UI.

**3. Parse, don't trust.** Answers arrive from the UI as loose JSON. `build_value` checks each by kind (number parses and is finite, date is `YYYY-MM-DD`, option id must be in `allowedValues`) and produces the exact shape Jira wants (`{"id": "201"}`, `[{"id":"11"}]`, `{"accountId": "…"}`, ADF for rich text). All problems are collected into one message.

**4. Validate twice.** The UI shows the fields, but Rust re-fetches the field list at push time and validates again. The UI is a convenience; the backend is the rule.

**5. Idempotency without a server key.** Jira can't dedupe for us. So: a task's Jira key is saved the instant the issue exists, and each time entry gets a `jira_worklog_id` the instant its worklog exists. A second push finds nothing new to do (tested: one POST to `/issue`, one to `/worklog`, even after pushing twice). The remaining gap (app killed between Jira's reply and our save) is documented in `push.rs`.

**6. Failing partway, gracefully.** If the issue is created but a worklog is refused (permissions, closed issue), the push returns `Ok` with `warnings`. The issue stays linked, successfully logged entries stay marked, and pushing again retries only the rest. We stop at the first refused worklog instead of hammering Jira.

**7. Rounding.** `round_up(seconds, minutes)` = `⌈s / block⌉ × block` in integer maths: `(s + block − 1) / block * block`. Entries under 60 s are skipped (Jira's minimum) *before* rounding, so a 30-second misclick can't become 15 minutes.

**8. Safe URLs.** Project keys, type ids and issue keys end up in URL paths, so `segment()` only allows letters, digits, `_` and `-`. `../`, `?` or `#` are rejected before any request exists. And in `create_issue`, `project` / `issuetype` / `summary` are inserted *after* the extra fields so extras can't override them (tested).

**9. Time formats.** Jira wants `2026-10-08T14:30:00.000+0000` for `started` (no colon in the offset). We store Unix seconds and format in UTC with chrono: `%Y-%m-%dT%H:%M:%S%.3f%z`.

**10. Svelte detail.** `bind:value` plus `onchange` on one `<select>` doesn't guarantee which runs first, so the handlers take the value from the event (`e.currentTarget.value`) and pass it along.

## What the form supports (and doesn't)
Supported: text, multi-line text, number, date, single select, multi select (components/versions), labels, person picker.
Not yet: date-times, cascading selects, multi-user fields, sub-tasks (they need a parent). If an unsupported field is *required*, the dialog says so and tells you to create that issue in Jira. Your time is kept; once the issue exists in Jira, fetch it and push the time.

## What is NOT tested
Everything is tested against a fake Jira built from the documented response shapes (`fields.rs` fixture, `wiremock`). Real Jira instances vary (company-managed vs team-managed projects, custom field types), so **your first real run is the real test**. If a field shows up as unsupported or Jira rejects the request, copy the message: it includes Jira's own explanation.

## Try it
```bash
cd ~/ProjectJira && npm run tauri dev
```
1. Add a task, ▶ for a couple of minutes, ■.
2. **Push** on the row → pick a **test project**, issue type → fill the required fields → read the sentence at the bottom → **Create issue and log time**.
3. Open the issue (blue key chip). Check the description and the worklog.
4. Push again: "Log time" is gone (nothing left to log); run another timer and push → only the new time is logged.

## Exercises (optional)
1. Show the unpushed time next to the Push button (`unpushed_seconds`).
2. Add a `date_time` field kind (Jira wants `2026-10-08T14:30:00.000+0000`).
3. Add a "Push all" button for month-end that opens the dialog for each task in turn.
