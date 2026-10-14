//! All-in EV tests. Every expectation here is either computed by hand or
//! asserted as a documented range where enumeration is impractical.

use proptest::prelude::*;
use std::collections::BTreeSet;
use texture::domain::{Action, ActionKind, Card, Currency, Game, Money, ParsedHand, ParsedSeat, Position, Site, Street};
use texture::ev;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn card(text: &str) -> Card {
    Card::parse(text).unwrap_or_else(|| panic!("bad card {text}"))
}

fn cards(text: &str) -> Vec<Card> {
    text.split_whitespace().map(card).collect()
}

fn money(minor: i64) -> Money {
    Money::new(minor, Currency::Eur)
}

fn seat(seat: u8, name: &str, stack: i64, contributed: i64, shown: &str) -> ParsedSeat {
    ParsedSeat {
        seat,
        screen_name: name.to_string(),
        starting_stack: money(stack),
        position: Position::Mp,
        contributed,
        result: money(0),
        shown_cards: cards(shown),
        sat_out: false,
    }
}

fn action(street: Street, seat: u8, kind: ActionKind, amount: i64) -> Action {
    Action { street, seat, kind, amount, to_amount: amount, is_first_aggression: false }
}

fn build_hand(
    seats: Vec<ParsedSeat>,
    actions: Vec<Action>,
    board: &str,
    pot: i64,
    rake: i64,
    showdown: bool,
) -> ParsedHand {
    let board_cards = cards(board);
    let streets_dealt = match board_cards.len() {
        0 => vec![Street::Preflop],
        3 => vec![Street::Preflop, Street::Flop],
        4 => vec![Street::Preflop, Street::Flop, Street::Turn],
        _ => vec![Street::Preflop, Street::Flop, Street::Turn, Street::River],
    };
    ParsedHand {
        site: Site::PokerStars,
        hand_id: "1".into(),
        game: Game::NoLimitHoldem,
        currency: Currency::Eur,
        small_blind: money(25),
        big_blind: money(50),
        table_name: "Test".into(),
        table_size: 6,
        button_seat: 1,
        timestamp_utc: chrono::Utc::now(),
        local_timestamp: "2024/01/02 13:45:06".into(),
        tz_token: "CET".into(),
        board: board_cards,
        seats,
        actions,
        pot_total: money(pot),
        rake: money(rake),
        jackpot_drop: money(0),
        went_to_showdown: showdown,
        streets_dealt,
    }
}

fn category(score: u32) -> u32 {
    score >> 20
}

// ---------------------------------------------------------------------------
// Hand evaluator
// ---------------------------------------------------------------------------

#[test]
fn evaluator_ranks_categories_in_order() {
    // Every hand below is a distinct, unambiguous category in 7 cards.
    let straight_flush = eval("As Ks Qs Js Ts 2h 3d");
    let quads = eval("Ah Ad Ac As Kh Kd 2c");
    let full_house = eval("Ah Ad Ac Kh Kd Ks 2c");
    let flush = eval("Ah Kh 9h 7h 5h 2c 3d");
    let straight = eval("9h 8d 7c 6s 5h Ah 2d");
    let trips = eval("Ah Ad Ac 9h 7d 5c 2d");
    let two_pair = eval("Ah Ad Kh Kd 9c 7s 2h");
    let pair = eval("Ah Ad Kh 9d 7c 5s 2h");
    let high_card = eval("Ah Kd Qh 9d 7c 5s 2h");

    assert_eq!(category(straight_flush), 8);
    assert_eq!(category(quads), 7);
    assert_eq!(category(full_house), 6);
    assert_eq!(category(flush), 5);
    assert_eq!(category(straight), 4);
    assert_eq!(category(trips), 3);
    assert_eq!(category(two_pair), 2);
    assert_eq!(category(pair), 1);
    assert_eq!(category(high_card), 0);

    let ordered = [
        high_card, pair, two_pair, trips, straight, flush, full_house, quads, straight_flush,
    ];
    for window in ordered.windows(2) {
        assert!(window[0] < window[1], "category ordering must be strict");
    }
}

