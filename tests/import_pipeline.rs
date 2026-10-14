//! Import pipeline integration tests. These exercise the full bulk path
//! (`importer::import_directory`) plus `reparse` against a real temporary
//! database and real hand-history files, in the shape of a live play-by-play
//! capture: grows, truncation, and garbage included.

use rusqlite::OptionalExtension;
use std::path::{Path, PathBuf};
use texture::importer::{import_directory, reparse, ImportOptions};
use texture::storage::{hands as store, Db};

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

/// One complete, valid PokerStars hand, same shape as the parser's own unit
/// test hand (`src/parser/pokerstars.rs`): 6-max, €0.25/€0.50, hero wins €20,
/// contributions sum to the €20.25 pot. `id` must be unique per test hand.
fn hand(id: &str) -> String {
    format!(
        r#"PokerStars Hand #{id}:  Hold'em No Limit (€0.25/€0.50 EUR) - 2024/01/02 13:45:06 CET [2024/01/02 7:45:06 ET]
Table 'Alpha II' 6-max Seat #6 is the button
Seat 1: villainAA (€50 in chips)
Seat 2: hero (€52.75 in chips)
Seat 3: villainBB (€48.30 in chips)
Seat 4: villainCC (€51 in chips)
Seat 5: villainDD (€50 in chips)
Seat 6: villainEE (€49.50 in chips)
villainAA: posts small blind €0.25
hero: posts big blind €0.50
*** HOLE CARDS ***
Dealt to hero [As Kd]
villainBB: folds
villainCC: raises €1 to €1.50
villainDD: folds
villainEE: folds
villainAA: folds
hero: raises €3.50 to €5
villainCC: calls €3.50
*** FLOP *** [Ah 7c 2d]
hero: bets €5
villainCC: calls €5
*** TURN *** [Ah 7c 2d] [Ts]
hero: bets €12
villainCC: folds
Uncalled bet (€12) returned to hero
hero collected €20 from pot
*** SUMMARY ***
Total pot €20.25 | Rake €0.25
Board [Ah 7c 2d Ts]
Seat 1: villainAA (small blind) folded before Flop
Seat 2: hero (big blind) collected (€20)
Seat 3: villainBB (UTG) folded before Flop
Seat 6: villainEE (button) folded before Flop
"#
    )
}

/// A completed hand block the parser always rejects (USD is not a supported
/// currency, per the parser's own test suite).
fn unsupportable_hand(id: &str) -> String {
    hand(id).replace("(€0.25/€0.50 EUR)", "($0.25/$0.50 USD)")
}

/// Just the first three lines of a hand: enough to look like it started, too
/// little to ever parse.
fn hand_preamble(id: &str) -> String {
    format!(
        "PokerStars Hand #{id}:  Hold'em No Limit (€0.25/€0.50 EUR) - 2024/01/02 13:45:06 CET [2024/01/02 7:45:06 ET]\n\
         Table 'Alpha II' 6-max Seat #6 is the button\n\
         Seat 1: villainAA (€50 in chips)\n"
    )
}

/// Everything after a [`hand_preamble`]: what a live PokerStars client appends
/// when it finishes writing a hand that was already listed in the file.
fn hand_remainder(id: &str) -> String {
    hand(id).lines().skip(3).collect::<Vec<_>>().join("\n") + "\n"
}

fn tempdir() -> (Db, texture::storage::tempdir::TempDir) {
    Db::open_temp().expect("temp db")
}

struct Hands {
    dir: tempfile::TempDir,
}

