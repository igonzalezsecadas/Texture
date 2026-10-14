//! All-in EV. Computed once at import time for hands where the hero was all-in
//! with known cards, including side pots and multiway eligibility.
//!
//! The result is stored per hand as `adjusted_result_minor`; nothing
//! street-by-street is computed. Reconciliation against PokerTracker 4 depends
//! on a deterministic, documented rounding convention.
//!
//! Documented conventions (all of these are deliberate and are candidates for
//! the known-divergence list):
//!
//! - **Point of all-in.** Equity is fixed at the last street on which a
//!   contending seat committed its last chip. The board is only the part that
//!   was known at that moment: preflop 0 cards, flop 3, turn 4, river 5.
//! - **Exhaustive vs sampled.** When the number of unknown-board combinations is
//!   at most [`EXHAUSTIVE_LIMIT`], every run-out is enumerated and the equity is
//!   exact. Otherwise a deterministic Monte Carlo sample of
//!   [`MC_ITERATIONS`] run-outs is used with the fixed seed [`MC_SEED`], so the
//!   same hand always produces the same number.
//! - **Rake.** Equity is applied to the raked, awardable pot: the gross pot is
//!   scaled down by the rake and any jackpot drop, with side pots sharing the
//!   deduction in proportion to their size. This keeps `luck_delta`
//!   (`net_result - adjusted_result`) comparable to `net_result`, which is also
//!   net of rake.
//! - **Rounding.** The final adjusted result is rounded to the nearest minor
//!   unit, half away from zero.

use crate::domain::{ActionKind, Card, ParsedHand, Street};
use std::collections::BTreeMap;

/// Unknown-board combinations above this are sampled instead of enumerated.
pub const EXHAUSTIVE_LIMIT: u64 = 400_000;
/// Deterministic Monte Carlo sample size.
pub const MC_ITERATIONS: u64 = 100_000;
/// Fixed seed for the deterministic Monte Carlo fallback.
pub const MC_SEED: u64 = 0x9E37_79B9_7F4A_7C15;

/// One side pot: the amount in it and the seats eligible to win it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidePot {
    pub amount: i64,
    pub eligible: Vec<u8>,
}

/// Split contributions into main and side pots, from the main pot outward.
///
/// Every contributor is assumed to be contending, so every contributor is
/// eligible at the levels it reached. Use [`side_pots_with_folded`] when some
/// contributors folded and their chips are merely dead money.
pub fn side_pots(contributions: &[(u8, i64)]) -> Vec<SidePot> {
    side_pots_with_folded(contributions, &[])
}

/// As [`side_pots`], but seats listed in `folded` put chips in the pot without
/// being eligible to win any of it.
pub fn side_pots_with_folded(contributions: &[(u8, i64)], folded: &[u8]) -> Vec<SidePot> {
    let mut levels: Vec<i64> = contributions
        .iter()
        .map(|(_, c)| *c)
        .filter(|c| *c > 0)
        .collect();
    levels.sort_unstable();
    levels.dedup();

    let mut pots = Vec::new();
    let mut previous = 0i64;
    for level in levels {
        let contributors: Vec<u8> = contributions
            .iter()
            .filter(|(_, c)| *c >= level)
            .map(|(seat, _)| *seat)
            .collect();
        let amount = (level - previous) * contributors.len() as i64;
        if amount > 0 {
            let mut eligible: Vec<u8> = contributors
                .iter()
                .copied()
                .filter(|seat| !folded.contains(seat))
                .collect();
            eligible.sort_unstable();
            if !eligible.is_empty() {
                pots.push(SidePot { amount, eligible });
            }
        }
        previous = level;
    }
    pots
}

// ---------------------------------------------------------------------------
// Hand evaluation
// ---------------------------------------------------------------------------

const CAT_HIGH_CARD: u32 = 0;
const CAT_PAIR: u32 = 1;
const CAT_TWO_PAIR: u32 = 2;
const CAT_TRIPS: u32 = 3;
const CAT_STRAIGHT: u32 = 4;
const CAT_FLUSH: u32 = 5;
const CAT_FULL_HOUSE: u32 = 6;
const CAT_QUADS: u32 = 7;
const CAT_STRAIGHT_FLUSH: u32 = 8;

fn suit_index(suit: char) -> usize {
    match suit {
        'c' => 0,
        'd' => 1,
        'h' => 2,
        _ => 3,
    }
}

/// Pack a category and up to five descending tiebreak ranks into one comparable
/// score. Higher is better.
fn encode(category: u32, ranks: &[u8]) -> u32 {
    let mut value = category << 20;
    for i in 0..5 {
        let rank = ranks.get(i).copied().unwrap_or(0) as u32;
        value |= rank << (16 - 4 * i);
    }
    value
}

