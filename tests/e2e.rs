//! End-to-end acceptance test: raw hand-history file in, engine out.
//!
//! This exercises the whole documented path for the spec's definition of done:
//! a read-only hand-history directory is imported, a hero profile is resolved,
//! reports honour a filter, re-import is idempotent, appends and partial writes
//! behave, a bad hand quarantines without stopping the file, and a snapshot can
//! be taken and reopened.

use std::path::Path;
use texture::domain::{FilterSpec, GroupBy};
use texture::importer::{self, ImportOptions, ImportProgress};
use texture::{snapshot, stats, storage};

/// One complete hand where everyone folds to the hero's big blind.
///
/// Contributions are €0.25 from the small blind plus the hero's €0.75 minus the
/// €0.25 returned uncalled, so the pot is exactly €0.50 and the hero nets €0.25.
fn walk_hand(hand_id: u64) -> String {
    format!(
        r#"PokerStars Hand #{hand_id}:  Hold'em No Limit (€0.25/€0.50 EUR) - 2024/01/02 13:45:06 CET [2024/01/02 7:45:06 ET]
Table 'Alpha' 6-max Seat #6 is the button
Seat 1: villainAA (€50 in chips)
Seat 2: hero (€50 in chips)
Seat 3: villainBB (€50 in chips)
Seat 4: villainCC (€50 in chips)
Seat 5: villainDD (€50 in chips)
Seat 6: villainEE (€50 in chips)
villainAA: posts small blind €0.25
hero: posts big blind €0.50
*** HOLE CARDS ***
Dealt to hero [As Kd]
villainBB: folds
villainCC: folds
villainDD: folds
villainEE: folds
villainAA: folds
Uncalled bet (€0.25) returned to hero
hero collected €0.50 from pot
*** SUMMARY ***
Total pot €0.50 | Rake €0.00
Seat 1: villainAA (small blind) folded before Flop
Seat 2: hero (big blind) collected (€0.50)
"#
    )
}

fn write_hands(dir: &Path, name: &str, hands: &[String]) {
    let mut text = String::new();
    for hand in hands {
        text.push_str(hand);
        text.push('\n');
    }
    std::fs::write(dir.join(name), text).expect("writing hand history");
}

fn noop_progress() -> impl FnMut(&ImportProgress) {
    |_p: &ImportProgress| {}
}

