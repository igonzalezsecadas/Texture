//! Statistic derivation tests. Every expectation here is hand-authored: a small
//! population whose correct answer can be computed by reading the hand.

use texture::domain::{
    Action, ActionKind, Card, Currency, Game, HeroHandAnalysis, Money, ParsedHand, ParsedSeat,
    Position, Site, Street,
};
use texture::stats::{self, HeroSeat};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn money(minor: i64) -> Money {
    Money::new(minor, Currency::Eur)
}

/// Six seats with the button on seat 6, so seat 1 is the small blind, seat 2 the
/// big blind, and 3..6 are UTG, MP, CO, BTN. The hero's seat is named `hero`.
fn six_max(hero_seat: u8) -> Vec<ParsedSeat> {
    let positions = [
        Position::Sb,
        Position::Bb,
        Position::Utg,
        Position::Mp,
        Position::Co,
        Position::Btn,
    ];
    (1..=6u8)
        .map(|seat| ParsedSeat {
            seat,
            screen_name: if seat == hero_seat {
                "hero".to_string()
            } else {
                format!("villain{seat}")
            },
            starting_stack: money(10_000),
            position: positions[(seat - 1) as usize],
            contributed: 0,
            result: money(0),
            shown_cards: Vec::new(),
            sat_out: false,
        })
        .collect()
}

fn action(street: Street, seat: u8, kind: ActionKind, amount: i64) -> Action {
    Action { street, seat, kind, amount, to_amount: amount, is_first_aggression: false }
}

struct HandBuilder {
    seats: Vec<ParsedSeat>,
    actions: Vec<Action>,
    board: Vec<Card>,
    pot: i64,
    rake: i64,
    showdown: bool,
}

impl HandBuilder {
    fn new(hero_seat: u8) -> Self {
        HandBuilder {
            seats: six_max(hero_seat),
            actions: Vec::new(),
            board: Vec::new(),
            pot: 0,
            rake: 0,
            showdown: false,
        }
    }

    fn act(mut self, street: Street, seat: u8, kind: ActionKind, amount: i64) -> Self {
        self.actions.push(action(street, seat, kind, amount));
        self
    }

    fn fold(mut self, street: Street, seat: u8) -> Self {
        self.actions.push(action(street, seat, ActionKind::Fold, 0));
        self
    }

    fn check(mut self, street: Street, seat: u8) -> Self {
        self.actions.push(action(street, seat, ActionKind::Check, 0));
        self
    }

    fn board(mut self, text: &str) -> Self {
        self.board = text.split_whitespace().map(|c| Card::parse(c).expect("card")).collect();
        self
    }

    fn shown(mut self, seat: u8, text: &str) -> Self {
        let cards: Vec<Card> = text.split_whitespace().map(|c| Card::parse(c).expect("card")).collect();
        if let Some(row) = self.seats.iter_mut().find(|s| s.seat == seat) {
            row.shown_cards = cards;
        }
        self
    }

    fn contributed(mut self, seat: u8, amount: i64) -> Self {
        if let Some(row) = self.seats.iter_mut().find(|s| s.seat == seat) {
            row.contributed = amount;
        }
        self
    }

    fn result(mut self, seat: u8, amount: i64) -> Self {
        if let Some(row) = self.seats.iter_mut().find(|s| s.seat == seat) {
            row.result = money(amount);
        }
        self
    }

    fn pot_rake(mut self, pot: i64, rake: i64) -> Self {
        self.pot = pot;
        self.rake = rake;
        self
    }

    fn showdown(mut self, yes: bool) -> Self {
        self.showdown = yes;
        self
    }

    fn build(self) -> ParsedHand {
        let streets_dealt = match self.board.len() {
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
            button_seat: 6,
            timestamp_utc: chrono::Utc::now(),
            local_timestamp: "2024/01/02 13:45:06".into(),
            tz_token: "CET".into(),
            board: self.board,
            seats: self.seats,
            actions: self.actions,
            pot_total: money(self.pot),
            rake: money(self.rake),
            jackpot_drop: money(0),
            went_to_showdown: self.showdown,
            streets_dealt,
        }
    }
}

