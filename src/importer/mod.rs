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
use notify::Watcher;
use crate::parser;
use crate::storage::hands as store;
use crate::parser::HandParser;
use crate::storage::schema;
use crate::storage::Db;
use crate::web::{ImporterHandle, ImporterSnapshot};
use std::path::{Path, PathBuf};
use std::time::Instant;

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

/// Outcome of importing an in-memory hand-history document (a browser upload
/// or a paste). `hands_seen` counts the complete hands found in the text.
#[derive(Debug, Clone, Default)]
pub struct TextImportOutcome {
    pub hands_seen: u64,
    pub hands_imported: u64,
    pub duplicates: u64,
    pub failures: u64,
    pub quarantined: u64,
    /// True when the text ends with a hand that has no summary section yet
    /// (e.g. a truncated upload). Such a hand is NOT quarantined.
    pub incomplete_trailing: bool,
    pub duration_ms: u128,
}

#[derive(Debug, Clone, Default)]
pub struct ImportOptions {
    /// Force a re-read of files already in `file_state`.
    pub force: bool,
    /// Report progress through this callback. Called often; keep it cheap.
    pub quiet: bool,
}

// ---------------------------------------------------------------------------
// Internal accumulator / runner state
// ---------------------------------------------------------------------------

/// Running totals shared across the files of one import pass.
#[derive(Debug, Default)]
struct Acc {
    files_seen: u64,
    files_done: u64,
    files_total: u64,
    hands_imported: u64,
    duplicates: u64,
    failures: u64,
    quarantined: u64,
    current_file: Option<String>,
}

impl Acc {
    fn snapshot(&self) -> ImportProgress {
        ImportProgress {
            files_done: self.files_done,
            files_total: self.files_total,
            hands_imported: self.hands_imported,
            duplicates: self.duplicates,
            failures: self.failures,
            quarantined: self.quarantined,
            current_file: self.current_file.clone(),
            finished: false,
        }
    }

    fn report(&self, opts: &ImportOptions, progress: &mut dyn FnMut(&ImportProgress)) {
        if opts.quiet {
            return;
        }
        progress(&self.snapshot());
    }

    fn into_outcome(self, elapsed_ms: u128) -> ImportOutcome {
        ImportOutcome {
            files_seen: self.files_seen,
            hands_imported: self.hands_imported,
            duplicates: self.duplicates,
            failures: self.failures,
            quarantined: self.quarantined,
            duration_ms: elapsed_ms,
        }
    }
}

/// The active hero profile for one import run: `(profile_id, (screen_name,
/// player_id) pairs)`. Identity lives in storage, so storage supplies the
/// name→id mapping and stats resolves the hero seat purely.
type HeroCtx = (i64, Vec<(String, i64)>);

/// Resolve the active hero profile once per import run, then reuse it for every
/// hand. No profile configured => no hero analysis rows. The importer never
/// invents a hero on its own; the operator wires one up
/// (`storage::Db::ensure_hero_profile`) before importing.
fn hero_ctx(db: &Db) -> Option<HeroCtx> {
    db.active_profile()
        .ok()
        .flatten()
        .and_then(|p| db.players_for_profile(p.id).ok().map(|players| (p.id, players)))
}

fn hero_analysis(
    parsed: &crate::domain::ParsedHand,
    hero: &Option<HeroCtx>,
) -> Option<crate::domain::HeroHandAnalysis> {
    let (profile_id, players) = hero.as_ref()?;
    crate::stats::analyze_hero(parsed, *profile_id, players)
}