fn eval(text: &str) -> u32 {
    // "As Ks Qs Js Ts" style: two hole cards plus five board cards.
    let all = cards(text);
    ev::eval_best(&all)
}

#[test]
fn evaluator_handles_the_wheel_and_kickers() {
    // A-2-3-4-5 is a five-high straight, not an ace-high hand.
    assert_eq!(category(ev::eval_best(&cards("Ah 2d 3c 4s 5h Kd 9c"))), 4);
    // The wheel loses to a six-high straight.
    assert!(ev::eval_best(&cards("Ah 2d 3c 4s 5h Kd 9c")) < ev::eval_best(&cards("6h 2d 3c 4s 5h Kd 9c")));
    // A third pair is a better kicker than any lower single card.
    assert!(
        ev::eval_best(&cards("Ah Ad Kh Kd 9c 9s 2h")) > ev::eval_best(&cards("Ah Ad Kh Kd 8c 7s 2h")),
        "the nine from the third pair must play as the kicker"
    );
    // A double-trips hand is a full house.
    assert_eq!(category(ev::eval_best(&cards("Ah Ad Ac Kh Kd Ks 2c"))), 6);
}

// ---------------------------------------------------------------------------
// Side pots
// ---------------------------------------------------------------------------

#[test]
fn side_pots_split_by_level() {
    let pots = ev::side_pots(&[(1, 100), (2, 100), (3, 50)]);
    assert_eq!(
        pots,
        vec![
            ev::SidePot { amount: 150, eligible: vec![1, 2, 3] },
            ev::SidePot { amount: 100, eligible: vec![1, 2] },
        ]
    );
    let total: i64 = pots.iter().map(|p| p.amount).sum();
    assert_eq!(total, 250, "every chip is in exactly one pot");
}

#[test]
fn folded_money_is_dead_but_counted() {
    let pots = ev::side_pots_with_folded(&[(1, 100), (2, 100), (3, 50)], &[3]);
    assert_eq!(pots.iter().map(|p| p.amount).sum::<i64>(), 250);
    assert!(
        pots.iter().all(|p| !p.eligible.contains(&3)),
        "a folded seat can never win a pot it is dead money in"
    );
}

#[test]
fn a_short_all_in_is_capped_by_its_own_level() {
    let pots = ev::side_pots(&[(1, 100), (2, 60)]);
    assert_eq!(
        pots,
        vec![
            ev::SidePot { amount: 120, eligible: vec![1, 2] },
            ev::SidePot { amount: 40, eligible: vec![1] },
        ]
    );
}

proptest! {
    #[test]
    fn side_pots_conserve_chips(contribs in prop::collection::vec((0u8..6u8, 0i64..500i64), 1..6)) {
        // De-duplicate seats; a seat contributes once.
        let mut seen = BTreeSet::new();
        let unique: Vec<(u8, i64)> = contribs
            .into_iter()
            .filter(|(s, _)| seen.insert(*s))
            .collect();
        let pots = ev::side_pots(&unique);
        let total: i64 = pots.iter().map(|p| p.amount).sum();
        let contributed: i64 = unique.iter().map(|(_, c)| *c).sum();
        prop_assert_eq!(total, contributed);
        for pot in &pots {
            prop_assert!(pot.amount > 0);
            prop_assert!(!pot.eligible.is_empty());
        }
    }
}

// ---------------------------------------------------------------------------
// Equity
// ---------------------------------------------------------------------------

#[test]
fn equity_is_exact_when_the_board_is_complete() {
    let hands = vec![cards("As Ad"), cards("Ks Kd")];
    let board = cards("2c 7h 9s Jd 3c");
    let equity = ev::equities(&hands, &board);
    assert_eq!(equity, vec![1.0, 0.0], "aces beat kings on a blank run-out");

    // Reversed: the kings are ahead here.
    let board = cards("Kc 7h 9s Jd 3c");
    let equity = ev::equities(&hands, &board);
    assert_eq!(equity, vec![0.0, 1.0], "a set of kings beats a pair of aces");
}

