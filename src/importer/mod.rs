//! Import pipeline. Bulk import and continuous import share this one code path
//! so their behavior cannot diverge.
//!
//! Behavior contract:
//! - A file is skipped when its `(size, mtime)` and content hash are unchanged.
//! - Only hands terminated by a summary section are imported; a trailing
//!   incomplete hand is held until the file grows.
//! - A hand that fails to parse is quarantined with a reason and never stops the
//!   file.
//! - Upserts are keyed on hand id, so re-reading is harmless.

use crate::config::Config;
use crate::storage::Db;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct ImportProgress {
    pub files_done: u64,
    pub files_total: u64,
    pub hands_imported: u64,
    pub duplicates: u64,
    pub failures: u64,
    pub quarantined: u64,
    pub current_file: Option<String>,
    pub finished: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ImportOutcome {
    pub files_seen: u64,
    pub hands_imported: u64,
    pub duplicates: u64,
    pub failures: u64,
    pub quarantined: u64,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// Force a re-read of files already in `file_state`.
    pub force: bool,
    /// Report progress through this callback. Called often; keep it cheap.
    pub quiet: bool,
}

/// Bulk-import every hand-history file under `dir`.
pub fn import_directory(
    db: &Db,
    dir: &Path,
    opts: &ImportOptions,
    progress: &mut dyn FnMut(&ImportProgress),
) -> anyhow::Result<ImportOutcome> {
    let _ = (db, dir, opts, progress);
    unimplemented!("importer::import_directory")
}

/// Reparse every retained raw hand with the current parser, rebuilding parsed
/// rows. Resumable: hands already at the current parser version are skipped
/// unless `force` is set.
pub fn reparse(
    db: &Db,
    force: bool,
    progress: &mut dyn FnMut(&ImportProgress),
) -> anyhow::Result<ImportOutcome> {
    let _ = (db, force, progress);
    unimplemented!("importer::reparse")
}

/// Import exactly one file. Used by the watcher and by bulk import.
pub fn import_file(
    db: &Db,
    path: &Path,
    opts: &ImportOptions,
    progress: &mut dyn FnMut(&ImportProgress),
) -> anyhow::Result<ImportOutcome> {
    let _ = (db, path, opts, progress);
    unimplemented!("importer::import_file")
}

/// Detect the hero's screen name from a PokerStars hand-history folder name.
/// PokerStars folder names look like `hero.name12345`, so the screen name is the
/// folder name up to the numeric account suffix.
pub fn detect_hero_name(dir: &Path) -> Option<String> {
    let name = dir.file_name()?.to_str()?;
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit());
    let trimmed = trimmed.trim_end_matches('.');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Run the continuous watcher until `shutdown` flips to `true`. Reacts to file
/// creation/modification events and re-scans on startup; falls back to polling
/// every `cfg.scan_interval_secs` for filesystems that do not report events.
pub async fn run_watcher(
    db: Db,
    cfg: Config,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let _ = (db, cfg, shutdown);
    unimplemented!("importer::run_watcher")
}

/// All hand-history files under `dir`, sorted for deterministic processing.
pub fn list_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if is_hand_history_file(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

pub fn is_hand_history_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("txt")
    )
}