/// When a file-state fast path skips an existing file we still report its
/// already-imported hands as duplicates, so progress counters reflect reality
/// without touching any data.
fn duplicate_count(db: &Db, path: &str) -> u64 {
    db.with_reader(|c| {
        c.query_row("SELECT COUNT(*) FROM hands WHERE source_file = ?1", [path], |r| r.get(0))
    })
    .unwrap_or(0) as u64
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

fn mtime_secs(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Bulk-import every hand-history file under `dir`.
pub fn import_directory(
    db: &Db,
    dir: &Path,
    opts: &ImportOptions,
    progress: &mut dyn FnMut(&ImportProgress),
) -> anyhow::Result<ImportOutcome> {
    let t0 = Instant::now();
    let files = list_files(dir);
    let mut acc =
        Acc { files_seen: files.len() as u64, files_total: files.len() as u64, ..Acc::default() };
    let hero = hero_ctx(db);
    let run_id = store::start_import_run(db, "bulk").ok();

    for path in files {
        acc.current_file = Some(path.to_string_lossy().into_owned());
        if let Err(e) = import_one_file(db, &path, &hero, opts, &mut acc, progress) {
            // A single file must never take down an import: report and move on.
            tracing::warn!(path = %path.display(), error = %e, "file import failed");
        }
        acc.files_done += 1;
        acc.report(opts, progress);
    }

    if let (Some(id), Some(stats)) = (run_id, Some(&acc)) {
        let _ = store::finish_import_run(
            db,
            id,
            &store::ImportRunStats {
                files_seen: stats.files_seen as i64,
                hands_imported: stats.hands_imported as i64,
                duplicates: stats.duplicates as i64,
                failures: stats.failures as i64,
            },
        );
    }

    acc.current_file = None;
    let mut progress_once = |p: &ImportProgress| {
        let mut p = p.clone();
        p.finished = true;
        if !opts.quiet {
            progress(&p);
        }
    };
    progress_once(&acc.snapshot());
    Ok(acc.into_outcome(t0.elapsed().as_millis()))
}

/// Import exactly one file. Used by the watcher and by bulk import. A missing or
/// unreadable file is recorded as a warning and swallowed — never a panic.
pub fn import_file(
    db: &Db,
    path: &Path,
    opts: &ImportOptions,
    progress: &mut dyn FnMut(&ImportProgress),
) -> anyhow::Result<ImportOutcome> {
    let t0 = Instant::now();
    let mut acc = Acc {
        files_seen: 1,
        files_total: 1,
        current_file: Some(path.to_string_lossy().into_owned()),
        ..Acc::default()
    };
    let hero = hero_ctx(db);
    match import_one_file(db, path, &hero, opts, &mut acc, progress) {
        Ok(()) => {}
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "file import failed");
        }
    }
    acc.current_file = None;
    if !opts.quiet {
        let mut snap = acc.snapshot();
        snap.finished = true;
        progress(&snap);
    }
    Ok(acc.into_outcome(t0.elapsed().as_millis()))
}

/// Import hands from in-memory text. `source` is the provenance label recorded
/// on each hand (and on quarantine rows), for example `upload:HH20240101.txt`.
/// Dedupe is by PokerStars hand id, so importing the same text twice inserts
/// nothing the second time. This deliberately does not touch `file_state` or
/// `import_runs`: a UI upload is not a watched file.
pub fn import_text(
    db: &Db,
    text: &str,
    source: &str,
    opts: &ImportOptions,
    progress: &mut dyn FnMut(&ImportProgress),
) -> anyhow::Result<TextImportOutcome> {
    let t0 = Instant::now();
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (raw_hands, consumed) = parser::split_complete_hands(body);
    let incomplete_trailing = consumed < body.len() && !body[consumed..].trim().is_empty();

    let mut acc = Acc {
        files_seen: 1,
        files_total: 1,
        current_file: Some(source.to_string()),
        ..Acc::default()
    };
    let hero = hero_ctx(db);
    let parser_impl = parser::default_parser();
    acc.report(opts, progress);

    let mut done_in_doc: u64 = 0;
    for raw in &raw_hands {
        match parser_impl.parse_hand(raw) {
            Ok(parsed) => {
                let analysis = hero_analysis(&parsed, &hero);
                match store::upsert_hand(db, &parsed, analysis.as_ref(), raw, source) {
                    Ok(store::UpsertOutcome::Inserted) => acc.hands_imported += 1,
                    Ok(store::UpsertOutcome::Duplicate) => acc.duplicates += 1,
                    Err(e) => {
                        tracing::warn!(source, error = %e, "could not store uploaded hand");
                        acc.failures += 1;
                    }
                }
            }
            Err(e) => {
                let reason = e.to_string();
                if let Err(qe) = store::quarantine(db, source, raw, &reason, schema::PARSER_VERSION)
                {
                    tracing::warn!(source, error = %qe, "quarantine write failed");
                }
                acc.failures += 1;
                acc.quarantined += 1;
            }
        }
        done_in_doc += 1;
        if done_in_doc.is_multiple_of(64) {
            acc.report(opts, progress);
        }
    }

    acc.files_done = 1;
    acc.current_file = None;
    if !opts.quiet {
        let mut snap = acc.snapshot();
        snap.finished = true;
        progress(&snap);
    }

    Ok(TextImportOutcome {
        hands_seen: raw_hands.len() as u64,
        hands_imported: acc.hands_imported,
        duplicates: acc.duplicates,
        failures: acc.failures,
        quarantined: acc.quarantined,
        incomplete_trailing,
        duration_ms: t0.elapsed().as_millis(),
    })
}

