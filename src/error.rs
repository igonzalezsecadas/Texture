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
}
