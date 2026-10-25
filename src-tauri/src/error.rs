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

    /// Could not reach Jira at all (DNS, offline, timeout, TLS...).
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),

    /// Jira answered, but with an error. The text is already written for the user.
    #[error("{0}")]
    Jira(String),

    /// Missing or invalid settings (for example "no Jira URL saved yet").
    #[error("{0}")]
    Config(String),

    #[error("could not store the secret: {0}")]
    Secret(String),

    /// GitLab answered with an error (or couldn't be reached). Written for the user.
    #[error("{0}")]
    Gitlab(String),

    /// The local AI server was unreachable or answered badly. Written for the user.
    #[error("{0}")]
    Llm(String),
}

/// Tauri needs command errors to be serializable. The UI only needs the message,
/// so we send the error as its display string.
impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