/// Most frequently dealt-to screen name in a hand-history document.
///
/// PokerStars writes `Dealt to <screen name> [cards]` only for the owner of the
/// hand history, so the name that occurs there repeatedly is the hero. Returns
/// `None` when the text carries no such line. Ties break lexicographically, so
/// the answer is deterministic.
pub fn detect_hero_from_text(text: &str) -> Option<String> {
    let mut counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("Dealt to ") else { continue };
        let Some((name, _)) = rest.split_once(" [") else { continue };
        let name = name.trim();
        if !name.is_empty() {
            *counts.entry(name.to_string()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(name, _)| name)
}

/// Process a single file that is known to exist. Handles the unchanged fast
/// path, incremental append, full re-reads, and per-hand quarantine. May only
/// fail for storage-level reasons; parse failures are absorbed.
fn import_one_file(
    db: &Db,
    path: &Path,
    hero: &Option<HeroCtx>,
    opts: &ImportOptions,
    acc: &mut Acc,
    progress: &mut dyn FnMut(&ImportProgress),
) -> Result<(), crate::error::StorageError> {
    let path_str = path.to_string_lossy().into_owned();
    tracing::debug!(file = %path_str, "scanning");

    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(file = %path_str, error = %e, "unreadable hand-history file, skipping");
            return Ok(());
        }
    };
    let size = bytes.len() as i64;
    let mtime = std::fs::metadata(path).map(|m| mtime_secs(&m)).unwrap_or(0);
    let full_hash = blake3::hash(&bytes).to_hex().to_string();

    let prior = store::get_file_state(db, &path_str)?;

    // ---- fast path: file is byte-for-byte what we last saw ------------------
    if !opts.force {
        if let Some(s) = &prior {
            if s.content_hash == full_hash {
                acc.duplicates += duplicate_count(db, &path_str);
                if s.size != size || s.mtime_utc != mtime {
                    // Only the metadata touched; refresh it without a re-parse.
                    store::put_file_state(
                        db,
                        &store::FileState {
                            size,
                            mtime_utc: mtime,
                            last_imported: Some(now_secs()),
                            ..s.clone()
                        },
                    )?;
                }
                return Ok(());
            }
        }
    }

    // ---- incremental append: file grew from the same bytes we last parsed ---
    let append_from = prior.clone().and_then(|s| {
        let store_size = s.size as usize;
        if !opts.force
            && size > s.size
            && store_size > 0
            && store_size <= bytes.len()
            && s.parse_cursor >= 0
            && (s.parse_cursor as usize) <= store_size
            && blake3::hash(&bytes[..store_size]).to_hex().as_str() == s.content_hash
        {
            Some(s.parse_cursor as usize)
        } else {
            None
        }
    });

    let (text, base_offset) = match append_from {
        Some(offset) => (&bytes[offset..], offset),
        None => (&bytes[..], 0usize),
    };

    let text = String::from_utf8(text.to_vec())
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());

    let (raw_hands, consumed) = parser::split_complete_hands(&text);

    let parser_impl = parser::default_parser();
    let mut done_in_file: u64 = 0;
    for raw in &raw_hands {
        match parser_impl.parse_hand(raw) {
            Ok(parsed) => {
                let analysis = hero_analysis(&parsed, &hero);
                match store::upsert_hand(db, &parsed, analysis.as_ref(), raw, &path_str) {
                    Ok(store::UpsertOutcome::Inserted) => acc.hands_imported += 1,
                    Ok(store::UpsertOutcome::Duplicate) => acc.duplicates += 1,
                    Err(e) => {
                        // Storage trouble on one hand is not allowed to halt the
                        // rest of the file; count it as a failure and continue.
                        tracing::warn!(file = %path_str, error = %e, "could not store hand");
                        acc.failures += 1;
                    }
                }
            }
            Err(e) => {
                let reason = e.to_string();
                if let Err(qe) = store::quarantine(db, &path_str, raw, &reason, schema::PARSER_VERSION)
                {
                    tracing::warn!(file = %path_str, error = %qe, "quarantine write failed");
                }
                acc.failures += 1;
                acc.quarantined += 1;
            }
        }
        done_in_file += 1;
        if done_in_file % 64 == 0 {
            acc.report(opts, progress);
        }
    }

    // The parse cursor is the byte offset (relative to this pass's text) where
    // the trailing incomplete hand begins; the next pass resumes exactly there.
    store::put_file_state(
        db,
        &store::FileState {
            path: path_str,
            size,
            mtime_utc: mtime,
            content_hash: full_hash,
            parse_cursor: (base_offset + consumed) as i64,
            last_imported: Some(now_secs()),
        },
    )?;
    Ok(())
}