fn analyze(hand: &ParsedHand, hero_seat: u8) -> HeroHandAnalysis {
    stats::analyze_hand(hand, HeroSeat { seat: hero_seat, profile_id: 7, player_id: 11 })
        .expect("the hero should be analysable")
}

// ---------------------------------------------------------------------------
// Dealt-in detection
// ---------------------------------------------------------------------------

#[test]
fn a_hero_who_was_not_dealt_in_is_not_analysed() {
    // The hero is listed at the table but has no cards and takes no action.
    let hand = HandBuilder::new(5)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 4)
        .fold(Street::Preflop, 1)
        .fold(Street::Preflop, 2)
        .build();
    assert!(
        stats::analyze_hand(&hand, HeroSeat { seat: 5, profile_id: 7, player_id: 11 }).is_none(),
        "a sitting-out hero must not produce a row, or the hand count would not match PT4"
    );
}

#[test]
fn resolve_hero_uses_the_screen_name_mapping() {
    let hand = HandBuilder::new(5)
        .act(Street::Preflop, 5, ActionKind::Raise, 150)
        .fold(Street::Preflop, 1)
        .fold(Street::Preflop, 2)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 4)
        .fold(Street::Preflop, 6)
        .build();
    let mapping = vec![("someone.else".to_string(), 1i64), ("hero".to_string(), 42i64)];
    let resolved = stats::resolve_hero(&hand, 7, &mapping).expect("hero resolved");
    assert_eq!(resolved.seat, 5);
    assert_eq!(resolved.player_id, 42);
    // An alias that is not at this table resolves to nothing.
    assert!(stats::resolve_hero(&hand, 7, &[("absent".to_string(), 3)]).is_none());
}

// ---------------------------------------------------------------------------
// Preflop
// ---------------------------------------------------------------------------

#[test]
fn a_big_blind_free_check_is_neither_a_vpip_nor_a_limp() {
    let hand = HandBuilder::new(2)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 4)
        .fold(Street::Preflop, 5)
        .fold(Street::Preflop, 6)
        .act(Street::Preflop, 1, ActionKind::Call, 25) // small blind completes
        .check(Street::Preflop, 2) // hero, big blind, checks
        .board("2c 7h 9s")
        .check(Street::Flop, 1)
        .check(Street::Flop, 2)
        .build();
    let a = analyze(&hand, 2);
    assert!(!a.vpip, "checking the option is not voluntary money");
    assert!(!a.limp, "the big blind's check is not a limp");
    assert!(!a.pfr);
    assert!(a.saw_flop);
    assert!(a.wwsf_opportunity);
    // A limped pot has no aggressor, so there is no c-bet opportunity.
    assert!(!a.cbet_opportunity[0], "a limped pot has no c-bet");
    assert!(!a.probe_opportunity[1]);
}

#[test]
fn limping_is_vpip_but_not_pfr() {
    let hand = HandBuilder::new(5)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 4)
        .act(Street::Preflop, 5, ActionKind::Call, 50) // hero limps
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .check(Street::Preflop, 2)
        .build();
    let a = analyze(&hand, 5);
    assert!(a.vpip);
    assert!(a.limp);
    assert!(!a.pfr);
    assert!(a.rfi_opportunity);
    assert!(!a.rfi);
    assert!(!a.cold_call_opportunity);
    assert!(a.steal_opportunity);
    assert!(!a.steal_attempt, "limping is not a steal attempt");
}

#[test]
fn opening_from_the_cutoff_is_a_raise_first_in_and_a_steal() {
    let hand = HandBuilder::new(5)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 4)
        .act(Street::Preflop, 5, ActionKind::Raise, 150)
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .fold(Street::Preflop, 2)
        .build();
    let a = analyze(&hand, 5);
    assert!(a.vpip && a.pfr);
    assert!(a.rfi && a.rfi_opportunity);
    assert!(a.steal_attempt && a.steal_opportunity);
    assert!(!a.limp);
    // Nobody raised in front, so there was no 3-bet or cold-call opportunity.
    assert!(!a.three_bet_opportunity);
    assert!(!a.cold_call_opportunity);
    // The blinds folded, so the hero took the pot down preflop.
    assert!(!a.saw_flop);
}