/// The high card of the best straight in `present`, with the ace playing low for
/// the wheel.
fn straight_high(present: &[bool; 15]) -> Option<u8> {
    for high in (5..=14u8).rev() {
        if (0..5).all(|i| present[(high - i) as usize]) {
            return Some(high);
        }
    }
    if present[14] && present[2] && present[3] && present[4] && present[5] {
        return Some(5);
    }
    None
}

/// Score a five-to-seven card hand. Exhaustive by construction: it inspects the
/// rank and suit multisets directly rather than enumerating five-card subsets.
pub fn eval_best(cards: &[Card]) -> u32 {
    let mut rank_count = [0u8; 15];
    let mut suit_count = [0u8; 4];
    let mut suit_rank = [[false; 15]; 4];
    for card in cards {
        let r = card.rank as usize;
        if !(2..=14).contains(&r) {
            continue;
        }
        rank_count[r] += 1;
        let s = suit_index(card.suit);
        suit_count[s] += 1;
        suit_rank[s][r] = true;
    }

    // Straight flush.
    for s in 0..4 {
        if suit_count[s] >= 5 {
            if let Some(high) = straight_high(&suit_rank[s]) {
                return encode(CAT_STRAIGHT_FLUSH, &[high]);
            }
        }
    }

    // Four of a kind.
    for r in (2..=14u8).rev() {
        if rank_count[r as usize] == 4 {
            let kicker = (2..=14u8)
                .rev()
                .find(|k| *k != r && rank_count[*k as usize] > 0)
                .unwrap_or(0);
            return encode(CAT_QUADS, &[r, kicker]);
        }
    }

    // Full house: the highest trips, then the best remaining pair (which may be
    // the second set of trips in a double-trips hand).
    let trips: Vec<u8> = (2..=14u8).rev().filter(|r| rank_count[*r as usize] >= 3).collect();
    if let Some(&top_trips) = trips.first() {
        if let Some(pair) = (2..=14u8)
            .rev()
            .find(|r| *r != top_trips && rank_count[*r as usize] >= 2)
        {
            return encode(CAT_FULL_HOUSE, &[top_trips, pair]);
        }
    }

    // Flush.
    for s in 0..4 {
        if suit_count[s] >= 5 {
            let top: Vec<u8> = (2..=14u8).rev().filter(|r| suit_rank[s][*r as usize]).take(5).collect();
            return encode(CAT_FLUSH, &top);
        }
    }

    // Straight.
    let mut present = [false; 15];
    for r in 2..=14u8 {
        present[r as usize] = rank_count[r as usize] > 0;
    }
    if let Some(high) = straight_high(&present) {
        return encode(CAT_STRAIGHT, &[high]);
    }

    // Three of a kind.
    if let Some(&top_trips) = trips.first() {
        let kickers: Vec<u8> = (2..=14u8)
            .rev()
            .filter(|r| *r != top_trips && rank_count[*r as usize] > 0)
            .take(2)
            .collect();
        return encode(
            CAT_TRIPS,
            &[
                top_trips,
                kickers.first().copied().unwrap_or(0),
                kickers.get(1).copied().unwrap_or(0),
            ],
        );
    }

    // Two pair.
    let pairs: Vec<u8> = (2..=14u8).rev().filter(|r| rank_count[*r as usize] == 2).collect();
    if pairs.len() >= 2 {
        // With a third pair, that pair is also the best available kicker.
        let kicker = (2..=14u8)
            .rev()
            .find(|r| *r != pairs[0] && *r != pairs[1] && rank_count[*r as usize] > 0)
            .unwrap_or(0);
        return encode(CAT_TWO_PAIR, &[pairs[0], pairs[1], kicker]);
    }

    // One pair.
    if let Some(&pair) = pairs.first() {
        let kickers: Vec<u8> = (2..=14u8)
            .rev()
            .filter(|r| *r != pair && rank_count[*r as usize] > 0)
            .take(3)
            .collect();
        return encode(
            CAT_PAIR,
            &[
                pair,
                kickers.first().copied().unwrap_or(0),
                kickers.get(1).copied().unwrap_or(0),
                kickers.get(2).copied().unwrap_or(0),
            ],
        );
    }

    let top: Vec<u8> = (2..=14u8).rev().filter(|r| rank_count[*r as usize] > 0).take(5).collect();
    encode(CAT_HIGH_CARD, &top)
}

// ---------------------------------------------------------------------------
// Equity
// ---------------------------------------------------------------------------

