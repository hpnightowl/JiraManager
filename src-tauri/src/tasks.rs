use serde::Serialize;
use std::sync::Mutex;
use tauri::State;

/// One task in the list. `Serialize` lets serde turn it into JSON for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub done: bool,
}

/// The data we guard with a lock.
#[derive(Default)]
struct Inner {
    next_id: u64,
    tasks: Vec<Task>,
}

/// App-wide task storage (in memory for now; SQLite arrives in milestone 3).
/// `Mutex` makes it safe when several commands run on different threads.
#[derive(Default)]
pub struct TaskStore {
    inner: Mutex<Inner>,
}

impl TaskStore {
    pub fn add(&self, title: &str) -> Result<Task, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("Task title can't be empty".into());
        }
        let mut inner = self.inner.lock().map_err(|_| "task store is poisoned")?;
        inner.next_id += 1;
        let task = Task {
            id: inner.next_id,
            title: title.to_string(),
            done: false,
        };
        inner.tasks.push(task.clone());
        Ok(task)
    }

    pub fn list(&self) -> Result<Vec<Task>, String> {
        let inner = self.inner.lock().map_err(|_| "task store is poisoned")?;
        Ok(inner.tasks.clone())
    }

    pub fn toggle(&self, id: u64) -> Result<Task, String> {
        let mut inner = self.inner.lock().map_err(|_| "task store is poisoned")?;
        let task = inner
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| format!("No task with id {id}"))?;
        task.done = !task.done;
        Ok(task.clone())
    }

    pub fn remove(&self, id: u64) -> Result<(), String> {
        let mut inner = self.inner.lock().map_err(|_| "task store is poisoned")?;
        let before = inner.tasks.len();
        inner.tasks.retain(|t| t.id != id);
        if inner.tasks.len() == before {
            return Err(format!("No task with id {id}"));
        }
        Ok(())
    }
}

// --- Tauri commands: thin wrappers the UI can `invoke(...)` ---------------
// `State<TaskStore>` is how Tauri hands us the store registered with `.manage()`.

#[tauri::command]
pub fn add_task(title: String, store: State<'_, TaskStore>) -> Result<Task, String> {
    store.add(&title)
}

#[tauri::command]
pub fn list_tasks(store: State<'_, TaskStore>) -> Result<Vec<Task>, String> {
    store.list()
}

#[tauri::command]
pub fn toggle_task(id: u64, store: State<'_, TaskStore>) -> Result<Task, String> {
    store.toggle(id)
}

#[tauri::command]
pub fn delete_task(id: u64, store: State<'_, TaskStore>) -> Result<(), String> {
    store.remove(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_trims_and_assigns_increasing_ids() {
        let store = TaskStore::default();
        let a = store.add("  fix login  ").unwrap();
        let b = store.add("write tests").unwrap();
        assert_eq!(a.title, "fix login");
        assert_eq!((a.id, b.id), (1, 2));
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn add_rejects_blank_title() {
        let store = TaskStore::default();
        assert!(store.add("   ").is_err());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn toggle_flips_done_and_unknown_id_errors() {
        let store = TaskStore::default();
        let t = store.add("x").unwrap();
        assert!(store.toggle(t.id).unwrap().done);
        assert!(!store.toggle(t.id).unwrap().done);
        assert!(store.toggle(999).is_err());
    }

    #[test]
    fn remove_deletes_and_unknown_id_errors() {
        let store = TaskStore::default();
        let t = store.add("x").unwrap();
        store.remove(t.id).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert!(store.remove(t.id).is_err());
    }
}