#[test]
fn equity_splits_when_the_board_plays() {
    // Both players play the board's six-high straight.
    let hands = vec![cards("As Kd"), cards("Ah Qd")];
    let board = cards("2c 3d 4h 5s 6c");
    let equity = ev::equities(&hands, &board);
    assert_eq!(equity, vec![0.5, 0.5]);
    assert!((equity.iter().sum::<f64>() - 1.0).abs() < 1e-12);
}

#[test]
fn equity_is_exact_on_a_flop_all_in() {
    // Hero has top set; villain has a flush draw. The turn and river are the only
    // unknowns, so this is enumerated exactly.
    let hands = vec![cards("Ac Ah"), cards("Ks Qs")];
    let board = cards("Ad 7s 2s");
    let equity = ev::equities(&hands, &board);
    let total: f64 = equity.iter().sum();
    assert!((total - 1.0).abs() < 1e-12, "equities sum to one, got {total}");
    assert!(
        equity[0] > 0.6 && equity[0] < 0.8,
        "top set against a flush draw is a documented favourite, got {}",
        equity[0]
    );
}

#[test]
fn preflop_equity_is_deterministic_and_in_range() {
    let hands = vec![cards("As Ad"), cards("Ks Kd")];
    let first = ev::equities(&hands, &[]);
    let second = ev::equities(&hands, &[]);
    assert_eq!(first, second, "the sampled equity must be reproducible");
    assert!(
        first[0] > 0.78 && first[0] < 0.86,
        "aces against kings is around 82%, got {}",
        first[0]
    );
    assert!((first.iter().sum::<f64>() - 1.0).abs() < 1e-9);
}

proptest! {
    #[test]
    fn equity_always_sums_to_one(
        a_rank in 2u8..15,
        b_rank in 2u8..15,
    ) {
        // Two fixed, non-colliding hands, complete board: exact and cheap.
        prop_assume!(a_rank != b_rank);
        // Keep the fixed board ranks out of the hole cards so no card repeats.
        prop_assume!(![4, 5, 6, 8, 9].contains(&a_rank));
        prop_assume!(![4, 5, 6, 8, 9].contains(&b_rank));
        prop_assume!(a_rank != 2 && b_rank != 3);
        let hands = vec![
            vec![Card { rank: a_rank, suit: 's' }, Card { rank: 2, suit: 'h' }],
            vec![Card { rank: b_rank, suit: 'd' }, Card { rank: 3, suit: 'c' }],
        ];
        let board = cards("4s 5s 6d 8h 9c");
        let equity = ev::equities(&hands, &board);
        let total: f64 = equity.iter().sum();
        prop_assert!((total - 1.0).abs() < 1e-12);
        prop_assert!(equity.iter().all(|e| (0.0..=1.0).contains(e)));
    }
}

// ---------------------------------------------------------------------------
// All-in adjusted result
// ---------------------------------------------------------------------------

#[test]
fn no_adjustment_without_a_showdown() {
    let hand = build_hand(
        vec![seat(1, "hero", 1000, 1000, "As Ad"), seat(2, "villain", 1000, 1000, "")],
        vec![action(Street::River, 1, ActionKind::Bet, 1000)],
        "2c 7h 9s Jd 3c",
        2000,
        0,
        false,
    );
    assert_eq!(ev::all_in_adjusted_result(&hand, 1), None);
}

#[test]
fn no_adjustment_when_the_hero_is_not_all_in() {
    let hand = build_hand(
        vec![seat(1, "hero", 1000, 400, "As Ad"), seat(2, "villain", 1000, 400, "Ks Kd")],
        vec![action(Street::River, 1, ActionKind::Bet, 400)],
        "2c 7h 9s Jd 3c",
        800,
        0,
        true,
    );
    assert_eq!(ev::all_in_adjusted_result(&hand, 1), None);
}

#[test]
fn no_adjustment_when_an_eligible_opponent_mucked() {
    let hand = build_hand(
        vec![seat(1, "hero", 1000, 1000, "As Ad"), seat(2, "villain", 1000, 1000, "")],
        vec![action(Street::River, 1, ActionKind::Bet, 1000)],
        "2c 7h 9s Jd 3c",
        2000,
        0,
        true,
    );
    assert_eq!(
        ev::all_in_adjusted_result(&hand, 1),
        None,
        "unknown opponent cards mean no adjustment is possible"
    );
}