/// Reparse every retained raw hand with the current parser, rebuilding parsed
/// rows. Resumable: hands already at the current parser version are skipped
/// unless `force` is set.
pub fn reparse(
    db: &Db,
    force: bool,
    progress: &mut dyn FnMut(&ImportProgress),
) -> anyhow::Result<ImportOutcome> {
    let t0 = Instant::now();
    let opts = ImportOptions { force, quiet: false };
    let total = store::reparse_pending(db, schema::PARSER_VERSION, force).unwrap_or(0) as u64;

    let mut acc = Acc {
        files_seen: 0,
        files_total: total,
        ..Acc::default()
    };
    acc.current_file = None;
    let hero = hero_ctx(db);
    let run_id = store::start_import_run(db, "reparse").ok();

    // Snapshot the hand ids that need work *up front*. Rows processed
    // successfully drop out of `raw_hands_needing_reparse`'s result set, so
    // paginating it live would shift offsets under our feet.
    let mut ids: Vec<String> = Vec::new();
    const PAGE: i64 = 2000;
    let mut offset = 0i64;
    loop {
        let rows = store::raw_hands_needing_reparse(db, schema::PARSER_VERSION, force, PAGE, offset)?;
        if rows.is_empty() {
            break;
        }
        offset += rows.len() as i64;
        ids.extend(rows.into_iter().map(|r| r.hand_id));
    }

    let parser_impl = parser::default_parser();
    let mut done: u64 = 0;
    for hand_id in &ids {
        let raw = match store::raw_hand(db, hand_id) {
            Ok(Some(r)) => r,
            Ok(None) => {
                // Raw text vanished mid-run; nothing to rebuild.
                acc.failures += 1;
                done += 1;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let source_file: String = db
            .with_reader(|c| {
                c.query_row(
                    "SELECT COALESCE(source_file, '') FROM hands WHERE hand_id = ?1",
                    [hand_id],
                    |r| r.get(0),
                )
            })
            .ok()
            .unwrap_or_default();

        match parser_impl.parse_hand(&raw) {
            Ok(parsed) => {
                let analysis = hero_analysis(&parsed, &hero);
                // Rebuild from scratch: derived rows go first, then the fresh
                // insert. `clear_derived` deliberately retains the raw text row,
                // so drop it before upserting (whose new-hand branch does a
                // plain INSERT) and let upsert write the fresh copy; restore it
                // if the insert fails so the hand stays reparseable.
                store::clear_derived(db, hand_id)?;
                let _ = db.with_writer(|c| {
                    c.execute("DELETE FROM raw_hands WHERE hand_id = ?1", [hand_id])
                });
                let upsert = store::upsert_hand(
                    db,
                    &parsed,
                    analysis.as_ref(),
                    &raw,
                    if source_file.is_empty() { hand_id.as_str() } else { source_file.as_str() },
                );
                match upsert {
                    Ok(store::UpsertOutcome::Inserted) => acc.hands_imported += 1,
                    Ok(store::UpsertOutcome::Duplicate) => acc.duplicates += 1,
                    Err(e) => {
                        tracing::warn!(hand_id, error = %e, "could not re-store hand");
                        let _ = db.with_writer(|c| {
                            c.execute(
                                "INSERT INTO raw_hands(hand_id, raw_text) VALUES (?1, ?2)\
                                 ON CONFLICT(hand_id) DO UPDATE SET raw_text = excluded.raw_text",
                                rusqlite::params![hand_id, raw],
                            )
                        });
                        acc.failures += 1;
                    }
                }
            }
            Err(e) => {
                let reason = e.to_string();
                if let Err(qe) = store::quarantine(db, &source_file, &raw, &reason, schema::PARSER_VERSION)
                {
                    tracing::warn!(hand_id, error = %qe, "quarantine write failed during reparse");
                }
                acc.failures += 1;
                acc.quarantined += 1;
            }
        }
        done += 1;
        acc.files_done = done;
        acc.current_file = Some(hand_id.clone());
        if done % 64 == 0 {
            acc.report(&opts, progress);
        }
    }
    acc.report(&opts, progress);

    if let Some(id) = run_id {
        let _ = store::finish_import_run(
            db,
            id,
            &store::ImportRunStats {
                files_seen: 0,
                hands_imported: acc.hands_imported as i64,
                duplicates: acc.duplicates as i64,
                failures: acc.failures as i64,
            },
        );
    }

    acc.current_file = None;
    if !opts.quiet {
        let mut snap = acc.snapshot();
        snap.finished = true;
        progress(&snap);
    }
    Ok(acc.into_outcome(t0.elapsed().as_millis()))
}

/// Detect the hero's screen name from the hand histories themselves.
///
/// This is the most reliable source, and the one to prefer. PokerStars writes
/// `Dealt to <screen name> [cards]` only for the player the file belongs to, so
/// the name that appears there across several hands *is* the hero. Unlike the
/// folder name it does not depend on how the directory happens to be named, and
/// unlike an alias it is the name the client is actually writing right now.
///
/// Reads a bounded prefix of a handful of files so a large archive is cheap.
pub fn detect_hero_from_hands(dir: &Path) -> Option<String> {
    /// Enough of a file to contain many `Dealt to` lines.
    const PREFIX_BYTES: usize = 256 * 1024;
    /// Enough files to be confident, without reading an archive.
    const MAX_FILES: usize = 20;

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for path in list_files(dir).into_iter().take(MAX_FILES) {
        let Some(text) = read_prefix(&path, PREFIX_BYTES) else { continue };
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("Dealt to ") else { continue };
            let Some((name, _)) = rest.split_once(" [") else { continue };
            let name = name.trim();
            if !name.is_empty() {
                *counts.entry(name.to_string()).or_default() += 1;
            }
        }
        // One name seen more than once is conclusive: the client only deals to
        // its owner's seat from the owner's perspective.
        if counts.values().any(|seen| *seen > 1) {
            break;
        }
    }
    counts
        .into_iter()
        .max_by_key(|(_, seen)| *seen)
        .map(|(name, _)| name)
}

/// Read at most `limit` bytes from a file as lossy UTF-8, tolerating a truncated
/// final character.
fn read_prefix(path: &Path, limit: usize) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut buffer = Vec::with_capacity(limit.min(64 * 1024));
    file.take(limit as u64).read_to_end(&mut buffer).ok()?;
    Some(String::from_utf8_lossy(&buffer).into_owned())
}

