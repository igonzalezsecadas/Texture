//! Error types. The parser has a dedicated error so a single bad hand can be
//! quarantined with a precise reason instead of aborting an import. Import and
//! web errors are recovered into `anyhow` for ergonomics.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ParseError {
    #[error("not a PokerStars hand history (missing 'PokerStars Hand #' header)")]
    NotPokerStars,
    #[error("unsupported game type: {0}")]
    UnsupportedGame(String),
    #[error("unsupported currency: {0}")]
    UnsupportedCurrency(String),
    #[error("hand is incomplete: {0}")]
    Incomplete(String),
    #[error("malformed {section}: {detail}")]
    Malformed { section: &'static str, detail: String },
    #[error("internal parse inconsistency: {0}")]
    Inconsistent(String),
}

impl ParseError {
    pub fn malformed(section: &'static str, detail: impl Into<String>) -> Self {
        ParseError::Malformed { section, detail: detail.into() }
    }
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("pool error: {0}")]
    Pool(#[from] r2d2::Error),
    #[error("migration failed at {version}: {source}")]
    Migration { version: u32, source: rusqlite::Error },
    #[error("not found: {0}")]
    NotFound(String),
    /// The data directory cannot be written by this process. This is almost
    /// always a host bind mount owned by root while the container runs as an
    /// unprivileged user, so the message says exactly how to fix it rather than
    /// leaving SQLite's bare `unable to open database file` on screen.
    #[error(
        "the data directory {path} is not writable by this process: {source}\n\
         hint: if texture runs in a container, that directory is bind-mounted from the host \
         and must be writable by the user inside the container.\n\
         On the host, make the data directory writable by the container's user, for example:\n\
         \x20   chown -R 1000:1000 <host data directory>\n\
         (uid 1000 is the user inside the official image; see compose.yaml for overriding it.)"
    )]
    DataDirUnwritable { path: std::path::PathBuf, source: std::io::Error },
}
