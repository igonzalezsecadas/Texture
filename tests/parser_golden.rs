//! Golden-file tests over the PokerStars fixture corpus.
//!
//! Two things are checked, and they are deliberately different:
//!
//! 1. **Invariants** that must hold for every hand ever parsed, whatever the
//!    fixture — contributions reconcile with the pot, rake never exceeds it, no
//!    seat collects more than was in the pot, and every seat has a position.
//! 2. **Hand-authored expectations** for each edge case, written out here, so a
//!    parser change that silently alters a semantic is caught with a readable
//!    failure rather than only a golden-file diff.
//!
//! The `.json` goldens are a regression tripwire. Regenerate them with:
//! `cargo test --test parser_golden -- --ignored regenerate_goldens`.
//!
//! The corpus is SYNTHESIZED: no real PokerStars histories were available on the
//! development machine. See `fixtures/README.md` for the assumptions that implies.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use texture::domain::{Card, Position, Street};
use texture::ev;
use texture::parser;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/parser")
}

fn fixture_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(fixture_dir())
        .expect("fixtures/parser exists")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("txt"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "the fixture corpus must not be empty");
    files
}

fn name_of(path: &Path) -> String {
    path.file_stem().and_then(|s| s.to_str()).unwrap_or_default().to_string()
}

fn parse_fixture(name: &str) -> texture::domain::ParsedHand {
    let path = fixture_dir().join(format!("{name}.txt"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    parser::parse_hand(&text).unwrap_or_else(|e| panic!("fixture {name} failed to parse: {e}"))
}

fn spent(hand: &texture::domain::ParsedHand) -> i64 {
    hand.seats.iter().map(|s| s.contributed).sum()
}

fn collected(hand: &texture::domain::ParsedHand) -> i64 {
    hand.seats.iter().map(|s| s.result.minor + s.contributed).sum()
}

// ---------------------------------------------------------------------------
// Invariants, for every fixture
// ---------------------------------------------------------------------------

#[test]
fn every_fixture_parses_and_satisfies_its_invariants() {
    for path in fixture_files() {
        let name = name_of(&path);
        let hand = parse_fixture(&name);

        assert_eq!(
            spent(&hand),
            hand.pot_total.minor,
            "{name}: contributions must equal the pot (returned uncalled bets excluded)"
        );
        assert!(
            hand.rake.minor <= hand.pot_total.minor,
            "{name}: rake must never exceed the pot"
        );
        assert!(
            collected(&hand) <= hand.pot_total.minor,
            "{name}: no seat can collect more than the pot"
        );
        assert!(
            hand.small_blind.minor > 0 && hand.big_blind.minor > hand.small_blind.minor,
            "{name}: blinds must be positive and ordered"
        );

        // Every seat is labelled, and the button is one of the occupied seats.
        let seats: BTreeSet<u8> = hand.seats.iter().map(|s| s.seat).collect();
        assert!(seats.contains(&hand.button_seat), "{name}: the button must be seated");
        assert!(
            hand.seats.iter().all(|s| Position::ALL.contains(&s.position)),
            "{name}: every seat needs a position"
        );

        // The board matches the streets that were dealt.
        let expected_board = match hand.streets_dealt.iter().filter(|s| **s != Street::Preflop).count()
        {
            0 => 0,
            1 => 3,
            2 => 4,
            _ => 5,
        };
        assert_eq!(
            hand.board.len(),
            expected_board,
            "{name}: the board length must match the streets dealt"
        );
        assert!(
            hand.board.iter().all(|c| c.rank >= 2 && c.rank <= 14),
            "{name}: every board card must be well formed"
        );

        // No duplicate cards anywhere in the hand.
        let mut seen: Vec<Card> = hand.board.clone();
        for seat in &hand.seats {
            seen.extend(seat.shown_cards.iter().copied());
        }
        let unique: BTreeSet<(u8, char)> = seen.iter().map(|c| (c.rank, c.suit)).collect();
        assert_eq!(unique.len(), seen.len(), "{name}: a card is duplicated");

        // Pot attribution is consistent with the result column: a seat can only
        // ever take back what it put in plus a non-negative collection.
        for seat in &hand.seats {
            assert!(
                seat.result.minor + seat.contributed >= 0,
                "{name}: a seat cannot collect a negative amount"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Hand-authored expectations per edge case
// ---------------------------------------------------------------------------

#[test]
fn fold_to_the_big_blind_returns_the_uncalled_bet() {
    let hand = parse_fixture("fold_to_bb");
    assert_eq!(hand.button_seat, 6);
    assert_eq!(hand.streets_dealt, vec![Street::Preflop]);
    assert!(!hand.went_to_showdown);
    assert_eq!(hand.pot_total.minor, 50, "the small blind's 25 is the whole pot");
    assert_eq!(hand.rake.minor, 0);
    let hero = hand.seat_by_name("hero").expect("hero");
    assert_eq!(hero.position, Position::Bb);
    assert_eq!(hero.contributed, 25, "the uncalled half of the blind came back");
    assert_eq!(hero.result.minor, 25);
    // The small blind's 25 is dead money in the pot.
    assert_eq!(hand.seat_by_name("villainSB").expect("sb").contributed, 25);
}

#[test]
fn a_hand_that_ends_on_the_river_has_no_showdown() {
    let hand = parse_fixture("no_showdown");
    assert!(!hand.went_to_showdown);
    assert_eq!(hand.board.len(), 5);
    assert_eq!(hand.streets_dealt, vec![Street::Preflop, Street::Flop, Street::Turn, Street::River]);
    assert_eq!(hand.pot_total.minor, 725, "the small blind's dead 25 is in the pot");
    assert_eq!(hand.rake.minor, 25);
    let hero = hand.seat_by_name("hero").expect("hero");
    assert_eq!(hero.contributed, 350, "the uncalled river bet was returned");
    assert_eq!(hero.result.minor, 350);
    assert_eq!(hand.seat_by_name("villainCO").expect("co").contributed, 350);
}

#[test]
fn a_limped_multiway_pot_reaches_showdown() {
    let hand = parse_fixture("limped_multiway");
    assert!(hand.went_to_showdown);
    assert_eq!(hand.pot_total.minor, 400);
    assert_eq!(hand.rake.minor, 20);
    // Four players put money in: the two limpers, the small blind and the hero.
    let contributors = hand.seats.iter().filter(|s| s.contributed > 0).count();
    assert_eq!(contributors, 4);
    let hero = hand.seat_by_name("hero").expect("hero");
    assert_eq!(hero.contributed, 150);
    assert_eq!(hero.result.minor, -150, "the hero mucked and lost what they put in");
    // The hero's dealt cards are retained even though they mucked at showdown.
    assert_eq!(hero.shown_cards.len(), 2);
}

#[test]
fn a_split_pot_credits_both_players() {
    let hand = parse_fixture("split_pot");
    assert_eq!(hand.pot_total.minor, 625);
    assert_eq!(hand.rake.minor, 25);
    let hero = hand.seat_by_name("hero").expect("hero");
    // 300 in, 300 back: the pot was chopped after rake, including the dead blind.
    assert_eq!(hero.contributed, 300);
    assert_eq!(hero.result.minor, 0);
    assert_eq!(hand.seat_by_name("villainCO").expect("co").result.minor, 0);
    assert!(hand.went_to_showdown);
}

#[test]
fn a_two_way_preflop_all_in_adjusts_for_equity() {
    let hand = parse_fixture("allin_preflop_twoway");
    let hero = hand.seat_by_name("hero").expect("hero");
    assert_eq!(hero.contributed, 5000);
    assert_eq!(hero.contributed, hero.starting_stack.minor, "the hero is all-in");
    assert_eq!(hand.seat_by_name("villainCO").expect("co").contributed, 5000);
    assert_eq!(hand.pot_total.minor, 10_025, "the folded small blind's 25 is dead money");
    assert_eq!(hero.result.minor, 4825, "awards 9825 less the 5000 invested");

    let adjusted = ev::all_in_adjusted_result(&hand, hero.seat)
        .expect("both hands are known and the hero is all-in");
    // Aces against kings is roughly an 82% favourite of the 9800 awardable pot.
    assert!(
        (2500..=3600).contains(&adjusted),
        "adjusted result should reflect roughly 82% equity, got {adjusted}"
    );
    assert!(
        adjusted < hero.result.minor,
        "winning as a favourite means the actual result exceeds the adjusted one"
    );
}

#[test]
fn a_three_way_all_in_builds_a_side_pot() {
    let hand = parse_fixture("allin_threeway_sidepot");
    let hero = hand.seat_by_name("hero").expect("hero");
    assert_eq!(hero.contributed, 2000);
    assert_eq!(hero.contributed, hero.starting_stack.minor);
    assert_eq!(hand.pot_total.minor, 5025);

    let contribution_rows: Vec<(u8, i64)> = hand
        .seats
        .iter()
        .filter(|s| s.contributed > 0)
        .map(|s| (s.seat, s.contributed))
        .collect();
    // The small blind folded, so its 25 is dead money in the bottom pot.
    let sb = hand.seat_by_name("villainSB").expect("sb").seat;
    let pots = ev::side_pots_with_folded(&contribution_rows, &[sb]);
    assert_eq!(pots.len(), 3, "two all-in levels above the dead blind");
    assert_eq!(
        pots.iter().map(|p| p.amount).sum::<i64>(),
        5025,
        "every chip is in exactly one pot"
    );
    assert_eq!(pots[0].amount, 100, "the dead small blind's level");
    assert_eq!(pots[1].amount, 2925, "the short stack's all-in level");
    assert_eq!(pots[2].amount, 2000, "the level only two players could cover");
    assert!(
        pots.iter().all(|p| !p.eligible.contains(&sb)),
        "a folded seat can never win a pot"
    );
    let short = hand.seat_by_name("villainBTN").expect("button").seat;
    assert!(
        !pots[2].eligible.contains(&short),
        "the short stack cannot win the side pot"
    );
    assert_eq!(hero.result.minor + hero.contributed, 4825);
    let adjusted = ev::all_in_adjusted_result(&hand, hero.seat).expect("all-in with known cards");
    assert!(
        (1500..=2300).contains(&adjusted),
        "roughly 81% equity of the awardable pot less the hero's 2000, got {adjusted}"
    );
}

#[test]
fn a_short_all_in_for_less_caps_the_pot() {
    let hand = parse_fixture("short_allin_for_less");
    let hero = hand.seat_by_name("hero").expect("hero");
    let short = hand.seat_by_name("villainBTN").expect("button");
    assert_eq!(short.starting_stack.minor, 100);
    assert_eq!(short.contributed, 100, "an all-in for less is capped at the stack");
    assert_eq!(hero.contributed, 100, "the uncalled part of the raise came back");
    // Both blinds folded, so their 25 and 50 are dead money alongside the 200
    // that was actually contested.
    assert_eq!(hand.pot_total.minor, 275);
    assert_eq!(hero.result.minor + hero.contributed, 275);
    // The hero was never all-in, so nothing is adjusted for equity.
    assert_eq!(ev::all_in_adjusted_result(&hand, hero.seat), None);
}

#[test]
fn a_posted_dead_blind_is_dead_money() {
    let hand = parse_fixture("posted_dead_blind");
    let dead = hand.seat_by_name("villainMP").expect("mp");
    assert_eq!(dead.contributed, 50, "the dead blind is in the pot");
    let hero = hand.seat_by_name("hero").expect("hero");
    assert_eq!(hero.contributed, 25);
    assert_eq!(hand.pot_total.minor, 100);
    assert_eq!(hero.result.minor, 75);
}

#[test]
fn a_sitting_out_seat_is_recorded_and_a_timeout_is_not_an_action() {
    let hand = parse_fixture("timeout_and_sitout");
    assert_eq!(hand.seats.len(), 6, "a sitting-out player still occupies a seat");
    let sitting_out = hand.seat_by_name("villainCO").expect("co");
    assert!(sitting_out.sat_out);
    assert_eq!(sitting_out.contributed, 0);
    // The timeout notice carries no chips and must not become an action.
    let timed_out_actions = hand
        .actions
        .iter()
        .filter(|a| a.seat == hand.seat_by_name("villainMP").expect("mp").seat)
        .count();
    assert_eq!(timed_out_actions, 1, "only the fold is an action");
    assert_eq!(hand.pot_total.minor, 50);
}

#[test]
fn a_nine_handed_table_gets_nine_distinct_positions() {
    let hand = parse_fixture("nine_max_full_ring");
    assert_eq!(hand.table_size, 9);
    assert_eq!(hand.button_seat, 9);
    assert_eq!(hand.seats.len(), 9);
    let positions: BTreeSet<Position> = hand.seats.iter().map(|s| s.position).collect();
    assert_eq!(positions.len(), 9, "full ring must label every seat distinctly");
    assert!(positions.contains(&Position::Utg2), "9-max needs a third early seat");
    assert_eq!(hand.seat_by_name("hero").expect("hero").position, Position::Btn);
    assert_eq!(hand.pot_total.minor, 1450);
    assert_eq!(hand.rake.minor, 50);
}

#[test]
fn a_check_raise_is_recorded_at_the_raised_amount() {
    let hand = parse_fixture("check_raise");
    let hero = hand.seat_by_name("hero").expect("hero");
    // 150 preflop, then a raise to 600 on the flop, less the 400 returned uncalled.
    assert_eq!(hero.contributed, 350);
    assert_eq!(hand.pot_total.minor, 725);
    let flop_player_actions: Vec<_> = hand
        .actions
        .iter()
        .filter(|a| {
            a.street == Street::Flop
                && a.seat == hero.seat
                && a.kind != texture::domain::ActionKind::ReturnUncalled
        })
        .collect();
    assert_eq!(flop_player_actions.len(), 2, "a check and then a raise");
    assert!(flop_player_actions.iter().any(|a| a.to_amount == 600));
}

#[test]
fn a_four_bet_pot_tracks_the_full_commitment() {
    let hand = parse_fixture("four_bet_pot");
    let hero = hand.seat_by_name("hero").expect("hero");
    let villain = hand.seat_by_name("villainCO").expect("co");
    // The hero posted 50, completed to 600, then called 900.
    assert_eq!(hero.contributed, 1500);
    assert_eq!(villain.contributed, 1500);
    assert_eq!(hand.pot_total.minor, 3025);
    assert_eq!(hand.rake.minor, 100);
    let raises = hand
        .actions
        .iter()
        .filter(|a| a.street == Street::Preflop && a.kind == texture::domain::ActionKind::Raise)
        .count();
    assert_eq!(raises, 3, "open, 3-bet and 4-bet");
}

// ---------------------------------------------------------------------------
// Golden files
// ---------------------------------------------------------------------------

#[test]
fn golden_files_match() {
    for path in fixture_files() {
        let name = name_of(&path);
        let golden = fixture_dir().join(format!("{name}.json"));
        let hand = parse_fixture(&name);
        let actual = serde_json::to_value(&hand).expect("serialises");
        let expected: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&golden)
                .unwrap_or_else(|e| panic!("missing golden {}: {e}", golden.display())),
        )
        .expect("golden is valid json");
        assert_eq!(
            actual,
            expected,
            "{name}: the parsed hand differs from the recorded golden. If this change is \
             intended, re-run `cargo test --test parser_golden -- --ignored regenerate_goldens` \
             and review the diff."
        );
    }
}

#[test]
#[ignore = "regenerates the golden files; run deliberately and review the diff"]
fn regenerate_goldens() {
    for path in fixture_files() {
        let name = name_of(&path);
        let hand = parse_fixture(&name);
        let json = serde_json::to_string_pretty(&hand).expect("serialises");
        std::fs::write(fixture_dir().join(format!("{name}.json")), json + "\n")
            .expect("writes golden");
        println!("wrote {name}.json");
    }
}