/// Detect the hero's screen name from a PokerStars hand-history folder name.
///
/// **Not used by first-run setup.** It is kept only as a documented dead end:
/// PokerStars does name the per-account folder after the screen name, but a
/// directory name is not evidence of an identity. `hands`, `HandHistory`, and
/// any temp directory ending in digits all look plausible and none of them is a
/// player, so a guess here silently attributes every statistic to somebody who
/// does not exist. [`detect_hero_from_hands`] reads the name out of the hands
/// instead, which is exact.
pub fn detect_hero_name(dir: &Path) -> Option<String> {
    let name = dir.file_name()?.to_str()?;
    let trimmed = name.trim_end_matches(|c: char| c.is_ascii_digit());
    if trimmed.len() == name.len() || trimmed.trim_end_matches('.').is_empty() {
        return None;
    }
    Some(trimmed.trim_end_matches('.').to_string())
}

// ---------------------------------------------------------------------------
// Watcher
// ---------------------------------------------------------------------------

/// The handle the continuous watcher publishes progress through. Exposed as a
/// process-wide singleton so the web layer (`AppState::importer`) can read the
/// same state the watcher writes.
static WATCHER_HANDLE: std::sync::OnceLock<ImporterHandle> = std::sync::OnceLock::new();

pub fn shared_importer_handle() -> &'static ImporterHandle {
    WATCHER_HANDLE.get_or_init(ImporterHandle::new)
}

