# Milestone 6b — A Jira-aware task list (parents, months, staged changes, better UI)

Follow-up to the push milestone, driven by real use: stories grouped under epics, a month view, assignee on create, "time logged in Jira" next to your timer, and one rule: **nothing reaches Jira until you press Push.**

## The one rule: pull is free, push is explicit
| Action | Touches Jira? |
|---|---|
| **Refresh from Jira** | Only *reads* (search). Never writes. |
| Choose a parent, rename, tick done, run timers | Local only. A chosen parent is *staged* (`pending_parent_key`) and shown as "not pushed yet". |
| **Push** (on a task) | The only thing that writes: creates the issue (with assignee/parent/etc.), sets a staged parent on an existing issue, logs time. |

The top bar says "Waiting to be pushed": unlogged time, tasks to create, parent changes.

## Data model (migration v5)
`tasks` gained: `created_at`, `issue_type`, `parent_key` (what Jira says), `pending_parent_key` (what you chose, not yet in Jira), `jira_logged_seconds`, `context_only`.
- *Effective parent* = `pending_parent_key ?? parent_key` (`effectiveParent` in `grouping.ts`).
- On Refresh, if Jira's parent now equals the staged one, the staged value is cleared (nothing left to push). If you staged a *different* one, Refresh keeps yours.
- `context_only` rows are parents that aren't yours (e.g. an epic owned by someone else). Jira returns the parent's key, title and type inside each issue, so no extra request is needed. They show as headings only: no timer, no push, and never counted in totals.

## Time logged in Jira
Refresh reads Jira's `timespent` per issue → "Jira 2h 15m" under your local clock. After a push, the number is bumped locally right away (`add_logged_seconds`), and the next Refresh replaces it with Jira's own value. "+30m to push" is your finished-but-unlogged time.

## Creating issues properly
The create screen now offers (when the project has them on its create screen): **Assignee** (defaults to you), **Priority**, **Labels**, **Components**, **Fix versions**, **Due date**; plus the project's required fields as before, and the staged **Parent**. Rules enforced in Rust:
- A parent must be in the same project (`same_project`).
- If the issue type's create screen has no `parent` field, the push is blocked with a clear message *before* anything is created.
- For an existing issue, a refused parent change becomes a **warning**; your time is still logged and the parent stays staged.

"Project default" in Priority/Assignee means "don't send the field": Jira applies its own default. (Explicitly forcing "Unassigned" isn't supported yet.)

## Frontend structure
```
src/app.css            design tokens (light + dark), buttons, pills: change the look here
src/lib/grouping.ts    groupByParent, filterGroups, groupByMonth, periodBounds   <- unit-tested
src/lib/TaskRow.svelte one task: type pill, key, parent picker, live clock, Jira time, Push
src/lib/PushDialog.svelte, SettingsCard.svelte
src/routes/+page.svelte summary cards, view switch, search, groups
```
Why `grouping.ts` is plain TypeScript: logic with edge cases (cycles, staged parents, month boundaries) is easiest to test without a UI. Run `npm test`.

## Ideas worth remembering
- **Pure functions + a test file** beat clicking around for anything with rules.
- **Group by root ancestor with a step limit**: parent links come from outside, so assume they can loop; `rootOf` stops instead of hanging.
- **Local time zone for months**: "created in October" means October where *you* are, so `monthKeyOf` uses `getMonth()` (local), not UTC.
- **Dev-only mock backend** (`src/lib/devmock.ts`): open `http://localhost:1420` in a normal browser (`npm run dev`) and the UI runs against fake data. It loads only when Tauri isn't present and only in dev builds.
- **SQL for windows of time**: `tracked_between` clips each entry to `[from, to)` with `MIN/MAX`, and counts a running timer up to *now*.

## Not done on purpose
- Renaming a Jira-linked task (it needs its own "update summary" push).
- Removing a parent already in Jira; changing the assignee of existing issues.
- Sprints, original estimate, and cascading selects. Say if you need them.

## Try it
```bash
cd ~/ProjectJira && npm run tauri dev
```
1. **Refresh from Jira**: epics appear as headings with your stories below. Switch to **By month created**.
2. On a story with no parent, pick an epic in **Parent**: it shows "not pushed yet" and "Waiting to be pushed" counts it.
3. **Push** it: the dialog says "set the parent of ABC-12 to ABC-1" and logs any unlogged time.
4. Push a *local* task into a **test project**: Assignee defaults to you.