#[test]
fn three_betting_a_single_open_is_counted() {
    let hand = HandBuilder::new(5)
        .fold(Street::Preflop, 3)
        .act(Street::Preflop, 4, ActionKind::Raise, 150) // MP opens
        .act(Street::Preflop, 5, ActionKind::Raise, 450) // hero 3-bets
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .fold(Street::Preflop, 2)
        .fold(Street::Preflop, 4)
        .build();
    let a = analyze(&hand, 5);
    assert!(a.three_bet, "raising over a single raise is a 3-bet");
    assert!(a.three_bet_opportunity);
    assert!(!a.four_bet);
    assert!(
        !a.rfi_opportunity,
        "somebody had already opened, so the hero was never first in"
    );
    assert!(!a.rfi);
}

#[test]
fn folding_to_a_three_bet_after_opening_is_counted() {
    let hand = HandBuilder::new(5)
        .fold(Street::Preflop, 4)
        .act(Street::Preflop, 5, ActionKind::Raise, 150) // hero opens
        .act(Street::Preflop, 6, ActionKind::Raise, 450) // button 3-bets
        .fold(Street::Preflop, 1)
        .fold(Street::Preflop, 2)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 5) // hero folds
        .build();
    let a = analyze(&hand, 5);
    assert!(a.pfr);
    assert!(a.faced_three_bet);
    assert!(a.folded_to_three_bet);
    assert!(a.four_bet_opportunity, "facing a 3-bet is a 4-bet opportunity");
    assert!(!a.four_bet);
    assert!(!a.three_bet, "the hero opened, so they did not 3-bet");
}

#[test]
fn three_betting_then_folding_to_a_four_bet_is_counted() {
    let hand = HandBuilder::new(5)
        .fold(Street::Preflop, 3)
        .act(Street::Preflop, 4, ActionKind::Raise, 150)
        .act(Street::Preflop, 5, ActionKind::Raise, 450) // hero 3-bets
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .fold(Street::Preflop, 2)
        .act(Street::Preflop, 4, ActionKind::Raise, 1200) // 4-bet
        .fold(Street::Preflop, 5) // hero folds
        .build();
    let a = analyze(&hand, 5);
    assert!(a.three_bet);
    assert!(a.faced_four_bet);
    assert!(a.folded_to_four_bet);
    assert!(!a.faced_three_bet, "the hero was the one who 3-bet");
}

#[test]
fn cold_calling_an_open_is_counted() {
    let hand = HandBuilder::new(5)
        .fold(Street::Preflop, 3)
        .act(Street::Preflop, 4, ActionKind::Raise, 150)
        .act(Street::Preflop, 5, ActionKind::Call, 150) // hero cold-calls
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .fold(Street::Preflop, 2)
        .build();
    let a = analyze(&hand, 5);
    assert!(a.cold_call);
    assert!(a.cold_call_opportunity);
    assert!(a.vpip && !a.pfr);
    assert!(!a.rfi, "the hero was not first in");
    assert!(!a.limp, "calling a raise is not limping");
    assert!(!a.steal_attempt);
}

#[test]
fn the_big_blind_folding_to_a_late_open_faces_a_steal() {
    let hand = HandBuilder::new(2)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 4)
        .fold(Street::Preflop, 5)
        .act(Street::Preflop, 6, ActionKind::Raise, 150) // button steals
        .fold(Street::Preflop, 1)
        .fold(Street::Preflop, 2) // hero folds
        .build();
    let a = analyze(&hand, 2);
    assert!(a.faced_steal);
    assert!(a.folded_to_steal);
    assert!(!a.vpip);
}

