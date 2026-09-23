//! core 共通のエラー型。

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("request timed out")]
    Timeout,
    #[error("failed to parse feed: {0}")]
    Parse(String),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("invalid OPML: {0}")]
    Opml(String),
    #[error("{0} not found")]
    NotFound(String),
    #[error("already subscribed: {0}")]
    AlreadySubscribed(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        if e.is_timeout() {
            Error::Timeout
        } else {
            Error::Http(e.to_string())
        }
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
