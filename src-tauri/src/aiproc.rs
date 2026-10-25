//! Lifecycle of the local AI server, so it doesn't keep running (and keep your memory) after you
//! close the app.
//!
//! The rules:
//! * Ollama that THIS APP started is stopped when you press "Stop AI" or close the app.
//! * Ollama that was already running (a background service you installed, or started yourself) is
//!   never killed: on "Stop AI" and on exit we only **unload its models from memory**. Stopping a
//!   service you set up is your decision (system tray / `systemctl`), not the app's.
//! * The app only ever starts Ollama on THIS computer (a loopback address), never a remote one.
//!
//! How it remembers what it started: the process handle in memory, AND the process id in a small
//! file (`ollama.pid` in the app-data folder). The file survives a crash or a forced close, so the
//! next launch can still stop an Ollama this app started earlier. A pid is only ever acted on after
//! checking the process really is `ollama` (process ids get reused). Every start/stop is also
//! written to `ai.log` in the same folder.

use crate::error::AppError;
use crate::llm::{client_from_settings, DEFAULT_URL};
use crate::tasks::TaskStore;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, State};

const KEY_AUTOSTART: &str = "llm.autostart";

/// The Ollama process this app started, if any.
#[derive(Default)]
pub struct AiProcess {
    child: Mutex<Option<Child>>,
    /// Where `ollama.pid` and `ai.log` live (None in unit tests that don't need them).
    dir: Mutex<Option<PathBuf>>,
}

impl AiProcess {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Child>> {
        self.child.lock().unwrap_or_else(|e| e.into_inner()) // a poisoned lock must not block shutdown
    }

    pub fn set_dir(&self, dir: PathBuf) {
        *self.dir.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir);
    }

    fn file(&self, name: &str) -> Option<PathBuf> {
        self.dir.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|d| d.join(name))
    }

    /// One line in `ai.log`: what happened and when. Failing to write it is never a problem.
    fn log(&self, message: &str) {
        let line = format!("{}  {message}\n", chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"));
        eprint!("ProjectJira: {message}\n");
        if let Some(path) = self.file("ai.log") {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = f.write_all(line.as_bytes());
            }
        }
    }

    /// The pid saved by this app (a previous run, or this one) IF that process is still alive and
    /// really is `ollama`. Anything else is stale and is forgotten.
    fn recorded_pid(&self) -> Option<u32> {
        let path = self.file("ollama.pid")?;
        let pid: u32 = std::fs::read_to_string(&path).ok()?.trim().parse().ok()?;
        if pid_alive(pid) && process_is_ollama(pid) {
            Some(pid)
        } else {
            let _ = std::fs::remove_file(path); // stale: the process is gone, or the pid belongs to something else now
            None
        }
    }

    /// Start `program args...` in the background (no window, no output) and remember it.
    pub fn start(&self, program: &str, args: &[&str]) -> Result<(), AppError> {
        if self.started_by_app() {
            return Ok(()); // ours is already running
        }
        let mut cmd = Command::new(program);
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AppError::Config("Ollama isn't installed (or isn't on your PATH). Install it from ollama.com first.".into())
            } else {
                AppError::Config(format!("Couldn't start Ollama: {e}"))
            }
        })?;
        let pid = child.id();
        if let Some(path) = self.file("ollama.pid") {
            let _ = std::fs::write(path, pid.to_string());
        }
        self.log(&format!("started Ollama (pid {pid})"));
        *self.lock() = Some(child);
        Ok(())
    }

    /// Did this app start an Ollama that is still alive? (Counts one started by an earlier run of the
    /// app that is still there, so it can still be cleaned up.)
    pub fn started_by_app(&self) -> bool {
        let own_child_alive = self.lock().as_mut().is_some_and(|c| c.try_wait().ok().flatten().is_none());
        own_child_alive || self.recorded_pid().is_some()
    }

    /// Stop the Ollama this app started, if any. Returns whether there was one. `reason` goes in the log.
    pub fn stop(&self, reason: &str) -> bool {
        let mut stopped = false;
        if let Some(mut child) = self.lock().take() {
            let pid = child.id();
            terminate(&mut child);
            self.log(&format!("stopped Ollama (pid {pid}) because: {reason}"));
            stopped = true;
        }
        // One started by an earlier run (or whose handle was lost): found through the pid file.
        if let Some(pid) = self.recorded_pid() {
            terminate_pid(pid);
            self.log(&format!("stopped Ollama (pid {pid}, from the pid file) because: {reason}"));
            stopped = true;
        }
        if let Some(path) = self.file("ollama.pid") {
            let _ = std::fs::remove_file(path);
        }
        if !stopped {
            self.log(&format!("nothing to stop ({reason}): this app has no Ollama of its own running"));
        }
        stopped
    }
}

