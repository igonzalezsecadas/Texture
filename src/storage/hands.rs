//! Hand, seat, raw-text, file-state, quarantine, and import-run persistence.
//!
//! All writes here go through [`Db::with_writer`]. Upserts are keyed on the
//! PokerStars hand id, which makes import idempotent.

use crate::domain::{HeroHandAnalysis, Position, ParsedHand};
use crate::error::StorageError;
use crate::storage::Db;
use crate::money::Currency;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    Inserted,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    pub path: String,
    pub size: i64,
    pub mtime_utc: i64,
    pub content_hash: String,
    pub parse_cursor: i64,
    pub last_imported: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineRow {
    pub id: i64,
    pub source_file: String,
    pub raw_text: String,
    pub reason: String,
    pub parser_version: u32,
    pub first_seen: i64,
    pub last_seen: i64,
    pub retry_count: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ImportRunStats {
    pub files_seen: i64,
    pub hands_imported: i64,
    pub duplicates: i64,
    pub failures: i64,
}

/// Insert or replace a parsed hand, its seats, its raw text, and (when the hero
/// was dealt in) its hero analysis row. Returns [`UpsertOutcome::Duplicate`] and
/// changes nothing when the hand id already exists.
pub fn upsert_hand(
    db: &Db,
    parsed: &ParsedHand,
    analysis: Option<&HeroHandAnalysis>,
    raw: &str,
    source_file: &str,
) -> Result<UpsertOutcome, StorageError> {
    let _ = (db, parsed, analysis, raw, source_file);
    unimplemented!("storage::hands::upsert_hand")
}

pub fn get_file_state(db: &Db, path: &str) -> Result<Option<FileState>, StorageError> {
    let _ = (db, path);
    unimplemented!("storage::hands::get_file_state")
}

pub fn put_file_state(db: &Db, state: &FileState) -> Result<(), StorageError> {
    let _ = (db, state);
    unimplemented!("storage::hands::put_file_state")
}

pub fn quarantine(
    db: &Db,
    source_file: &str,
    raw: &str,
    reason: &str,
    parser_version: u32,
) -> Result<(), StorageError> {
    let _ = (db, source_file, raw, reason, parser_version);
    unimplemented!("storage::hands::quarantine")
}

pub fn list_quarantine(db: &Db, limit: i64, offset: i64) -> Result<Vec<QuarantineRow>, StorageError> {
    let _ = (db, limit, offset);
    unimplemented!("storage::hands::list_quarantine")
}

pub fn quarantine_count(db: &Db) -> Result<i64, StorageError> {
    let _ = db;
    unimplemented!("storage::hands::quarantine_count")
}

pub fn delete_quarantine(db: &Db, id: i64) -> Result<(), StorageError> {
    let _ = (db, id);
    unimplemented!("storage::hands::delete_quarantine")
}

pub fn start_import_run(db: &Db, kind: &str) -> Result<i64, StorageError> {
    let _ = (db, kind);
    unimplemented!("storage::hands::start_import_run")
}

pub fn finish_import_run(db: &Db, id: i64, stats: &ImportRunStats) -> Result<(), StorageError> {
    let _ = (db, id, stats);
    unimplemented!("storage::hands::finish_import_run")
}

pub fn raw_hand(db: &Db, hand_id: &str) -> Result<Option<String>, StorageError> {
    let _ = (db, hand_id);
    unimplemented!("storage::hands::raw_hand")
}

/// Encode/decode the small fixed-length boolean arrays stored as text.
pub fn encode_bools<const N: usize>(v: &[bool; N]) -> String {
    v.iter().map(|b| if *b { '1' } else { '0' }).collect()
}

pub fn decode_bools<const N: usize>(s: &str, out: &mut [bool; N]) {
    for (i, b) in out.iter_mut().enumerate() {
        *b = s.as_bytes().get(i).map(|c| *c == b'1').unwrap_or(false);
    }
}

pub fn encode_u32s<const N: usize>(v: &[u32; N]) -> String {
    v.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(",")
}

pub fn decode_u32s<const N: usize>(s: &str, out: &mut [u32; N]) {
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = s.split(',').nth(i).and_then(|p| p.parse().ok()).unwrap_or(0);
    }
}

pub fn currency_code(c: Currency) -> &'static str {
    c.code()
}

pub fn ts_seconds(t: DateTime<Utc>) -> i64 {
    t.timestamp()
}

pub fn position_code(p: Position) -> &'static str {
    p.code()
}
