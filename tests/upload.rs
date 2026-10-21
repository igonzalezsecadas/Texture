//! Importing hand text uploaded through the browser (or pasted) shares the
//! file importer's parsing, hero resolution and dedupe rules. These tests pin
//! the contract: re-uploading the same text changes nothing, malformed hands
//! are quarantined rather than dropped, and a truncated trailing hand is held
//! back instead of being treated as an error.

use texture::importer::{
    detect_hero_from_text, import_text, ImportOptions, TextImportOutcome,
};
use texture::storage::hands as store;
use texture::storage::Db;

/// One complete hand where the hero (seat 2, the big blind) takes the pot down
/// preflop. Same shape as the web/e2e fixtures.
fn walk_hand(hand_id: u64, hero: &str, day: u32) -> String {
    format!(
        r#"PokerStars Hand #{hand_id}:  Hold'em No Limit (€0.25/€0.50 EUR) - 2024/01/{day:02} 13:00:00 CET [2024/01/{day:02} 7:00:00 ET]
Table 'Alpha' 6-max Seat #6 is the button
Seat 1: villainAA (€50 in chips)
Seat 2: {hero} (€50 in chips)
Seat 3: villainBB (€50 in chips)
Seat 4: villainCC (€50 in chips)
Seat 5: villainDD (€50 in chips)
Seat 6: villainEE (€50 in chips)
villainAA: posts small blind €0.25
{hero}: posts big blind €0.50
*** HOLE CARDS ***
Dealt to {hero} [As Kd]
villainBB: folds
villainCC: folds
villainDD: folds
villainEE: folds
villainAA: folds
Uncalled bet (€0.25) returned to {hero}
{hero} collected €0.50 from pot
*** SUMMARY ***
Total pot €0.50 | Rake €0.00
Seat 1: villainAA (small blind) folded before Flop
Seat 2: {hero} (big blind) collected (€0.50)
"#
    )
}

struct Fixture {
    db: Db,
    _data: tempfile::TempDir,
}

fn fixture(hero: Option<&str>) -> Fixture {
    let data = tempfile::tempdir().expect("data dir");
    let db = Db::open(&data.path().join("texture.sqlite")).expect("open db");
    if let Some(hero) = hero {
        db.ensure_hero_profile(hero).expect("hero profile");
    }
    Fixture { db, _data: data }
}

fn import(db: &Db, text: &str, source: &str) -> TextImportOutcome {
    import_text(db, text, source, &ImportOptions { quiet: true, ..Default::default() }, &mut |_| {})
        .expect("text import")
}

fn hand_count(db: &Db) -> i64 {
    db.with_reader(|c| c.query_row("SELECT COUNT(*) FROM hands", [], |r| r.get(0)))
        .expect("count hands")
}

#[test]
fn uploading_the_same_text_twice_inserts_nothing_new() {
    let f = fixture(Some("hero"));
    let text = format!("{}\n{}", walk_hand(1000, "hero", 2), walk_hand(1001, "hero", 3));

    let first = import(&f.db, &text, "upload:first.txt");
    assert_eq!(first.hands_seen, 2);
    assert_eq!(first.hands_imported, 2);
    assert_eq!(first.duplicates, 0);
    assert_eq!(first.quarantined, 0);
    assert_eq!(hand_count(&f.db), 2);

    let second = import(&f.db, &text, "upload:again.txt");
    assert_eq!(second.hands_seen, 2);
    assert_eq!(second.hands_imported, 0, "a re-upload must not insert duplicates");
    assert_eq!(second.duplicates, 2);
    assert_eq!(hand_count(&f.db), 2, "the hand table is unchanged by a re-upload");
}

#[test]
fn malformed_hands_are_quarantined_and_good_hands_still_import() {
    let f = fixture(Some("hero"));
    let text = format!(
        "{}\nPokerStars Hand #9999: this is not a hand\n*** SUMMARY ***\nTotal pot €0.00\n",
        walk_hand(1000, "hero", 2)
    );

    let out = import(&f.db, &text, "upload:mixed.txt");
    assert_eq!(out.hands_seen, 2);
    assert_eq!(out.hands_imported, 1);
    assert_eq!(out.failures, 1);
    assert_eq!(out.quarantined, 1);
    assert_eq!(hand_count(&f.db), 1);
    assert_eq!(store::quarantine_count(&f.db).expect("quarantine count"), 1);
}

#[test]
fn a_truncated_trailing_hand_is_held_back_without_quarantine() {
    let f = fixture(Some("hero"));
    let text = format!(
        "{}\nPokerStars Hand #2000:  Hold'em No Limit (€0.25/€0.50 EUR) - 2024/01/05 13:00:00 CET [2024/01/05 7:00:00 ET]\nTable 'Alpha' 6-max Seat #6 is the button\n*** HOLE CARDS ***\n",
        walk_hand(1000, "hero", 2)
    );

    let out = import(&f.db, &text, "upload:partial.txt");
    assert_eq!(out.hands_seen, 1);
    assert_eq!(out.hands_imported, 1);
    assert!(out.incomplete_trailing, "the summary-less tail must be reported");
    assert_eq!(out.quarantined, 0, "a hand still being written is not an error");
    assert_eq!(store::quarantine_count(&f.db).expect("quarantine count"), 0);
}

#[test]
fn an_active_hero_profile_produces_analysis_rows() {
    let f = fixture(Some("hero"));
    let out = import(&f.db, &walk_hand(1000, "hero", 2), "upload:hero.txt");
    assert_eq!(out.hands_imported, 1);

    let analyses: i64 = f
        .db
        .with_reader(|c| c.query_row("SELECT COUNT(*) FROM hero_hands", [], |r| r.get(0)))
        .expect("count hero hands");
    assert_eq!(analyses, 1, "the uploaded hand must be attributed to the hero");
}

#[test]
fn the_hero_can_be_detected_from_uploaded_text() {
    let text = format!(
        "{}\n{}\n{}",
        walk_hand(1000, "hero", 2),
        walk_hand(1001, "hero", 3),
        walk_hand(1002, "somebody_else", 4)
    );
    assert_eq!(detect_hero_from_text(&text).as_deref(), Some("hero"));
    assert_eq!(detect_hero_from_text("no deals here"), None);
}

#[test]
fn empty_text_is_not_an_incomplete_hand() {
    let f = fixture(None);
    let out = import(&f.db, "   \n\n", "upload:empty.txt");
    assert_eq!(out.hands_seen, 0);
    assert_eq!(out.hands_imported, 0);
    assert!(!out.incomplete_trailing);
    assert_eq!(hand_count(&f.db), 0);
}