/// Is a process with this id running?
fn pid_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&format!("\"{pid}\"")))
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        Command::new("kill").args(["-0", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }
}

/// Does this process id belong to a program called `ollama`? Checked before ever stopping a saved
/// pid, because ids are reused: a stale file must never make us stop an unrelated program.
fn process_is_ollama(pid: u32) -> bool {
    #[cfg(windows)]
    {
        Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"]).output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains("ollama"))
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        if let Ok(name) = std::fs::read_to_string(format!("/proc/{pid}/comm")) {
            return name.trim() == "ollama"; // Linux
        }
        Command::new("ps").args(["-p", &pid.to_string(), "-o", "comm="]).output() // macOS and others
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().ends_with("ollama"))
            .unwrap_or(false)
    }
}

/// Ask nicely, wait a few seconds, then force.
fn terminate_pid(pid: u32) {
    #[cfg(windows)]
    {
        // /T also ends the processes Ollama started (the model runner).
        let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !pid_alive(pid) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
}

/// Stop a process we hold a handle to: the same polite-then-forced stop, and reap it afterwards.
fn terminate(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return; // already gone
    }
    let pid = child.id();
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Is the `ollama` program installed? (`ollama --version` runs.)
pub fn ollama_installed() -> bool {
    let mut cmd = Command::new("ollama");
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.status().is_ok_and(|s| s.success())
}

/// Starting only makes sense for a server on this computer.
fn is_loopback(base_url: &str) -> bool {
    let rest = base_url.split("://").nth(1).unwrap_or(base_url);
    let authority = rest.split('/').next().unwrap_or(rest); // "host:port", "[::1]:port" or "host"
    let host = match authority.strip_prefix('[') {
        Some(inside) => inside.split(']').next().unwrap_or(""),
        None => authority.rsplit_once(':').map_or(authority, |(h, _)| h),
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn autostart_enabled(store: &TaskStore) -> bool {
    store.get_setting(KEY_AUTOSTART).ok().flatten().map_or(true, |v| v != "0") // on unless switched off
}

#[derive(Debug, Serialize)]
pub struct AiStatus {
    /// The `ollama` program exists on this computer.
    pub installed: bool,
    /// Something answers at the saved address.
    pub running: bool,
    /// This app started it, so this app will stop it.
    pub started_by_app: bool,
    /// Models currently held in memory.
    pub loaded_models: Vec<String>,
    /// Start Ollama automatically when the app opens.
    pub autostart: bool,
    /// The saved address is on this computer, so "Start AI" is possible.
    pub local: bool,
}

async fn status_of(store: &TaskStore, ai: &AiProcess) -> Result<AiStatus, AppError> {
    let client = client_from_settings(store)?;
    let running = client.is_up().await;
    let base = store.get_setting("llm.base_url")?.unwrap_or_else(|| DEFAULT_URL.to_string());
    Ok(AiStatus {
        installed: ollama_installed(),
        running,
        started_by_app: running && ai.started_by_app(),
        loaded_models: if running { client.loaded_models().await.unwrap_or_default() } else { vec![] },
        autostart: autostart_enabled(store),
        local: is_loopback(&base),
    })
}

/// Start Ollama (if it is installed and not already answering) and wait until it is ready.
pub async fn start_ai_impl(store: &TaskStore, ai: &AiProcess) -> Result<AiStatus, AppError> {
    let client = client_from_settings(store)?;
    if !client.is_up().await {
        let base = store.get_setting("llm.base_url")?.unwrap_or_else(|| DEFAULT_URL.to_string());
        if !is_loopback(&base) {
            return Err(AppError::Config(format!("The AI address {base} isn't on this computer, so the app can't start it. Start it on that machine.")));
        }
        ai.start("ollama", &["serve"])?;
        // Wait up to 20 s for it to answer.
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && !client.is_up().await {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        if !client.is_up().await {
            ai.stop("it didn't answer after starting");
            return Err(AppError::Llm("Ollama was started but isn't answering. Try running `ollama serve` in a terminal to see why.".into()));
        }
    }
    status_of(store, ai).await
}

/// Free the AI's memory, and stop the server too if this app started it.
pub async fn stop_ai_impl(store: &TaskStore, ai: &AiProcess) -> Result<(String, AiStatus), AppError> {
    let client = client_from_settings(store)?;
    let unloaded = if client.is_up().await { client.unload_all().await.unwrap_or(0) } else { 0 };
    let stopped_server = ai.stop("Stop AI pressed");
    let message = match (stopped_server, unloaded) {
        (true, _) => "The AI was stopped and its memory freed.".to_string(),
        (false, n) if n > 0 => format!(
            "Unloaded {n} model(s) from memory. Ollama itself keeps running because you started it yourself (or it's a background service); \
             quit it from your system tray, or with `sudo systemctl stop ollama`, to stop it completely."
        ),
        (false, _) => "Nothing was loaded in memory.".to_string(),
    };
    Ok((message, status_of(store, ai).await?))
}

/// Called when the app closes: free the model's memory and stop the Ollama this app started.
pub fn shutdown(app: &AppHandle) {
    let (Some(ai), Some(store)) = (app.try_state::<AiProcess>(), app.try_state::<TaskStore>()) else { return };
    if let Ok(client) = client_from_settings(&store) {
        // Best-effort and time-boxed: closing must never hang because the AI is slow.
        tauri::async_runtime::block_on(async {
            let _ = tokio::time::timeout(Duration::from_secs(4), client.unload_all()).await;
        });
    }
    ai.stop("the app is closing");
}

/// Called once when the app opens: start Ollama in the background if the user wants that.
pub fn autostart(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let (Some(ai), Some(store)) = (handle.try_state::<AiProcess>(), handle.try_state::<TaskStore>()) else { return };
        if autostart_enabled(&store) && ollama_installed() {
            let _ = start_ai_impl(&store, &ai).await; // a failure here just means "AI off"; Setup shows why
        }
    });
}

// --- commands ---------------------------------------------------------------------------

#[tauri::command]
pub async fn ai_status(store: State<'_, TaskStore>, ai: State<'_, AiProcess>) -> Result<AiStatus, AppError> {
    status_of(&store, &ai).await
}

#[tauri::command]
pub async fn start_ai(store: State<'_, TaskStore>, ai: State<'_, AiProcess>) -> Result<AiStatus, AppError> {
    start_ai_impl(&store, &ai).await
}

/// Returns a sentence to show the user, plus the new status.
#[tauri::command]
pub async fn stop_ai(store: State<'_, TaskStore>, ai: State<'_, AiProcess>) -> Result<(String, AiStatus), AppError> {
    stop_ai_impl(&store, &ai).await
}

#[tauri::command]
pub fn set_ai_autostart(on: bool, store: State<'_, TaskStore>) -> Result<(), AppError> {
    store.set_setting(KEY_AUTOSTART, if on { "1" } else { "0" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pj-aiproc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A program literally NAMED `ollama` that just sleeps: a stand-in for `ollama serve`
    /// (the process name comes from the file name, which is what the safety check looks at).
    #[cfg(unix)]
    fn fake_ollama(dir: &std::path::Path) -> String {
        let link = dir.join("ollama");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink("/bin/sleep", &link).or_else(|_| std::os::unix::fs::symlink("/usr/bin/sleep", &link)).unwrap();
        link.to_string_lossy().to_string()
    }

    #[test]
    fn only_addresses_on_this_computer_can_be_started() {
        for local in ["http://127.0.0.1:11434", "http://localhost:11434", "http://[::1]:11434", "http://127.0.0.1"] {
            assert!(is_loopback(local), "{local}");
        }
        for remote in ["http://192.168.1.20:11434", "http://desktop:11434", "http://10.0.0.5:11434"] {
            assert!(!is_loopback(remote), "{remote}");
        }
    }

    #[test]
    fn autostart_is_on_until_switched_off() {
        let store = TaskStore::in_memory();
        assert!(autostart_enabled(&store));
        store.set_setting(KEY_AUTOSTART, "0").unwrap();
        assert!(!autostart_enabled(&store));
        store.set_setting(KEY_AUTOSTART, "1").unwrap();
        assert!(autostart_enabled(&store));
    }

    #[test]
    fn a_missing_program_is_explained() {
        let ai = AiProcess::default();
        let err = ai.start("definitely-not-an-installed-program", &[]).unwrap_err().to_string();
        assert!(err.contains("isn't installed"), "got: {err}");
        assert!(!ai.started_by_app());
        assert!(!ai.stop("test")); // nothing to stop
    }

    #[cfg(unix)]
    #[test]
    fn a_process_the_app_started_is_stopped_promptly_and_logged() {
        let dir = temp_dir("own");
        let ai = AiProcess::default();
        ai.set_dir(dir.clone());
        let program = fake_ollama(&dir);

        ai.start(&program, &["60"]).unwrap();
        assert!(ai.started_by_app());
        let pid: u32 = std::fs::read_to_string(dir.join("ollama.pid")).unwrap().trim().parse().unwrap();
        ai.start(&program, &["60"]).unwrap(); // already running: no second process
        assert_eq!(std::fs::read_to_string(dir.join("ollama.pid")).unwrap().trim(), pid.to_string());

        let started = Instant::now();
        assert!(ai.stop("test"));
        assert!(started.elapsed() < Duration::from_secs(5), "should stop promptly after a polite request");
        assert!(!pid_alive(pid) || !process_is_ollama(pid)); // gone (or at worst a reaped zombie)
        assert!(!dir.join("ollama.pid").exists());
        assert!(!ai.started_by_app() && !ai.stop("again"));

        let log = std::fs::read_to_string(dir.join("ai.log")).unwrap();
        assert!(log.contains("started Ollama") && log.contains("stopped Ollama") && log.contains("because: test"), "log: {log}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_ollama_left_behind_by_a_previous_run_is_found_and_stopped_through_the_pid_file() {
        let dir = temp_dir("orphan");
        let program = fake_ollama(&dir);

        // "Previous run": a separate shell starts Ollama and writes its pid, then exits: the app is
        // gone without cleaning up, and Ollama is left running with nobody holding its handle.
        let pid_file = dir.join("ollama.pid");
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!("'{program}' 60 >/dev/null 2>&1 & echo $! > '{}'", pid_file.display()))
            .status()
            .unwrap();
        assert!(status.success());
        let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
        assert!(pid_alive(pid) && process_is_ollama(pid), "the leftover is running");

        // "Next run": a brand new AiProcess with no memory of it, but the same folder.
        let second = AiProcess::default();
        second.set_dir(dir.clone());
        assert!(second.started_by_app(), "the leftover should be recognised as ours");
        assert!(second.stop("test: next launch"));

        // Gone (a dead process the system hasn't tidied away yet shows state Z; that counts as gone).
        let is_gone = |pid: u32| {
            !pid_alive(pid)
                || std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |s| s.rsplit(") ").next().is_some_and(|rest| rest.starts_with('Z')))
        };
        let deadline = Instant::now() + Duration::from_secs(6);
        while !is_gone(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(is_gone(pid), "the leftover Ollama should be gone");
        assert!(!pid_file.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_stale_pid_that_belongs_to_another_program_is_never_touched() {
        let dir = temp_dir("stale");
        // A live process that is NOT called ollama, with its pid written into the pid file.
        let mut innocent = Command::new("sleep").arg("60").spawn().unwrap();
        std::fs::write(dir.join("ollama.pid"), innocent.id().to_string()).unwrap();

        let ai = AiProcess::default();
        ai.set_dir(dir.clone());
        assert!(!ai.started_by_app(), "not ollama, so not ours");
        assert!(!ai.stop("test"));
        assert!(innocent.try_wait().unwrap().is_none(), "the unrelated program must still be running");
        assert!(!dir.join("ollama.pid").exists(), "the stale file is forgotten");

        let _ = innocent.kill();
        let _ = innocent.wait();
        let _ = std::fs::remove_dir_all(dir);
    }

    async fn store_pointing_at(server: &MockServer) -> TaskStore {
        let store = TaskStore::in_memory();
        store.set_setting("llm.base_url", &server.uri()).unwrap();
        store.set_setting("llm.model", "m").unwrap();
        store
    }

    #[tokio::test]
    async fn stop_unloads_models_but_leaves_a_server_it_did_not_start_running() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/version")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "version": "0.1" }))).mount(&server).await;
        Mock::given(method("GET")).and(path("/api/ps")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [{ "name": "qwen2.5:3b" }] }))).mount(&server).await;
        Mock::given(method("POST")).and(path("/api/generate")).respond_with(ResponseTemplate::new(200).set_body_json(json!({}))).mount(&server).await;
        let store = store_pointing_at(&server).await;
        let ai = AiProcess::default();

        let status = status_of(&store, &ai).await.unwrap();
        assert!(status.running && !status.started_by_app && status.loaded_models == ["qwen2.5:3b"]);

        let (message, _) = stop_ai_impl(&store, &ai).await.unwrap();
        assert!(message.contains("Unloaded 1 model") && message.contains("keeps running"), "got: {message}");
        let generate_calls = server.received_requests().await.unwrap().iter().filter(|r| r.url.path() == "/api/generate").count();
        assert_eq!(generate_calls, 1);
    }

    #[tokio::test]
    async fn a_remote_address_is_never_started_by_the_app() {
        let store = TaskStore::in_memory();
        store.set_setting("llm.base_url", "http://192.168.1.50:11434").unwrap();
        let err = start_ai_impl(&store, &AiProcess::default()).await.unwrap_err().to_string();
        assert!(err.contains("isn't on this computer"), "got: {err}");
    }

    #[tokio::test]
    async fn an_ai_that_is_already_up_is_not_started_twice() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/version")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "version": "0.1" }))).mount(&server).await;
        Mock::given(method("GET")).and(path("/api/ps")).respond_with(ResponseTemplate::new(200).set_body_json(json!({ "models": [] }))).mount(&server).await;
        let store = store_pointing_at(&server).await;
        let ai = AiProcess::default();
        let status = start_ai_impl(&store, &ai).await.unwrap();
        assert!(status.running && !status.started_by_app); // it was already there: not ours to stop
    }
}