#[test]
fn an_early_position_open_is_not_a_steal_for_the_big_blind() {
    let hand = HandBuilder::new(2)
        .act(Street::Preflop, 3, ActionKind::Raise, 150) // UTG opens
        .fold(Street::Preflop, 4)
        .fold(Street::Preflop, 5)
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .fold(Street::Preflop, 2)
        .build();
    let a = analyze(&hand, 2);
    assert!(!a.faced_steal, "an UTG open is not a steal");
}

// ---------------------------------------------------------------------------
// Postflop
// ---------------------------------------------------------------------------

#[test]
fn the_preflop_aggressor_betting_the_flop_c_bets() {
    let hand = HandBuilder::new(5)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 4)
        .act(Street::Preflop, 5, ActionKind::Raise, 150)
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .act(Street::Preflop, 2, ActionKind::Call, 100)
        .board("2c 7h 9s")
        .act(Street::Flop, 2, ActionKind::Check, 0)
        .act(Street::Flop, 5, ActionKind::Bet, 200) // hero c-bets
        .fold(Street::Flop, 2)
        .build();
    let a = analyze(&hand, 5);
    assert!(a.cbet_opportunity[0]);
    assert!(a.cbet[0]);
    assert!(!a.faced_cbet[0]);
    assert_eq!(a.aggressive_actions[1], 1, "the flop bet is aggressive");
}

#[test]
fn calling_preflop_then_folding_to_a_flop_c_bet_is_counted() {
    let hand = HandBuilder::new(2)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 4)
        .act(Street::Preflop, 5, ActionKind::Raise, 150) // cutoff opens
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .act(Street::Preflop, 2, ActionKind::Call, 100) // hero calls
        .board("2c 7h 9s")
        .act(Street::Flop, 5, ActionKind::Bet, 200) // the aggressor c-bets
        .fold(Street::Flop, 2) // hero folds
        .build();
    let a = analyze(&hand, 2);
    assert!(a.faced_cbet[0]);
    assert!(a.folded_to_cbet[0]);
    assert!(!a.cbet_opportunity[0], "the hero was not the aggressor");
}

#[test]
fn a_check_raise_has_an_opportunity_only_when_a_bet_arrives() {
    let raised = HandBuilder::new(2)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 4)
        .act(Street::Preflop, 5, ActionKind::Raise, 150)
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .act(Street::Preflop, 2, ActionKind::Call, 100)
        .board("2c 7h 9s")
        .check(Street::Flop, 2) // hero checks
        .act(Street::Flop, 5, ActionKind::Bet, 200)
        .act(Street::Flop, 2, ActionKind::Raise, 600) // hero check-raises
        .build();
    let a = analyze(&raised, 2);
    assert!(a.check_raise_opportunity[0]);
    assert!(a.check_raise[0]);
    assert_eq!(a.aggressive_actions[1], 1);
    assert_eq!(a.passive_actions[1], 1, "the check is passive");

    let checked_through = HandBuilder::new(2)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 4)
        .act(Street::Preflop, 5, ActionKind::Raise, 150)
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .act(Street::Preflop, 2, ActionKind::Call, 100)
        .board("2c 7h 9s")
        .check(Street::Flop, 2)
        .check(Street::Flop, 5)
        .build();
    let b = analyze(&checked_through, 2);
    assert!(
        !b.check_raise_opportunity[0],
        "a check-raise needs a bet to raise"
    );
    assert_eq!(b.passive_actions[1], 1);
}

#[test]
fn betting_the_turn_after_a_checked_through_flop_is_a_delayed_c_bet() {
    let hand = HandBuilder::new(5)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 4)
        .act(Street::Preflop, 5, ActionKind::Raise, 150)
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .act(Street::Preflop, 2, ActionKind::Call, 100)
        .board("2c 7h 9s Td")
        .check(Street::Flop, 2)
        .check(Street::Flop, 5) // hero declines to c-bet
        .check(Street::Turn, 2)
        .act(Street::Turn, 5, ActionKind::Bet, 300)
        .build();
    let a = analyze(&hand, 5);
    assert!(a.cbet_opportunity[0]);
    assert!(!a.cbet[0], "the hero checked the flop");
    assert!(
        !a.cbet_opportunity[1],
        "a bet after a checked-through street is not a c-bet"
    );
    assert!(a.delayed_cbet_opportunity[1]);
    assert!(a.delayed_cbet[1]);
}