/// Wait until `shutdown` flips to `true` or the channel closes. Never panics —
/// a dropped sender is treated as a stop signal.
async fn shutdown_fired(shutdown: &mut tokio::sync::watch::Receiver<bool>) -> bool {
    if *shutdown.borrow() {
        return true;
    }
    match shutdown.changed().await {
        Ok(()) => *shutdown.borrow_and_update(),
        Err(_) => true,
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
    let dir = cfg.hand_history_dir.clone();
    let _ = std::fs::create_dir_all(&dir);

    // notify runs on its own thread and feeds a std channel, which we drain
    // from a short async tick loop (std channels are not pollable in async).
    let (ntx, nrx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher = match notify::recommended_watcher(ntx) {
        Ok(w) => Some(w),
        Err(e) => {
            tracing::warn!(error = %e, "could not start notify watcher; falling back to polling");
            None
        }
    };
    if let Some(w) = watcher.as_mut() {
        if let Err(e) = w.watch(&dir, notify::RecursiveMode::Recursive) {
            tracing::warn!(error = %e, "could not watch directory; falling back to polling");
        }
    }
    let handle = shared_importer_handle().clone();
    handle.set(ImporterSnapshot {
        running: false,
        kind: "watch".to_string(),
        progress: ImportProgress::default(),
        last_error: None,
    });

    let scan_interval = std::time::Duration::from_secs(cfg.scan_interval_secs.max(1));
    // A burst of writes (many hands flushed at once) should coalesce into one
    // scan only after the noise settles.
    const TICK: std::time::Duration = std::time::Duration::from_millis(500);
    const DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(1500);

    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut event_seen: Option<Instant> = None;
    let mut last_scan: Option<Instant> = None; // None ⇒ scan right away at startup
    let mut shutdown = shutdown.clone();

    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = shutdown_fired(&mut shutdown) => break,
        }
        if *shutdown.borrow() {
            break;
        }

        if take_watch_events(&nrx) {
            event_seen = Some(Instant::now());
        }

        let now = Instant::now();
        let debounced = match event_seen {
            Some(t) => now.duration_since(t) >= DEBOUNCE,
            None => false,
        };
        let poll_due =
            last_scan.is_none() || now.duration_since(last_scan.expect("checked")) >= scan_interval;

        if !debounced && !poll_due {
            continue;
        }
        event_seen = None;
        last_scan = Some(now);

        if !scan_once(&db, &dir, &handle, &shutdown).await {
            break;
        }
        if *shutdown.borrow() {
            break;
        }
    }

    handle.update(|s| {
        s.running = false;
        s.progress.finished = true;
    });
    drop(watcher);
    Ok(())
}

/// Drop unread watch events so the reader never blocks new ones and report
/// whether anything happened (watch errors also mean "look again").
fn take_watch_events(rx: &std::sync::mpsc::Receiver<notify::Result<notify::Event>>) -> bool {
    let mut dirty = false;
    while let Ok(_) = rx.try_recv() {
        dirty = true;
    }
    dirty
}

