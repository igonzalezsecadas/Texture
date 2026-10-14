//! Statistic derivation. Turns a [`ParsedHand`] plus a resolved hero seat into
//! the hero-centric counters stored in `hero_hands`, and derives sessions at
//! query time.
//!
//! Definitions live in `docs/stat-definitions.md`; this module implements them.

use crate::domain::{FilterSpec, HeroHandAnalysis, ParsedHand, Session};
use chrono::{DateTime, Utc};

/// Derive the analysis row for the hero in one hand. Returns `None` when the
/// hero was not dealt into the hand (the spec counts only dealt-in hands).
pub fn analyze_hand(
    hand: &ParsedHand,
    profile_id: i64,
    player_id: i64,
) -> Option<HeroHandAnalysis> {
    let _ = (hand, profile_id, player_id);
    unimplemented!("stats::analyze_hand")
}

/// A session input row: `(hand_id, timestamp_utc, net_result_minor)`.
#[derive(Debug, Clone, Copy)]
pub struct SessionHand {
    pub key: i64,
    pub t: DateTime<Utc>,
    pub net_minor: i64,
}

/// Group ordered hero hands into sessions using an idle gap. The gap is applied
/// in UTC so daylight saving and travel cannot corrupt grouping. Session ids are
/// the index of the session in the ordered sequence, stable for a given filter.
pub fn derive_sessions(hands: &[SessionHand], gap_secs: i64) -> Vec<(i64, usize, usize)> {
    let _ = (hands, gap_secs);
    unimplemented!("stats::derive_sessions")
}

/// Convenience: sessions matching a filter, materialized for the session screen.
pub fn sessions_for(db: &crate::storage::Db, filter: &FilterSpec, gap_secs: i64) -> Vec<Session> {
    let _ = (db, filter, gap_secs);
    unimplemented!("stats::sessions_for")
}