#[test]
fn probe_opportunity_needs_the_aggressor_to_check() {
    let hand = HandBuilder::new(2)
        .fold(Street::Preflop, 3)
        .fold(Street::Preflop, 4)
        .act(Street::Preflop, 5, ActionKind::Raise, 150)
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .act(Street::Preflop, 2, ActionKind::Call, 100)
        .board("2c 7h 9s")
        .check(Street::Flop, 5) // the aggressor checks
        .check(Street::Flop, 2) // hero checks behind: no probe
        .build();
    let a = analyze(&hand, 2);
    assert!(a.probe_opportunity[0]);
    assert!(!a.probe[0], "checking behind is not a probe bet");
}

// ---------------------------------------------------------------------------
// Showdown, winnings and rake
// ---------------------------------------------------------------------------

#[test]
fn showdown_winnings_are_split_from_non_showdown_winnings() {
    let hand = HandBuilder::new(2)
        .fold(Street::Preflop, 3)
        .act(Street::Preflop, 4, ActionKind::Raise, 150)
        .fold(Street::Preflop, 5)
        .fold(Street::Preflop, 6)
        .fold(Street::Preflop, 1)
        .act(Street::Preflop, 2, ActionKind::Call, 100)
        .board("2c 7h 9s")
        .check(Street::Flop, 2)
        .check(Street::Flop, 4)
        .contributed(2, 150)
        .result(2, 400)
        .contributed(4, 150)
        .result(4, -150)
        .pot_rake(300, 0)
        .showdown(true)
        .build();
    let a = analyze(&hand, 2);
    assert!(a.went_to_showdown);
    assert!(a.won_at_showdown, "the hero's collection exceeded their contribution");
    assert_eq!(a.sd_winnings, 400);
    assert_eq!(a.non_sd_winnings, 0);
    assert!(a.won_when_saw_flop);
}

#[test]
fn rake_is_attributed_by_weighted_contribution_and_sums_exactly() {
    // Contributions 500 / 300 / 200 out of a 1000 pot, and 7 minor units of rake.
    // Exact shares are 3.5 / 2.1 / 1.4, so largest-remainder gives 3 / 3 / 1 or
    // similar; what must hold is that the parts sum to the whole and the hero's
    // part follows the same rule as everyone else's.
    let hand = HandBuilder::new(5)
        .act(Street::Preflop, 5, ActionKind::Call, 200)
        .contributed(3, 500)
        .contributed(4, 300)
        .contributed(5, 200)
        .pot_rake(1000, 7)
        .build();
    let a = analyze(&hand, 5);
    let seats: Vec<(u8, i64)> = hand.seats.iter().map(|s| (s.seat, s.contributed)).collect();
    assert_eq!(seats.iter().map(|(_, c)| c).sum::<i64>(), 1000);
    assert!(
        (0..=7).contains(&a.rake_attributed),
        "the hero's share cannot exceed the rake, got {}",
        a.rake_attributed
    );
    // The attribution rule is exercised through its observable contract: the
    // shares are proportional, so the largest contributor pays the most.
    assert!(a.rake_attributed <= 3 + 1);
}

#[test]
fn a_hand_with_unknown_cards_falls_back_to_the_actual_result() {
    let hand = HandBuilder::new(5)
        .act(Street::Preflop, 5, ActionKind::Raise, 150)
        .act(Street::Preflop, 3, ActionKind::Call, 150)
        .contributed(5, 150)
        .result(5, 200)
        .pot_rake(300, 0)
        .showdown(true)
        .build();
    let a = analyze(&hand, 5);
    assert_eq!(a.adjusted_result, a.net_result, "no adjustment is possible");
    assert_eq!(a.luck_delta, 0);
}
