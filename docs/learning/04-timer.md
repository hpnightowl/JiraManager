# Milestone 4 — Start/stop timer (crash-safe)

Goal: ▶ starts timing a task, ■ stops it, totals add up across sessions, only one timer runs at a time, and a crash or forgotten timer can't silently inflate your hours.

## Files
| File | Role |
|---|---|
| `db.rs` | Migration **v2**: `time_entries` table + the "only one running timer" index |
| `timer.rs` | New. `start_timer / stop_timer / heartbeat / stale_timer` + commands + tests |
| `tasks.rs` | `Task` gains `tracked_seconds` and `running_since`; list/get use a JOIN |
| `+page.svelte` | ▶/■ buttons, live clock, "timer left running" banner |
| `Cargo.toml` | + `chrono` (only its clock, to read the current UTC time) |

## Design decisions (the important part)

**1. Store the *start*, not a counter.** A timer row is `(task_id, started_at, ended_at)`, `ended_at = NULL` while running. Nothing is written every second. Elapsed = `ended_at - started_at`, or `now - started_at` while running. So the data is correct even if the app dies: the row is already on disk.

**2. Times are Unix seconds in UTC.** No time zones or text parsing in the database. Convert to local time only when showing it (`toLocaleTimeString`) and later to ISO-8601 for Jira's worklog `started` field.

**3. The database enforces "one running timer".**
```sql
CREATE UNIQUE INDEX one_running_timer ON time_entries((1)) WHERE ended_at IS NULL;
```
A *partial* index only covers rows where `ended_at IS NULL`; indexing the constant `(1)` and marking it UNIQUE means a second running row is rejected by SQLite itself. (Indexing `ended_at` wouldn't work: NULLs never count as equal.) A test inserts two running rows by hand and expects failure. Rule of thumb: guard invariants in the database *and* the code.

**4. Transactions.** `start_timer` stops the old timer and starts the new one inside one transaction, so we never see "two timers" or "no timer" halfway. `let tx = conn.transaction()?;` … `tx.commit()?;`. If we return early (`?` error, or "already running") the `tx` is dropped and rolled back automatically: Rust's drop = cleanup.

**5. Time is a parameter.** `start_timer(task_id, now)` doesn't call the clock; the *command* wrapper does. Tests then say "start at 1000, stop at 1090 → 90 s" with no sleeping and no flakiness. This "inject the clock" trick is worth using everywhere.

**6. Clamping.** `MIN(MAX(?1, started_at), ?2)` forces the end time into `[started_at, now]`. Whatever the UI sends, you can't get negative or future time. (With two arguments, SQLite's `MIN`/`MAX` pick the smaller/larger value.)

**7. Crash recovery with a heartbeat.** While a timer runs, the UI calls `heartbeat` every 15 s, which stamps `heartbeat_at`. At startup, `stale_timer` looks for a running row whose heartbeat is older than 60 s: the app wasn't alive. The UI then asks: *stop it at the last moment we know you were there*, or *keep running*. We never guess silently.

**8. Computing the display in the UI.** Rust sends `tracked_seconds` (finished sessions) and `running_since`. Svelte ticks a `nowSec` state every second and shows `tracked + (now - running_since)`. Rust stays idle while the clock runs.

## Rust ideas
- **Several `impl` blocks, several files**: `timer.rs` adds methods to `TaskStore`, which lives in `tasks.rs`. For that, `conn()` became `pub(crate)` ("visible inside this app, not to outside users").
- **`Option<T>` crossing the boundary**: `running_since: Option<i64>` becomes `number | null` in TypeScript; `at: Option<i64>` accepts `null` from JS.
- **Name conversion**: JS passes `{ taskId }`, Rust receives `task_id`. Tauri converts camelCase ↔ snake_case for command *arguments*; the struct fields we send back stay `snake_case`.
- **`PartialEq` derive** on `StaleTimer` lets tests compare with `assert_eq!`.
- **`#[cfg(test)]`** on `in_memory()` means that helper isn't even compiled into the real app.

## Try it
```bash
cd ~/ProjectJira && npm run tauri dev
```
Your existing tasks keep working: the app upgrades the database from v1 to v2 on first start.
1. Add a task, press ▶: the row turns green and ticks. Press ■: time is kept. ▶ again: it adds up.
2. Add a second task and press its ▶: the first timer stops by itself, at that second.
3. **Crash test:** start a timer, close the window, wait about 90 s, reopen. A yellow banner offers to stop it at the time you closed the app. (Reopen within 60 s and it just continues.)

## Exercises (optional)
1. Show a "today" total in the header (sum of `elapsed` over tasks).
2. Add a `add_manual_time(task_id, seconds)` command for forgotten timers (insert a closed entry ending at `now`). Test it.
3. Round the displayed time up to 15-minute blocks in a `fmt15()` function; this is what we'll later need for Jira.
