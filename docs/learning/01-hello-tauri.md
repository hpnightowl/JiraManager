# Milestone 1 — Hello Tauri

## The big picture
A Tauri app is two programs in one window:

```
 Svelte UI (src/)  ──invoke("greet", {name})──▶  Rust (src-tauri/src/lib.rs)
 runs inside the OS webview                      native process, full OS access
                   ◀────────── String ──────────
```
- The **UI** is just a web page (HTML/CSS/TS). It can't touch files, SQLite, keychain, etc.
- The **Rust side** can. The UI asks it to do things through *commands*.
- Our Jira token will live only on the Rust side, never in the webview.

## Files to know
| Path | What it is |
|---|---|
| `src/routes/+page.svelte` | The main page (SvelteKit, SPA mode) |
| `src-tauri/src/main.rs` | Tiny entry point; just calls `project_jira_lib::run()` |
| `src-tauri/src/lib.rs` | Real app setup + commands. We'll grow this |
| `src-tauri/Cargo.toml` | Rust dependencies (like `package.json`) |
| `src-tauri/tauri.conf.json` | Window title/size, build commands, bundle settings |

## Rust, first look (from `lib.rs`)
```rust
#[tauri::command]                       // attribute macro: exposes the fn to the UI
fn greet(name: &str) -> String {        // &str = borrowed text; String = owned text
    format!("Hello, {}!", name)         // last expression, no `;` = return value
}
```
- `&str` vs `String`: borrowing vs owning. You'll see this everywhere.
- `tauri::generate_handler![greet]` registers the command; the UI calls `invoke("greet", { name })`.

## Commands
```
npm install          # once: frontend deps
npm run tauri dev    # runs Vite + compiles Rust + opens the window (first build is slow)
```