#[test]
fn no_adjustment_when_the_hero_folded() {
    let hand = build_hand(
        vec![seat(1, "hero", 1000, 1000, "As Ad"), seat(2, "villain", 1000, 1000, "Ks Kd")],
        vec![
            action(Street::River, 1, ActionKind::Fold, 0),
            action(Street::River, 2, ActionKind::Bet, 1000),
        ],
        "2c 7h 9s Jd 3c",
        2000,
        0,
        true,
    );
    assert_eq!(ev::all_in_adjusted_result(&hand, 1), None);
}

#[test]
fn two_way_all_in_on_the_river_is_exact() {
    // Hero is all-in for 1000 and certain to win a 2000 pot with no rake, so the
    // adjusted result is the whole pot minus what the hero put in.
    let hand = build_hand(
        vec![seat(1, "hero", 1000, 1000, "As Ad"), seat(2, "villain", 1000, 1000, "Ks Kd")],
        vec![action(Street::River, 1, ActionKind::Bet, 1000)],
        "2c 7h 9s Jd 3c",
        2000,
        0,
        true,
    );
    assert_eq!(ev::all_in_adjusted_result(&hand, 1), Some(1000));

    // Losing the same all-in adjusts to the full loss.
    let hand = build_hand(
        vec![seat(1, "hero", 1000, 1000, "Qs Qd"), seat(2, "villain", 1000, 1000, "Ks Kd")],
        vec![action(Street::River, 1, ActionKind::Bet, 1000)],
        "2c 7h 9s Jd 3c",
        2000,
        0,
        true,
    );
    assert_eq!(ev::all_in_adjusted_result(&hand, 1), Some(-1000));
}

#[test]
fn rake_is_deducted_from_the_awardable_pot() {
    // Same certainty, but 100 minor units of rake leaves 1900 to be awarded.
    let hand = build_hand(
        vec![seat(1, "hero", 1000, 1000, "As Ad"), seat(2, "villain", 1000, 1000, "Ks Kd")],
        vec![action(Street::River, 1, ActionKind::Bet, 1000)],
        "2c 7h 9s Jd 3c",
        2000,
        100,
        true,
    );
    assert_eq!(ev::all_in_adjusted_result(&hand, 1), Some(900));
}

#[test]
fn three_way_all_in_builds_the_side_pot_hero_is_capped_at() {
    let hand = build_hand(
        vec![
            seat(1, "hero", 500, 500, "As Ad"),
            seat(2, "villainA", 1000, 1000, "Ks Kd"),
            seat(3, "villainB", 300, 300, "Qs Qd"),
        ],
        vec![action(Street::River, 1, ActionKind::Bet, 500)],
        "2c 7h 9s Jd 3c",
        1800,
        0,
        true,
    );

    let pots = ev::side_pots_with_folded(
        &[(1, 500), (2, 1000), (3, 300)],
        &[],
    );
    assert_eq!(
        pots,
        vec![
            ev::SidePot { amount: 900, eligible: vec![1, 2, 3] },
            ev::SidePot { amount: 400, eligible: vec![1, 2] },
            ev::SidePot { amount: 500, eligible: vec![2] },
        ],
        "the hero is capped at the 500 level"
    );

    // Aces win everything the hero is eligible for: 900 + 400 = 1300, minus the
    // 500 the hero put in.
    assert_eq!(ev::all_in_adjusted_result(&hand, 1), Some(800));
}

#[test]
fn unknown_hero_cards_mean_no_adjustment() {
    let hand = build_hand(
        vec![seat(1, "hero", 1000, 1000, ""), seat(2, "villain", 1000, 1000, "Ks Kd")],
        vec![action(Street::River, 1, ActionKind::Bet, 1000)],
        "2c 7h 9s Jd 3c",
        2000,
        0,
        true,
    );
    assert_eq!(ev::all_in_adjusted_result(&hand, 1), None);
}