fn full_deck() -> Vec<Card> {
    let mut deck = Vec::with_capacity(52);
    for rank in 2..=14u8 {
        for suit in ['c', 'd', 'h', 's'] {
            deck.push(Card { rank, suit });
        }
    }
    deck
}

fn binomial(n: u64, k: u64) -> u64 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut result: u64 = 1;
    for i in 0..k {
        // Each intermediate value is C(n, i+1), so the division is exact.
        result = result.saturating_mul(n - i) / (i + 1);
    }
    result
}

/// Call `f` with each ascending index combination of size `k` from `0..n`.
fn for_each_combination(n: usize, k: usize, mut f: impl FnMut(&[usize])) {
    if k > n {
        return;
    }
    if k == 0 {
        f(&[]);
        return;
    }
    let mut idx: Vec<usize> = (0..k).collect();
    loop {
        f(&idx);
        let mut i = k;
        loop {
            if i == 0 {
                return;
            }
            i -= 1;
            if idx[i] != i + n - k {
                idx[i] += 1;
                for j in (i + 1)..k {
                    idx[j] = idx[j - 1] + 1;
                }
                break;
            }
        }
    }
}

/// A deterministic xorshift64* generator, so a sampled equity is reproducible.
struct XorShift(u64);

impl XorShift {
    fn new(seed: u64) -> Self {
        XorShift(seed | 1)
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Equity per entry of `hands` against a known or partial `board`, from the
/// point of all-in. Values sum to 1.0. Ties split the pot evenly.
///
/// Every entry of `hands` must have at least two known cards; callers are
/// responsible for only asking when the cards are known.
pub fn equities(hands: &[Vec<Card>], board: &[Card]) -> Vec<f64> {
    let n = hands.len();
    if n == 0 {
        return Vec::new();
    }
    if hands.iter().any(|h| h.len() < 2) {
        return vec![0.0; n];
    }

    let board = &board[..board.len().min(5)];
    let mut known: Vec<Card> = board.to_vec();
    for h in hands {
        known.extend(h.iter().copied());
    }
    let unknown: Vec<Card> = full_deck()
        .into_iter()
        .filter(|c| !known.iter().any(|k| k.rank == c.rank && k.suit == c.suit))
        .collect();

    let needed = 5usize.saturating_sub(board.len());
    if needed > unknown.len() {
        return vec![0.0; n];
    }

    let mut share = vec![0.0f64; n];
    // Reused buffers keep the hot loop allocation-free.
    let mut full: Vec<Card> = Vec::with_capacity(5);
    let mut player_cards: Vec<Card> = Vec::with_capacity(7);
    let mut scores: Vec<u32> = vec![0; n];

    let mut settle = |board_cards: &[Card], share: &mut [f64]| {
        for (i, hole) in hands.iter().enumerate() {
            player_cards.clear();
            player_cards.extend_from_slice(hole);
            player_cards.extend_from_slice(board_cards);
            scores[i] = eval_best(&player_cards);
        }
        let best = *scores.iter().max().unwrap_or(&0);
        let winners = scores.iter().filter(|s| **s == best).count().max(1);
        let each = 1.0 / winners as f64;
        for (i, s) in scores.iter().enumerate() {
            if *s == best {
                share[i] += each;
            }
        }
    };

    let combos = binomial(unknown.len() as u64, needed as u64);
    if combos <= EXHAUSTIVE_LIMIT {
        let mut count: u64 = 0;
        let unknown_local = unknown.clone();
        for_each_combination(unknown.len(), needed, |idx| {
            full.clear();
            full.extend_from_slice(board);
            for i in idx {
                full.push(unknown_local[*i]);
            }
            settle(&full, &mut share);
            count += 1;
        });
        if count > 0 {
            for s in share.iter_mut() {
                *s /= count as f64;
            }
        }
        return share;
    }

    // Deterministic Monte Carlo fallback.
    let mut rng = XorShift::new(MC_SEED);
    let mut deck = unknown.clone();
    for _ in 0..MC_ITERATIONS {
        for i in 0..needed {
            let j = i + (rng.next_u64() % (deck.len() - i) as u64) as usize;
            deck.swap(i, j);
        }
        full.clear();
        full.extend_from_slice(board);
        full.extend_from_slice(&deck[..needed]);
        settle(&full, &mut share);
    }
    for s in share.iter_mut() {
        *s /= MC_ITERATIONS as f64;
    }
    share
}

// ---------------------------------------------------------------------------
// Point of all-in
// ---------------------------------------------------------------------------

/// The board as it was known on `street`.
fn board_at_street(hand: &ParsedHand, street: Street) -> Vec<Card> {
    let count = match street {
        Street::Preflop => 0,
        Street::Flop => 3,
        Street::Turn => 4,
        Street::River => 5,
    };
    hand.board.iter().copied().take(count).collect()
}

/// Seats that folded during the hand.
fn folded_seats(hand: &ParsedHand) -> Vec<u8> {
    let mut folded: Vec<u8> = hand
        .actions
        .iter()
        .filter(|a| a.kind == ActionKind::Fold)
        .map(|a| a.seat)
        .collect();
    folded.sort_unstable();
    folded.dedup();
    folded
}

/// The last street on which a contending seat committed its final chip.
fn all_in_street(hand: &ParsedHand, contending: &[u8]) -> Option<Street> {
    let starts: BTreeMap<u8, i64> =
        hand.seats.iter().map(|s| (s.seat, s.starting_stack.minor)).collect();
    let mut cumulative: BTreeMap<u8, i64> = BTreeMap::new();
    let mut last: Option<Street> = None;
    for action in &hand.actions {
        let entry = cumulative.entry(action.seat).or_insert(0);
        match action.kind {
            ActionKind::Fold | ActionKind::Check | ActionKind::SitOut => {}
            // A returned uncalled bet never reached the pot.
            ActionKind::ReturnUncalled => *entry -= action.amount,
            _ => *entry += action.amount,
        }
        if !contending.contains(&action.seat) {
            continue;
        }
        if let Some(start) = starts.get(&action.seat) {
            if *start > 0 && *entry >= *start {
                last = Some(action.street);
            }
        }
    }
    last
}

// ---------------------------------------------------------------------------
// All-in adjusted result
// ---------------------------------------------------------------------------

/// The hero's all-in adjusted net result for one hand, in minor units, or `None`
/// when no adjustment is possible.
///
/// `None` is returned when: the hero folded; the hero was not all-in; the hand
/// did not reach showdown; the hero's own cards are unknown; an eligible
/// opponent's cards are unknown; or no side pot could be constructed.
pub fn all_in_adjusted_result(hand: &ParsedHand, hero_seat: u8) -> Option<i64> {
    let hero = hand.seat(hero_seat)?;
    if !hand.went_to_showdown {
        return None;
    }
    let folded = folded_seats(hand);
    if folded.contains(&hero_seat) {
        return None;
    }
    // The hero must have committed their entire stack.
    if hero.starting_stack.minor <= 0 || hero.contributed < hero.starting_stack.minor {
        return None;
    }
    if hero.shown_cards.len() < 2 {
        return None;
    }

    let contending: Vec<u8> = hand
        .seats
        .iter()
        .map(|s| s.seat)
        .filter(|s| !folded.contains(s))
        .collect();

    let contributions: Vec<(u8, i64)> =
        hand.seats.iter().map(|s| (s.seat, s.contributed)).collect();
    let pots = side_pots_with_folded(&contributions, &folded);
    if pots.is_empty() {
        return None;
    }

    let street = all_in_street(hand, &contending)?;
    let board = board_at_street(hand, street);

    let gross: i64 = pots.iter().map(|p| p.amount).sum();
    let taken = hand.rake.minor.saturating_add(hand.jackpot_drop.minor).max(0);
    let awardable = (gross - taken).max(0);

    let mut expected = 0.0f64;
    for pot in &pots {
        // Only pots the hero can win matter; the rest is dead money for the hero.
        if !pot.eligible.contains(&hero_seat) {
            continue;
        }
        let mut hole_cards: Vec<Vec<Card>> = Vec::with_capacity(pot.eligible.len());
        for seat in &pot.eligible {
            let seat_row = hand.seat(*seat)?;
            if seat_row.shown_cards.len() < 2 {
                // Cards unknown: no adjustment is possible, exactly as PT4 reports.
                return None;
            }
            hole_cards.push(seat_row.shown_cards.iter().copied().take(2).collect());
        }
        let equity = equities(&hole_cards, &board);
        let hero_index = pot.eligible.iter().position(|s| *s == hero_seat)?;
        let hero_equity = equity.get(hero_index).copied().unwrap_or(0.0);
        let pot_share = pot.amount as f64 / gross as f64;
        expected += hero_equity * awardable as f64 * pot_share;
    }

    let adjusted = expected - hero.contributed as f64;
    Some(round_half_away_from_zero(adjusted))
}

/// Round to the nearest minor unit, half away from zero.
fn round_half_away_from_zero(value: f64) -> i64 {
    if value >= 0.0 {
        (value + 0.5).floor() as i64
    } else {
        (value - 0.5).ceil() as i64
    }
}
