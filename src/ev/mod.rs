//! All-in EV. Computed once at import time for hands where the hero was all-in
//! with known cards, including side pots and multiway eligibility.
//!
//! The result is stored per hand as `adjusted_result_minor`; nothing
//! street-by-street is computed. Reconciliation against PokerTracker 4 depends
//! on a deterministic, documented rounding convention.

use crate::domain::{Card, ParsedHand};

/// One side pot: the amount in it and the seats eligible to win it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidePot {
    pub amount: i64,
    pub eligible: Vec<u8>,
}

/// Split total contributions (minor units, per seat) into main and side pots.
pub fn side_pots(contributions: &[(u8, i64)]) -> Vec<SidePot> {
    let _ = contributions;
    unimplemented!("ev::side_pots")
}

/// Exact-ish equity per seat given known hole cards and a board, by exhaustive
/// enumeration when the remaining card space is small enough, otherwise by a
/// deterministic Monte Carlo sample with a fixed seed.
///
/// Returns equity in `[0.0, 1.0]` per entry of `hands`.
pub fn equities(hands: &[Vec<Card>], board: &[Card]) -> Vec<f64> {
    let _ = (hands, board);
    unimplemented!("ev::equities")
}

/// The hero's all-in adjusted net result for one hand, in minor units, or `None`
/// when no adjustment is possible (hero folded, cards unknown, no showdown).
pub fn all_in_adjusted_result(hand: &ParsedHand, hero_seat: u8) -> Option<i64> {
    let _ = (hand, hero_seat);
    unimplemented!("ev::all_in_adjusted_result")
}
