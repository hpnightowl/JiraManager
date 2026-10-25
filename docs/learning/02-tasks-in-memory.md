# Milestone 2 — Tasks in memory

Goal: add, list, tick off and delete tasks. Data lives in Rust memory (lost on close; SQLite comes in milestone 3).

```
Svelte UI ──invoke("add_task", {title})──▶ #[tauri::command] add_task ──▶ TaskStore::add()
          ◀──────────── Task (JSON) ───────────────────────────────────── Result<Task, String>
```

## Files
| File | New concept |
|---|---|
| `src-tauri/src/tasks.rs` | structs, `Result`, `Mutex`, ownership, tests |
| `src-tauri/src/lib.rs` | `mod tasks;` + `.manage(TaskStore::default())` + `generate_handler![...]` |
| `src/routes/+page.svelte` | Svelte 5 runes (`$state`), `invoke`, `onMount`, `{#each}` |

## Rust ideas, each tied to the code

**1. Structs and derive** — `#[derive(Debug, Clone, Serialize)] struct Task {..}`.
`derive` asks the compiler to write boilerplate for you. `Serialize` (serde) makes `Task` convertible to JSON so Tauri can send it to the UI.

**2. `String` vs `&str`** — `add(&self, title: &str)` *borrows* text, it doesn't take ownership. `title.to_string()` makes an owned copy we can store in the `Task`. Rule of thumb: functions *read* with `&str`, structs *store* `String`.

**3. `Result<T, E>` instead of exceptions** — `Result<Task, String>` means "a Task, or an error message". Callers must handle both. The `?` operator says "if this is an error, return it right now", so `lock().map_err(..)?` stays one line. Tauri turns `Err(String)` into a rejected promise in JS, which is why the UI has `try/catch`.

**4. `&self` vs `&mut`** — `list(&self)` only reads. We still *mutate* inside `add` because the data sits behind a `Mutex`: you lock it, and the guard (`let mut inner`) gives temporary mutable access. The lock is released automatically when `inner` goes out of scope (no `unlock()` call to forget).

**5. Why `Mutex`?** Tauri may run commands on several threads. Rust won't compile code that shares mutable data across threads unsafely; `Mutex` is the standard fix.

**6. Iterators and closures** — `iter_mut().find(|t| t.id == id)` returns an `Option<&mut Task>`; `.ok_or_else(|| ..)` turns `None` into an `Err`. `retain(|t| t.id != id)` keeps only matching items.

**7. `.clone()`** — we return copies (`task.clone()`) because the original stays inside the store. Cheap here; avoid in hot loops.

**8. Tauri `State`** — `.manage(TaskStore::default())` registers one shared instance; a command declares `store: State<'_, TaskStore>` and Tauri injects it. `Default` gives an empty store.

**9. Testing** — the real logic is in `TaskStore` (no Tauri needed), so unit tests are plain Rust. Run: `cd src-tauri && cargo test`.

## Try it
```bash
cd ~/ProjectJira && npm run tauri dev
```
Add a task, tick it, delete it, try an empty title (you'll see the Rust error message). Edits to `.svelte` files hot-reload; edits to `.rs` files rebuild and restart the app.

## Exercises (optional)
1. Add a `rename_task(id, title)` command (copy the shape of `toggle`).
2. Make `add` reject titles longer than 200 characters; add a test.
3. Show a count of open tasks in the header.
