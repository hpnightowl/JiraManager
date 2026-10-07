use serde::{Serialize, Serializer};

/// One error type for the whole backend.
/// `thiserror` writes the `Display`/`Error` boilerplate from the `#[error(..)]` text.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// `#[from]` lets `?` convert a rusqlite error into this automatically.
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("{0}")]
    Invalid(String),

    #[error("No task with id {0}")]
    NotFound(i64),

    #[error("internal error: the database lock was poisoned")]
    Poisoned,
}

/// Tauri needs command errors to be serializable. The UI only needs the message,
/// so we send the error as its display string.
impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