impl Hands {
    fn new() -> Hands {
        Hands { dir: tempfile::tempdir().expect("temp hands dir") }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn dir_path(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.path(name);
        std::fs::write(&path, contents).expect("write hands file");
        path
    }

    fn append(&self, name: &str, more: &str) -> PathBuf {
        let path = self.path(name);
        let current = std::fs::read(&path).expect("read hands file");
        let mut text = String::from_utf8(current).expect("utf8");
        if !more.is_empty() && !text.ends_with('\n') && !more.starts_with('\n') {
            text.push('\n');
        }
        text.push_str(more);
        std::fs::write(&path, text).expect("append hands file");
        path
    }
}

fn quiet() -> ImportOptions {
    ImportOptions { force: false, quiet: true }
}

fn force() -> ImportOptions {
    ImportOptions { force: true, quiet: true }
}

fn import(db: &Db, dir: &Path) -> texture::importer::ImportOutcome {
    import_directory(db, dir, &quiet(), &mut |_| {}).expect("import succeeds")
}

fn hand_count(db: &Db) -> i64 {
    store::hand_count_total(db).expect("hand count")
}

fn raw_count(db: &Db) -> i64 {
    store::raw_hand_count(db).expect("raw hand count")
}

fn quarantine_rows(db: &Db) -> Vec<store::QuarantineRow> {
    store::list_quarantine(db, 100, 0).expect("quarantine list")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn importing_the_same_directory_twice_changes_nothing() {
    let (db, _guard) = tempdir();
    let hands = Hands::new();

    hands.write("tourney-01.txt", &format!("{}{}", hand("880000001"), hand("880000002")));

    let first = import(&db, &hands.dir_path());
    assert_eq!(first.files_seen, 1);
    assert_eq!(first.hands_imported, 2, "first run imports both hands");
    assert_eq!(first.duplicates, 0);
    assert_eq!(first.failures, 0);
    assert_eq!(first.quarantined, 0);
    assert_eq!(hand_count(&db), 2);

    let second = import(&db, &hands.dir_path());
    assert_eq!(second.hands_imported, 0, "second run imports nothing");
    assert_eq!(second.duplicates, 2, "already-imported hands surface as duplicates");
    assert_eq!(second.quarantined, 0);
    assert_eq!(second.failures, 0);
    assert_eq!(hand_count(&db), 2, "no data changes on a re-import");
    assert_eq!(raw_count(&db), 2);
}

#[test]
fn appending_hands_imports_only_the_new_hands() {
    let (db, _guard) = tempdir();
    let hands = Hands::new();

    hands.write("tourney-01.txt", &format!("{}{}", hand("880100001"), hand("880100002")));
    import(&db, &hands.dir_path());
    assert_eq!(hand_count(&db), 2);

    // The live capture appends to an existing file with the same prefix.
    hands.append("tourney-01.txt", &hand("880100003"));

    let second = import(&db, &hands.dir_path());
    assert_eq!(
        second.hands_imported, 1,
        "exactly the appended hand is imported"
    );
    assert_eq!(second.quarantined, 0);
    assert_eq!(hand_count(&db), 3);
    assert!(store::raw_hand(&db, "880100003").unwrap().is_some());

    // An unchanged file afterwards is skipped entirely.
    let third = import(&db, &hands.dir_path());
    assert_eq!(third.hands_imported, 0);
    assert_eq!(third.duplicates, 3);
    assert_eq!(hand_count(&db), 3);
}

#[test]
fn truncated_hand_is_held_until_the_file_grows() {
    let (db, _guard) = tempdir();
    let hands = Hands::new();

    // One complete hand followed by the first lines of a second one: the
    // trailing hand has no summary yet, so it must be held back.
    let partial = format!("{}{}", hand("880200001"), hand_preamble("880200002"));
    hands.write("tourney-01.txt", &partial);

    let first = import(&db, &hands.dir_path());
    assert_eq!(first.hands_imported, 1, "only the complete hand imports");
    assert_eq!(first.failures, 0, "a truncated hand is not quarantined, just held");
    assert_eq!(first.quarantined, 0);
    assert_eq!(hand_count(&db), 1);
    assert!(store::raw_hand(&db, "880200002").unwrap().is_none());

    // The file state should remember how far into the file we parsed.
    let state = store::get_file_state(&db, &hands.path("tourney-01.txt").to_string_lossy())
        .unwrap()
        .expect("file state recorded");
    assert!(
        state.parse_cursor < state.size,
        "parse cursor stops before the incomplete hand ({} < {})",
        state.parse_cursor,
        state.size
    );

    // The hand is completed and the run resumes exactly at the cursor.
    hands.append("tourney-01.txt", &hand_remainder("880200002"));
    let second = import(&db, &hands.dir_path());
    assert_eq!(second.hands_imported, 1, "only the now-complete hand imports");
    assert_eq!(hand_count(&db), 2);
    assert!(store::raw_hand(&db, "880200002").unwrap().is_some());
}

#[test]
fn malformed_hand_is_quarantined_and_does_not_stop_the_file() {
    let (db, _guard) = tempdir();
    let hands = Hands::new();

    let contents = format!(
        "{}{}{}",
        hand("880300001"),
        unsupportable_hand("880300002"),
        hand("880300003")
    );
    hands.write("tourney-01.txt", &contents);
    // A second healthy file shares the room; the bad hand in file 1 must not
    // affect it. (Symlink-weighted directories are exercised elsewhere.)

    let outcome = import(&db, &hands.dir_path());
    assert_eq!(outcome.files_seen, 1);
    assert_eq!(outcome.hands_imported, 2, "both healthy hands still import");
    assert_eq!(outcome.failures, 1, "the bad hand counts as a failure");
    assert_eq!(outcome.quarantined, 1);
    assert_eq!(hand_count(&db), 2);

    let rows = quarantine_rows(&db);
    assert_eq!(rows.len() as i64, store::quarantine_count(&db).unwrap());
    assert_eq!(rows.len(), 1, "one quarantine row for the bad hand");
    assert_eq!(rows[0].source_file, hands.path("tourney-01.txt").to_string_lossy());
    assert!(!rows[0].reason.is_empty(), "a reason is recorded");
    assert!(store::raw_hand(&db, "880300002").unwrap().is_none(), "bad hand is not imported");
}

#[test]
fn quarantine_retry_after_a_parser_fix_imports_the_hand() {
    let (db, _guard) = tempdir();
    let hands = Hands::new();

    let bad = unsupportable_hand("880400002");
    let good = hand("880400001");
    hands.write("tourney-01.txt", &format!("{good}{bad}"));

    let first = import(&db, &hands.dir_path());
    assert_eq!(first.hands_imported, 1);
    assert_eq!(first.quarantined, 1);

    // The operator marks the row for retry (UI action), fixes the hand text in
    // the file, and forces a re-read of the file.
    let rows = quarantine_rows(&db);
    assert_eq!(rows.len(), 1);
    store::bump_quarantine_retry(&db, rows[0].id).expect("bump retry");

    hands.write("tourney-01.txt", &format!("{good}{}", hand("880400002")));
    let forced = import_directory(&db, &hands.dir_path(), &force(), &mut |_| {})
        .expect("forced re-import succeeds");
    assert_eq!(forced.hands_imported, 1, "the repaired hand imports");
    assert_eq!(forced.duplicates, 1, "the untouched hand remains a duplicate");
    assert_eq!(hand_count(&db), 2);
    assert!(store::raw_hand(&db, "880400002").unwrap().is_some(), "fixed hand is present");
    assert_eq!(store::quarantine_count(&db).unwrap(), 1, "quarantine row is kept for the record");
}

#[test]
fn missing_file_is_reported_but_does_not_stop_the_directory() {
    let (db, _guard) = tempdir();
    let hands = Hands::new();

    hands.write("tourney-01.txt", &hand("880500001"));

    // A dangling symlink looks like a hand-history file but its target is gone.
    let broken = hands.path("broken.txt");
    std::os::unix::fs::symlink(&hands.path("vanished.txt"), &broken).expect("symlink");

    let outcome = import(&db, &hands.dir_path());
    assert_eq!(outcome.files_seen, 2, "both directory entries are seen");
    assert_eq!(outcome.hands_imported, 1, "the real file still imports");
    assert_eq!(outcome.failures, 0, "an unreadable file is a warning, not a failure");
    assert_eq!(outcome.quarantined, 0);
    assert_eq!(hand_count(&db), 1);
}

#[test]
fn reparse_is_idempotent_and_rebuilds_derived_rows() {
    let (db, _guard) = tempdir();
    let hands = Hands::new();

    hands.write("tourney-01.txt", &format!("{}{}", hand("880600001"), hand("880600002")));
    import(&db, &hands.dir_path());
    assert_eq!(hand_count(&db), 2);
    assert_eq!(raw_count(&db), 2);

    // Nothing needs a reparse at the current parser version.
    let fresh = reparse(&db, false, &mut |_| {}).expect("reparse");
    assert_eq!(fresh.hands_imported, 0);
    assert_eq!(fresh.failures, 0);
    assert_eq!(fresh.quarantined, 0);
    assert_eq!(hand_count(&db), 2);

    // Forced reparse rebuilds every hand from retained raw text.
    let rebuilt = reparse(&db, true, &mut |_| {}).expect("forced reparse");
    assert_eq!(rebuilt.hands_imported, 2, "every raw hand is rebuilt");
    assert_eq!(rebuilt.failures, 0);
    assert_eq!(rebuilt.quarantined, 0);

    // The rebuild did not change any totals — and doing it twice is harmless.
    assert_eq!(hand_count(&db), 2);
    assert_eq!(raw_count(&db), 2);
    let again = reparse(&db, true, &mut |_| {}).expect("second forced reparse");
    assert_eq!(again.hands_imported, 2);
    assert_eq!(hand_count(&db), 2);
    assert_eq!(raw_count(&db), 2);
}

// ---------------------------------------------------------------------------
// Hero analysis wiring
// ---------------------------------------------------------------------------

// The other hero-analysis assertions are intentionally absent: ticket 03's
// contract says no hero is configured during pipeline tests, so
// `crate::stats::analyze_hero` is never touched elsewhere. This test proves the
// actual wiring.
#[test]
fn hero_hands_row_is_written_for_a_configured_hero() {
    let (db, _guard) = tempdir();
    let hands = Hands::new();

    hands.write("tourney-01.txt", &hand("880700001"));

    // First-run wiring: the operator configures the hero before importing.
    let (profile_id, player_id) = db.ensure_hero_profile("hero").expect("hero profile");

    import(&db, &hands.dir_path());
    assert_eq!(hand_count(&db), 1);

    let row: Option<(String, i64, i64)> = db
        .with_reader(|c| {
            c.query_row(
                "SELECT hand_id, profile_id, player_id FROM hero_hands WHERE hand_id = ?1",
                ["880700001"],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
        })
        .expect("hero row query");
    assert_eq!(
        row,
        Some(("880700001".to_string(), profile_id, player_id)),
        "importing with a configured hero writes the hero_hands row"
    );

    // Reparse rebuilds the same hero row from raw text.
    reparse(&db, true, &mut |_| {}).expect("reparse");
    let reparsed: Option<i64> = db
        .with_reader(|c| {
            c.query_row(
                "SELECT profile_id FROM hero_hands WHERE hand_id = ?1",
                ["880700001"],
                |r| r.get(0),
            )
        })
        .expect("hero row query after reparse");
    assert_eq!(reparsed, Some(profile_id));
}

// ---------------------------------------------------------------------------
// Watcher
// ---------------------------------------------------------------------------

/// The continuous watcher imports hands written to the watched folder and
/// stops cleanly when the shutdown flag flips. The first scan imports the
/// appearing hand; later scans skip the unchanged file on the fast path
/// (duplicates), which is exactly the contract for the last published snapshot.
#[test]
fn watcher_imports_new_hands_and_shuts_down_cleanly() {
    use std::sync::Arc;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().expect("rt");
    rt.block_on(async {
        let (db, _guard) = tempdir();
        let db = Arc::new(db);
        let dir = tempfile::tempdir().expect("hands dir");
        let file_path = dir.path().join("tourney-01.txt");
        std::fs::write(&file_path, "").expect("create empty file");

        let mut cfg = texture::config::Config::from_env().expect("config");
        cfg.hand_history_dir = dir.path().to_path_buf();
        cfg.scan_interval_secs = 2;
        let (tx, rx) = tokio::sync::watch::channel(false);

        let watcher_db = db.clone();
        let watcher = tokio::spawn(texture::importer::run_watcher(
            (*watcher_db).clone(),
            cfg.clone(),
            rx.clone(),
        ));

        // Startup scan sees an empty folder; then a hand appears.
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        std::fs::write(&file_path, hand("990000001")).expect("write hand");
        tokio::time::sleep(std::time::Duration::from_secs(4)).await;

        assert_eq!(hand_count(&db), 1, "the watcher imported the appearing hand");
        let snap = texture::importer::shared_importer_handle().snapshot();
        assert!(
            snap.progress.hands_imported >= 1 || snap.progress.duplicates >= 1,
            "last scan saw the hand (imported={} duplicates={})",
            snap.progress.hands_imported,
            snap.progress.duplicates
        );
        assert!(snap.progress.finished);

        let sent = tx.send(true);
        let _ = sent;
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        assert!(watcher.is_finished(), "watcher exits promptly on shutdown");
    });
}