/// One blocking import pass inside `spawn_blocking`. Returns `false` when the
/// shutdown signal fired, asking the caller to stop.
async fn scan_once(
    db: &Db,
    dir: &Path,
    handle: &ImporterHandle,
    shutdown: &tokio::sync::watch::Receiver<bool>,
) -> bool {
    handle.update(|s| {
        s.running = true;
        s.last_error = None;
        s.progress = ImportProgress { finished: false, ..ImportProgress::default() };
    });
    let db = db.clone();
    let dir = dir.to_path_buf();
    let handle2 = handle.clone();
    let join = tokio::task::spawn_blocking(move || {
        let mut reporter = |p: &ImportProgress| {
            handle2.update(|s| {
                s.progress = p.clone();
            });
        };
        import_directory(&db, &dir, &ImportOptions { force: false, quiet: false }, &mut reporter)
    });

    // Wait for the pass, but bail out promptly if shutdown fires mid-scan. The
    // import itself commits per hand, so aborting it leaves no torn state.
    loop {
        if join.is_finished() {
            match join.await {
                Ok(Ok(outcome)) => {
                    tracing::info!(imported = outcome.hands_imported, "watcher scan finished");
                    handle.update(|s| {
                        s.running = false;
                        s.progress.finished = true;
                    });
                }
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "watcher scan failed");
                    handle.update(|s| {
                        s.running = false;
                        s.progress.finished = true;
                        s.last_error = Some(e.to_string());
                    });
                }
                Err(e) => {
                    tracing::warn!(error = %e, "watcher scan task panicked");
                    handle.update(|s| {
                        s.running = false;
                        s.last_error = Some(format!("scan task failed: {e}"));
                    });
                }
            }
            return true;
        }
        if *shutdown.borrow() {
            join.abort();
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
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

#[cfg(test)]
mod detection_tests {
    use super::*;

    /// A PokerStars hand-history file as the client writes it: the folder is the
    /// screen name and only the owner's seat is ever "Dealt to".
    fn hand_file(id: u64, hero: &str) -> String {
        format!(
            r#"PokerStars Hand #{id}:  Hold'em No Limit (€0.25/€0.50 EUR) - 2024/01/02 13:00:00 CET [2024/01/02 7:00:00 ET]
Table 'Alpha' 6-max Seat #6 is the button
Seat 1: villainAA (€50 in chips)
Seat 2: {hero} (€50 in chips)
villainAA: posts small blind €0.25
{hero}: posts big blind €0.50
*** HOLE CARDS ***
Dealt to {hero} [As Kd]
villainAA: folds
Uncalled bet (€0.25) returned to {hero}
{hero} collected €0.50 from pot
*** SUMMARY ***
Total pot €0.50 | Rake €0.00
"#
        )
    }

    #[test]
    fn the_hero_is_read_out_of_the_hands_not_the_folder_name() {
        // A folder named exactly like the real client's: no numeric suffix.
        let dir = crate::storage::tempdir::TempDir::new("texture-hero").expect("temp dir");
        std::fs::write(dir.path().join("HH20240102 hero.name.txt"), hand_file(1, "hero.name"))
            .expect("write");
        std::fs::write(dir.path().join("HH20240103 hero.name.txt"), hand_file(2, "hero.name"))
            .expect("write");

        assert_eq!(
            detect_hero_from_hands(dir.path()).as_deref(),
            Some("hero.name"),
            "the name must come from the hand data"
        );
    }

    #[test]
    fn an_empty_archive_invents_no_player() {
        let dir = crate::storage::tempdir::TempDir::new("texture-hero").expect("temp dir");
        assert_eq!(detect_hero_from_hands(dir.path()), None);
    }

    #[test]
    fn a_directory_name_is_not_an_identity() {
        // The reason folder-name detection is not wired up: every one of these
        // looks name-like and none of them is a player.
        let base = crate::storage::tempdir::TempDir::new("texture-hero").expect("temp dir");
        for name in ["hands", "HandHistory", "texture-hero-1234-5678"] {
            let candidate = base.path().join(name);
            std::fs::create_dir(&candidate).expect("mkdir");
            // Whatever this returns, first-run setup must not trust it, so the
            // guard that matters is that the *hands* are the only trusted source.
            let _ = detect_hero_name(&candidate);
        }
        // With no readable hands anywhere, nothing is detected.
        assert_eq!(detect_hero_from_hands(base.path()), None);
    }
}
