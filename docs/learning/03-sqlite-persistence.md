# Milestone 3 — SQLite persistence

Goal: tasks survive closing the app. The UI didn't change at all — only the Rust storage did. That's the payoff of keeping the UI↔Rust boundary small.

## Files
| File | Role |
|---|---|
| `src-tauri/src/db.rs` | Schema migrations (`MIGRATIONS` list + `migrate()`) |
| `src-tauri/src/error.rs` | One `AppError` enum for the whole backend |
| `src-tauri/src/tasks.rs` | `TaskStore` now wraps a SQLite `Connection` |
| `src-tauri/src/lib.rs` | `setup()` finds the app-data folder, opens the DB, registers the store |
| `Cargo.toml` | + `rusqlite` (feature `bundled` compiles SQLite into the binary: nothing to install on Windows/Linux) + `thiserror` |

Database file: Linux `~/.local/share/com.projectjira.app/projectjira.db`, Windows `%APPDATA%\com.projectjira.app\projectjira.db`.

## Ideas, each tied to the code

**1. Migrations** — databases outlive app versions, so schema changes must be *appended*, never edited. `MIGRATIONS` is an ordered list; SQLite's header field `user_version` records how many have run; `migrate()` runs only the new ones, each in a transaction (all-or-nothing). Idempotent: running twice does nothing (tested).

**2. Bound parameters** — `INSERT ... VALUES (?1)` with `params![title]`. Never paste user text into SQL (SQL injection). There's a test that stores `x'); DROP TABLE tasks; --` literally.

**3. A real error enum** — `AppError` with `thiserror`:
- `#[error("...")]` generates the message/`Display`.
- `#[from] rusqlite::Error` makes `?` auto-convert DB errors into `AppError::Db`.
- We `impl Serialize` so Tauri can send it to the UI as a string. The Svelte `catch` is unchanged.
- Pattern to remember: *library errors in, one app error out.*

**4. `Option` vs error** — `query_row(...).optional()?` turns "no such row" into `None`; `.ok_or(AppError::NotFound(id))` then makes it *our* error. `execute()` returns the number of rows changed; `0` on UPDATE/DELETE means unknown id.

**5. Lifetimes, lightly** — `MutexGuard<'_, Connection>`: the guard borrows from `self`, and `'_` says "tied to whatever `self` lives". While you hold the guard the lock is held; it's released when the guard drops (end of the function).

**6. `setup()` and `app.manage()`** — in milestone 2 the store existed before the app started. Now we need the OS data path, which is only known once Tauri is running, so we create and register the store inside `.setup(|app| ..)`. `?` works there because errors convert into `Box<dyn Error>`.

**7. Tests with real SQLite** — `Connection::open_in_memory()` gives a throwaway DB per test (fast, isolated). One test uses a temp file and reopens it to prove persistence.

## Try it
```bash
cd ~/ProjectJira && npm run tauri dev
```
Add two tasks, tick one, **close the window, start again** — they're still there. Inspect the file:
```bash
sudo apt-get install -y sqlite3        # optional, one-time
sqlite3 ~/.local/share/com.projectjira.app/projectjira.db "SELECT * FROM tasks; PRAGMA user_version;"
```

## Exercises (optional)
1. Write migration v2: `ALTER TABLE tasks ADD COLUMN created_at TEXT;` (append to `MIGRATIONS`, don't edit v1). Then start the app: your existing tasks keep working — that's the whole point.
2. Add `created_at` to `Task` and show it in the UI.
3. Add a test that `migrate` upgrades a DB that was created at v1.