#[test]
fn v1_acceptance_path() {
    let hands_dir = tempfile::tempdir().expect("hands temp dir");
    let data_dir = tempfile::tempdir().expect("data temp dir");
    let db = storage::Db::open(&data_dir.path().join("texture.sqlite")).expect("open db");

    // First run: the operator registers the hero identity. Everything downstream
    // depends on this being right, so it is explicit rather than inferred.
    let (profile_id, _player_id) = db.ensure_hero_profile("hero").expect("hero profile");

    let hands: Vec<String> = (1..=3).map(walk_hand).collect();
    write_hands(hands_dir.path(), "2024-01-02.txt", &hands);

    // --- bulk import -----------------------------------------------------
    let opts = ImportOptions::default();
    let mut progress = noop_progress();
    let first = importer::import_directory(&db, hands_dir.path(), &opts, &mut progress)
        .expect("import succeeds");
    assert_eq!(first.hands_imported, 3, "three hands imported");
    assert_eq!(first.quarantined, 0, "nothing quarantined");
    assert_eq!(storage::hands::hand_count_total(&db).expect("count"), 3);

    // --- reports honour the empty filter set as the lifetime total -------
    let groups = storage::queries::aggregate(&db, &FilterSpec::default(), GroupBy::None)
        .expect("aggregate");
    assert_eq!(groups.len(), 1, "ungrouped aggregate is a single population");
    let row = &groups[0].row;
    assert_eq!(row.hands, 3, "all three hands are in the hero population");
    assert_eq!(row.net_result_minor, 75, "three walks of 25 minor units each");
    assert!(
        (row.bb_per_100 - 50.0).abs() < 1e-9,
        "1.5 big blinds over 3 hands is 50 BB/100, got {}",
        row.bb_per_100
    );

    // --- idempotence -----------------------------------------------------
    let mut progress = noop_progress();
    let second = importer::import_directory(&db, hands_dir.path(), &opts, &mut progress)
        .expect("second import succeeds");
    assert_eq!(second.hands_imported, 0, "re-import inserts nothing");
    assert_eq!(second.duplicates, 3, "every hand is recognised as a duplicate");
    assert_eq!(storage::hands::hand_count_total(&db).expect("count"), 3);

    // --- appending while playing ----------------------------------------
    let mut appended = hands.clone();
    appended.push(walk_hand(4));
    write_hands(hands_dir.path(), "2024-01-02.txt", &appended);
    let mut progress = noop_progress();
    let third = importer::import_directory(&db, hands_dir.path(), &opts, &mut progress)
        .expect("incremental import succeeds");
    assert_eq!(third.hands_imported, 1, "only the appended hand is new");
    assert_eq!(storage::hands::hand_count_total(&db).expect("count"), 4);

    // --- a hand being written right now is not read ----------------------
    let mut partial = appended.clone();
    let incomplete = walk_hand(5);
    // Everything up to and including the last complete hand, plus a fragment.
    partial.push(incomplete[..incomplete.len() / 3].to_string());
    write_hands(hands_dir.path(), "2024-01-02.txt", &partial);
    let mut progress = noop_progress();
    let fourth = importer::import_directory(&db, hands_dir.path(), &opts, &mut progress)
        .expect("partial import succeeds");
    assert_eq!(fourth.hands_imported, 0, "an incomplete hand is held back");
    assert_eq!(
        storage::hands::hand_count_total(&db).expect("count"),
        4,
        "the partial hand must not be counted"
    );

    // --- completing it imports exactly that hand -------------------------
    partial.pop();
    partial.push(incomplete.clone());
    write_hands(hands_dir.path(), "2024-01-02.txt", &partial);
    let mut progress = noop_progress();
    let fifth = importer::import_directory(&db, hands_dir.path(), &opts, &mut progress)
        .expect("completion import succeeds");
    assert_eq!(fifth.hands_imported, 1, "the completed hand is now imported");
    assert_eq!(storage::hands::hand_count_total(&db).expect("count"), 5);

    // --- a malformed hand quarantines and does not stop the file ---------
    let mut with_bad = partial.clone();
    // A seat taking an action we cannot interpret, carrying an amount, must be a
    // parse error rather than a silent no-op.
    let bad = walk_hand(6)
        .replace("hero: posts big blind €0.50", "hero: posts big blind €0.50\nhero: wiggles €9.99");
    with_bad.push(bad);
    with_bad.push(walk_hand(7));
    write_hands(hands_dir.path(), "2024-01-02.txt", &with_bad);
    let mut progress = noop_progress();
    let _sixth = importer::import_directory(&db, hands_dir.path(), &opts, &mut progress)
        .expect("import with a bad hand succeeds");
    assert_eq!(
        storage::hands::hand_count_total(&db).expect("count"),
        6,
        "the good hand after the bad one still imported (5 + 1)"
    );
    assert!(
        storage::hands::quarantine_count(&db).expect("quarantine count") >= 1,
        "the malformed hand is quarantined rather than dropped"
    );
    let quarantined = storage::hands::list_quarantine(&db, 10, 0).expect("list quarantine");
    assert!(
        quarantined.iter().all(|q| !q.reason.is_empty()),
        "every quarantined hand carries a reason"
    );

    // --- raw text is retained, so any parse decision can be revisited ----
    let listed = storage::queries::hand_list(&db, &FilterSpec::default(), 10, 0).expect("hand list");
    assert!(!listed.is_empty(), "the hand browser has rows");
    let raw = storage::hands::raw_hand(&db, &listed[0].hand_id)
        .expect("raw hand query")
        .expect("raw text retained");
    assert!(raw.starts_with("PokerStars Hand #"), "the original text is verbatim");

    // --- filters select a different population ---------------------------
    let filtered = FilterSpec { from: Some(chrono::Utc::now() + chrono::Duration::days(1)), ..Default::default() };
    let mut progress = noop_progress();
    let _ = importer::import_directory(&db, hands_dir.path(), &opts, &mut progress);
    let empty = storage::queries::aggregate(&db, &filtered, GroupBy::None).expect("filtered aggregate");
    let filtered_hands: i64 = empty.iter().map(|g| g.row.hands).sum();
    assert_eq!(filtered_hands, 0, "a future date range selects nothing");

    // --- the chart payload is bounded regardless of hand count -----------
    let series = storage::queries::cumulative_series(&db, &FilterSpec::default(), 1000).expect("series");
    assert!(series.len() <= 1000, "the chart payload is bounded");
    assert!(!series.is_empty(), "the chart has points");

    // --- hero alias survives a rename ------------------------------------
    // "hero" and a second screen name both belong to the same profile.
    let renamed_player = db.upsert_player("hero.new.name").expect("second screen name");
    db.add_profile_player(profile_id, renamed_player).expect("alias mapping");
    let members = db.players_for_profile(profile_id).expect("profile players");
    assert!(members.iter().any(|(name, _)| name == "hero.new.name"));

    // --- snapshot can be taken and restored ------------------------------
    let snap_dir = data_dir.path().join("snapshots");
    let snap_path = snapshot::take_snapshot(&db, &snap_dir).expect("snapshot taken");
    assert!(snap_path.exists(), "snapshot file exists");
    let restored = storage::Db::open(&snap_path).expect("snapshot opens as a database");
    assert_eq!(
        storage::hands::hand_count_total(&restored).expect("restored count"),
        storage::hands::hand_count_total(&db).expect("live count"),
        "the snapshot contains the same hands as the live database"
    );
    assert_eq!(
        storage::hands::quarantine_count(&restored).expect("restored quarantine"),
        storage::hands::quarantine_count(&db).expect("live quarantine"),
        "the snapshot preserves the quarantine list"
    );

    // --- sessions are derived, not stored --------------------------------
    let gap = db.session_gap_secs().expect("session gap");
    let sessions = stats::sessions_for(&db, &FilterSpec::default(), gap);
    assert!(
        !sessions.is_empty(),
        "hands on the same timestamp collapse into at least one session"
    );
}
