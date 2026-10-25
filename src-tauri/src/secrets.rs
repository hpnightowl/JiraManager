//! Where API tokens live. They never go in the SQLite database or in logs.
//!
//! 1. Preferred: the OS keychain (Windows Credential Manager, or the Linux "Secret Service"
//!    that GNOME Keyring / KWallet provide).
//! 2. Fallback: a file in the app-data folder that only your user can read (mode 0600 on
//!    Linux). Used when no keychain exists, e.g. inside WSL. The UI tells the user which one
//!    is in use, so this is never a silent downgrade.

use crate::error::AppError;
use std::path::PathBuf;

const SERVICE: &str = "com.projectjira.app";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Keychain,
    File,
}

impl Backend {
    pub fn label(self) -> &'static str {
        match self {
            Backend::Keychain => "OS keychain",
            Backend::File => "local file (no OS keychain available; protected by file permissions only)",
        }
    }
}

pub struct SecretStore {
    dir: PathBuf,
    use_keychain: bool,
}

impl SecretStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, use_keychain: true }
    }

    /// Never touches the OS keychain: used by tests so they don't depend on the machine.
    #[cfg(test)]
    pub fn file_only(dir: PathBuf) -> Self {
        Self { dir, use_keychain: false }
    }

    /// Only letters, digits, `-` and `_`: a name can never climb out of the app folder (`../x`).
    fn file_path(&self, name: &str) -> PathBuf {
        let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
        self.dir.join(format!("secret-{safe}"))
    }

    fn entry(name: &str) -> Result<keyring::Entry, keyring::Error> {
        keyring::Entry::new(SERVICE, name)
    }

    /// Save a secret; returns where it ended up.
    pub fn set(&self, name: &str, value: &str) -> Result<Backend, AppError> {
        if self.use_keychain {
            let stored = Self::entry(name).and_then(|e| e.set_password(value));
            if stored.is_ok() {
                let _ = std::fs::remove_file(self.file_path(name)); // drop any older file copy
                return Ok(Backend::Keychain);
            }
            // Any keychain failure (no D-Bus, locked, ...) falls through to the file.
        }
        self.write_file(name, value)?;
        Ok(Backend::File)
    }

    /// Read a secret, and say where it came from.
    pub fn get(&self, name: &str) -> Result<Option<(String, Backend)>, AppError> {
        if self.use_keychain {
            if let Ok(value) = Self::entry(name).and_then(|e| e.get_password()) {
                return Ok(Some((value, Backend::Keychain)));
            }
        }
        match std::fs::read_to_string(self.file_path(name)) {
            Ok(value) => Ok(Some((value, Backend::File))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(AppError::Secret(e.to_string())),
        }
    }

    pub fn delete(&self, name: &str) -> Result<(), AppError> {
        if self.use_keychain {
            if let Ok(entry) = Self::entry(name) {
                let _ = entry.delete_credential(); // "nothing to delete" is fine
            }
        }
        match std::fs::remove_file(self.file_path(name)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(AppError::Secret(e.to_string())),
        }
    }

    fn write_file(&self, name: &str, value: &str) -> Result<(), AppError> {
        use std::io::Write;
        let io = |e: std::io::Error| AppError::Secret(e.to_string());
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        let path = self.file_path(name);

        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600); // set at creation, so there is no moment where it is world-readable
        }
        let mut file = options.open(&path).map_err(io)?;
        #[cfg(unix)]
        {
            // `mode()` only applies when the file is created; an older, looser file is tightened too,
            // before the secret is written into it.
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600)).map_err(io)?;
        }
        file.write_all(value.as_bytes()).map_err(io)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pj-secrets-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn set_get_delete_roundtrip_via_file() {
        let dir = temp_dir("roundtrip");
        let store = SecretStore::file_only(dir.clone());
        assert!(store.get("jira").unwrap().is_none());
        assert_eq!(store.set("jira", "s3cret").unwrap(), Backend::File);
        assert_eq!(store.get("jira").unwrap(), Some(("s3cret".into(), Backend::File)));
        store.set("jira", "rotated").unwrap(); // overwriting replaces, not appends
        assert_eq!(store.get("jira").unwrap().unwrap().0, "rotated");
        store.delete("jira").unwrap();
        store.delete("jira").unwrap(); // deleting twice is fine
        assert!(store.get("jira").unwrap().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn secret_file_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("perms");
        let store = SecretStore::file_only(dir.clone());
        store.set("jira", "s3cret").unwrap();
        let mode = std::fs::metadata(store.file_path("jira")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_older_world_readable_file_is_tightened_when_the_secret_is_rewritten() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("loose");
        let store = SecretStore::file_only(dir.clone());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(store.file_path("jira"), "old").unwrap();
        std::fs::set_permissions(store.file_path("jira"), std::fs::Permissions::from_mode(0o644)).unwrap();
        store.set("jira", "new").unwrap();
        assert_eq!(std::fs::metadata(store.file_path("jira")).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_secret_name_cannot_point_outside_the_app_folder() {
        let dir = temp_dir("names");
        let store = SecretStore::file_only(dir.clone());
        let path = store.file_path("../../etc/evil");
        assert_eq!(path.parent().unwrap(), dir.as_path());
        assert!(!path.to_string_lossy().contains(".."));
        let _ = std::fs::remove_dir_all(dir);
    }
}
