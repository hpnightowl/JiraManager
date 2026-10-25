# Running and packaging ProjectJira

## Cheat sheet (inside Debian, from `~/ProjectJira`)
| I want to... | Command |
|---|---|
| Start the app I already built | `scripts/run.sh` |
| Develop (rebuilds as I edit) | `scripts/dev.sh` |
| Check everything (types, tests, build) | `scripts/test.sh` |
| Compile the optimized app + `.deb` | `scripts/build.sh` (add `--install` to install it) |

From Windows, double-click in `Documents\dotfiles\wsl\`: `ProjectJira.cmd` (start), `Build-ProjectJira.cmd`, `Test-ProjectJira.cmd`.
Prerequisites on a fresh machine: `bash <dotfiles>/wsl/setup-debian-dev.sh` (Rust, Node, the Linux libraries).

## Run it now (during testing)
Inside Debian (WSL). The window appears on your Windows desktop.

```bash
cd ~/ProjectJira
npm run tauri dev          # development: rebuilds when you change code
```
or the built app:
```bash
~/ProjectJira/src-tauri/target/release/project-jira
```
or, after installing the launcher once, from Windows just double-click `Documents\dotfiles\wsl\ProjectJira.cmd`:
```bash
bash /mnt/c/Users/<you>/Documents/dotfiles/wsl/install-projectjira.sh     # one time, inside Debian
```

## The tabs
**Tasks** (the working screen; groups start collapsed), **Stories** (how many stories you have created, and
"I need N stories" suggestions), **Setup** (Jira, local AI, git sources, with a checklist) and **How to use**
(a guide inside the app). The Setup tab opens by itself until Jira is connected, and shows a "!" on the tab.

## First run: the Setup tab
1. **Jira**: site address, email, API token.
2. **Local AI (optional)**: the checklist checks **Ollama**. If it isn't installed, install it from ollama.com (Windows: the
   installer; Linux: `curl -fsSL https://ollama.com/install.sh | sh`) and come back (it re-checks when the window regains
   focus). If it is installed but not running, press **Start AI**. Then press **Download <model>** (asks for a second
   click, about 1.9 GB); a progress bar follows. The model comes from the Ollama library through *your* Ollama; the app
   never installs Ollama for you.
3. **Git sources (optional)**: GitLab servers (one token each) and repository folders.

Without step 2 the app still works fully; "Improve with AI" offers a simple template instead.

## The AI's lifecycle (it should not keep running after you close the app)
- Setup → Local AI has **Start AI** and **Stop AI**. *Stop AI* frees the model's memory, and stops Ollama too if **this app
  started it**.
- By default the app starts Ollama when it opens (if installed) and stops it when you close the app. Untick the box in the
  same card to turn that off.
- Ollama that was already running (a service, or one you started yourself in a terminal) is **never killed** by the app.
  Closing the app only unloads its models from memory. To stop such an Ollama completely: quit it from the system tray, or
  `sudo systemctl stop ollama` (or close the terminal running `ollama serve`).
- The app only ever *starts* Ollama on this computer, never at a remote address.

## Build the Linux package
```bash
cd ~/ProjectJira
npm run tauri build -- --bundles deb
# result: src-tauri/target/release/bundle/deb/ProjectJira_0.1.0_amd64.deb
#         src-tauri/target/release/project-jira   (the bare binary)
sudo apt install ./src-tauri/target/release/bundle/deb/ProjectJira_0.1.0_amd64.deb
```
Other formats (`--bundles appimage`, `rpm`) work too; AppImage downloads its own packaging tool during the build.

## A Windows .exe/.msi (not built yet)
Windows installers must be built **on Windows** with the C++ build tools. On this PC that is blocked by
**Smart App Control**, which also refuses to *run* an unsigned `.exe`. The options:
1. Keep running the Linux build inside WSL (what you do now).
2. Turn Smart App Control off (irreversible without reinstalling Windows; your decision), then build with
   `npm run tauri build` on Windows.
3. Build in CI (GitLab or GitHub runners) and **code-sign** the result with a certificate. Signing is what
   lets Smart App Control accept it.

## Data and secrets (where things live)
| What | Where |
|---|---|
| Tasks, settings, merge requests, story drafts | `~/.local/share/com.projectjira.app/projectjira.db` (Windows: `%APPDATA%\com.projectjira.app\`) |
| Jira and GitLab tokens | the OS keychain; if none exists (WSL) a private file `secret-<name>` in the same folder |
| Ollama models | Ollama's own folder (not this app's) |
