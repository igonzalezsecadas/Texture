//! Statistic derivation. Turns a [`ParsedHand`] plus a resolved hero seat into
//! the hero-centric counters stored in `hero_hands`, and derives sessions at
//! query time.
//!
//! Every definition implemented here is written down in
//! `docs/stat-definitions.md`, which is authoritative. Where this code has to
//! choose between readings of an ambiguous rule, the reasoning is in a comment
//! next to the flag.

use crate::domain::{
    ActionKind, FilterSpec, HeroHandAnalysis, ParsedHand, Position, Session, Street,
};
use crate::ev;
use crate::storage::queries;
use crate::storage::Db;
use chrono::{DateTime, Utc};

/// The hero's identity within a single hand: which seat they occupied, and which
/// profile/player that seat belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeroSeat {
    pub seat: u8,
    pub profile_id: i64,
    pub player_id: i64,
}

/// Street position within a hand, used to reason about "before/after".
fn street_order(street: Street) -> u8 {
    street.index() as u8
}

fn is_player_action(kind: ActionKind) -> bool {
    matches!(
        kind,
        ActionKind::Fold
            | ActionKind::Check
            | ActionKind::Call
            | ActionKind::Bet
            | ActionKind::Raise
    )
}

fn is_voluntary_money(kind: ActionKind) -> bool {
    matches!(kind, ActionKind::Call | ActionKind::Bet | ActionKind::Raise)
}

fn is_aggressive(kind: ActionKind) -> bool {
    matches!(kind, ActionKind::Bet | ActionKind::Raise)
}

fn is_passive(kind: ActionKind) -> bool {
    matches!(kind, ActionKind::Call | ActionKind::Check)
}

/// Resolve the hero's seat from a `(screen_name, player_id)` mapping supplied by
/// the caller. The parser deliberately does not know identities, so this stays a
/// pure function and storage stays out of the derivation.
pub fn resolve_hero(
    hand: &ParsedHand,
    profile_id: i64,
    players: &[(String, i64)],
) -> Option<HeroSeat> {
    for (screen_name, player_id) in players {
        let Some(seat) = hand.seat_by_name(screen_name) else {
            continue;
        };
        if !was_dealt_in(hand, seat.seat) {
            continue;
        }
        return Some(HeroSeat { seat: seat.seat, profile_id, player_id: *player_id });
    }
    None
}

/// Whether a seat was actually dealt into the hand.
///
/// PokerStars lists every occupied seat, including players who are sitting out.
/// A seat was dealt in when it received hole cards (`Dealt to <hero>`, which the
/// parser records in `shown_cards`), or took any action, or posted a blind or
/// ante. Counting only dealt-in hands is what makes the hero's hand count match
/// PokerTracker 4.
fn was_dealt_in(hand: &ParsedHand, seat: u8) -> bool {
    let Some(row) = hand.seat(seat) else { return false };
    if !row.shown_cards.is_empty() {
        return true;
    }
    hand.actions.iter().any(|a| a.seat == seat)
}

/// Convenience for the import pipeline: resolve the hero seat, then derive.
pub fn analyze_hero(
    hand: &ParsedHand,
    profile_id: i64,
    players: &[(String, i64)],
) -> Option<HeroHandAnalysis> {
    let hero = resolve_hero(hand, profile_id, players)?;
    analyze_hand(hand, hero)
}

/// Derive the analysis row for a resolved hero seat.
///
/// Returns `None` when the seat is not in the hand, when the hero was not dealt
/// in, or when the position of another seat cannot be resolved.
pub fn analyze_hand(hand: &ParsedHand, hero: HeroSeat) -> Option<HeroHandAnalysis> {
    let hero_row = hand.seat(hero.seat)?;
    if !was_dealt_in(hand, hero.seat) {
        return None;
    }

    let mut h = HeroHandAnalysis {        hand_id: hand.hand_id.clone(),
        profile_id: hero.profile_id,
        player_id: hero.player_id,
        position: hero_row.position,
        table_size: hand.table_size,
        currency: hand.currency,
        small_blind: hand.small_blind.minor,
        big_blind: hand.big_blind.minor,
        timestamp_utc: hand.timestamp_utc,
        ..HeroHandAnalysis::blank()
    };

    // ---- money -----------------------------------------------------------
    h.net_result = hero_row.result.minor;
    h.contributed = hero_row.contributed;
    h.won_amount = hero_row.result.minor + hero_row.contributed;

    let (rake, jackpot) = attribute_rake(hand, hero.seat);
    h.rake_attributed = rake;
    h.jackpot_attributed = jackpot;

    if let Some(adjusted) = ev::all_in_adjusted_result(hand, hero.seat) {
        h.adjusted_result = adjusted;
    } else {
        // No adjustment is possible, so the actual result is the adjusted result.
        h.adjusted_result = h.net_result;
    }
    h.luck_delta = h.net_result - h.adjusted_result;

    // ---- preflop ---------------------------------------------------------
    let preflop = indices_of(hand, Street::Preflop);
    let hero_raises_before: Vec<usize> = hero_raise_positions(&preflop, hand, hero.seat)
        .into_iter()
        .map(|pos| raises_before(&preflop, hand, pos))
        .collect();
    let hero_raise_positions = hero_raise_positions(&preflop, hand, hero.seat);
    let first_voluntary = preflop.iter().position(|&i| {
        hand.actions[i].seat == hero.seat && is_voluntary_money(hand.actions[i].kind)
    });
    let first_entry = preflop.iter().position(|&i| is_voluntary_money(hand.actions[i].kind));

    if let Some(pos) = first_voluntary {
        let kind = hand.actions[preflop[pos]].kind;
        // How many raises were already in front of the hero's first voluntary
        // action. This is independent of whether the hero ever raised.
        let raises_before_first = raises_before(&preflop, hand, pos);
        let entered_before = first_entry.is_some_and(|e| e < pos);
        let opening = !entered_before;

        h.vpip = true;
        h.pfr = !hero_raise_positions.is_empty();
        // A limp is calling the big blind with no raise in front, from a seat
        // that is not the big blind: the big blind's free check is neither a VPIP
        // nor a limp.
        h.limp = kind == ActionKind::Call && raises_before_first == 0 && hero_row.position != Position::Bb;
        h.rfi_opportunity = opening && hero_row.position != Position::Bb;
        h.rfi = h.rfi_opportunity && kind == ActionKind::Raise;
        h.three_bet_opportunity = raises_before_first >= 1;
        h.three_bet = hero_raises_before.first().is_some_and(|r| *r == 1);
        h.cold_call_opportunity = raises_before_first >= 1;
        h.cold_call = h.cold_call_opportunity && kind == ActionKind::Call;
        h.steal_opportunity =
            opening && matches!(hero_row.position, Position::Co | Position::Btn | Position::Sb);
        h.steal_attempt = h.steal_opportunity && kind == ActionKind::Raise;
    }

    // A second raise by the hero is a 4-bet, and anything beyond rolls into the
    // 4-bet counters because the schema has no 5-bet bucket.
    h.four_bet = hero_raise_positions.len() >= 2
        || hero_raises_before.first().is_some_and(|r| *r >= 2);

    // Facing a 3-bet: the hero opened and an opponent re-raised.
    if hero_raises_before.first() == Some(&0) {
        if let Some(pos) = hero_raise_positions.first() {
            if let Some(reraise) = first_opponent_raise_after(&preflop, hand, hero.seat, *pos) {
                h.faced_three_bet = true;
                h.folded_to_three_bet = next_hero_action_after(&preflop, hand, hero.seat, reraise)
                    .is_some_and(|p| hand.actions[preflop[p]].kind == ActionKind::Fold);
            }
        }
    }
    h.four_bet_opportunity = h.faced_three_bet
        || hero_raises_before.first().is_some_and(|r| *r >= 2);

    // Facing a 4-bet: the hero 3-bet and an opponent re-raised, or the hero faced
    // a 3-bet and then re-raised before being re-raised again.
    let four_bet_anchor = if hero_raises_before.first() == Some(&1) {
        hero_raise_positions.first().copied()
    } else if hero_raises_before.first() == Some(&0) {
        hero_raise_positions.get(1).copied()
    } else {
        None
    };
    if let Some(anchor) = four_bet_anchor {
        if let Some(reraise) = first_opponent_raise_after(&preflop, hand, hero.seat, anchor) {
            h.faced_four_bet = true;
            h.folded_to_four_bet = next_hero_action_after(&preflop, hand, hero.seat, reraise)
                .is_some_and(|p| hand.actions[preflop[p]].kind == ActionKind::Fold);
        }
    }

    // Facing a steal: the hero is in the big blind and the pot was opened by a
    // late-position seat with nobody else in.
    if hero_row.position == Position::Bb {
        if let Some(entry) = first_entry {
            let opener = hand.actions[preflop[entry]];
            if opener.kind == ActionKind::Raise {
                if let Some(opener_position) = position_of(hand, opener.seat) {
                    if matches!(opener_position, Position::Co | Position::Btn | Position::Sb) {
                        h.faced_steal = true;
                        h.folded_to_steal = preflop.iter().any(|&i| {
                            let a = hand.actions[i];
                            a.seat == hero.seat && a.kind == ActionKind::Fold
                        });
                    }
                }
            }
        }
    }

    // ---- postflop --------------------------------------------------------
    let hero_fold_street = hand
        .actions
        .iter()
        .find(|a| a.seat == hero.seat && a.kind == ActionKind::Fold)
        .map(|a| a.street);

    let postflop = [Street::Flop, Street::Turn, Street::River];
    let mut aggressor = hand
        .actions
        .iter()
        .rev()
        .find(|a| a.street == Street::Preflop && a.kind == ActionKind::Raise)
        .map(|a| a.seat);
    // A limped pot has no aggressor, so there is no c-bet, probe or delayed c-bet
    // opportunity in it.

    let mut checked_through_previous = false;
    for (index, street) in postflop.into_iter().enumerate() {
        let dealt = hand.streets_dealt.contains(&street);
        let actions = indices_of(hand, street);
        let entering_aggressor = aggressor;

        // The hero is still in the hand on a street they did not fold before.
        let in_hand = dealt
            && hero_fold_street
                .is_none_or(|fold| street_order(fold) >= street_order(street));

        let last_aggressor = actions
            .iter()
            .rev()
            .find(|&&i| is_aggressive(hand.actions[i].kind))
            .map(|&i| hand.actions[i].seat);
        if let Some(seat) = last_aggressor {
            aggressor = Some(seat);
        }

        let first_bet = actions.iter().position(|&i| is_aggressive(hand.actions[i].kind));
        let hero_first = actions.iter().position(|&i| hand.actions[i].seat == hero.seat);
        let no_bet_before_hero = match (first_bet, hero_first) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(bet), Some(hf)) => hf <= bet,
        };

        if in_hand {
            // C-bet: betting a street immediately after being the aggressor. A bet
            // after a checked-through previous street is a delayed c-bet instead,
            // which is what PT4 means by the two names.
            h.cbet_opportunity[index] =
                entering_aggressor == Some(hero.seat) && !checked_through_previous && no_bet_before_hero;
            if h.cbet_opportunity[index] {
                h.cbet[index] = hero_first.is_some_and(|hf| hand.actions[actions[hf]].kind == ActionKind::Bet);
            }
            h.delayed_cbet_opportunity[index] = index >= 1
                && entering_aggressor == Some(hero.seat)
                && checked_through_previous;
            if h.delayed_cbet_opportunity[index] {
                h.delayed_cbet[index] =
                    hero_first.is_some_and(|hf| hand.actions[actions[hf]].kind == ActionKind::Bet);
            }

            // Facing a c-bet: the aggressor from the previous street bets and the
            // hero has to respond to it.
            let facing_aggressor_bet = match (first_bet, hero_first) {
                (Some(bet), Some(hf)) => {
                    hand.actions[actions[bet]].seat == entering_aggressor.unwrap_or(u8::MAX)
                        && entering_aggressor != Some(hero.seat)
                        && hf > bet
                }
                _ => false,
            };
            if facing_aggressor_bet {
                h.faced_cbet[index] = true;
                let response = actions.get(hero_first.unwrap_or(usize::MAX));
                h.folded_to_cbet[index] =
                    response.is_some_and(|&i| hand.actions[i].kind == ActionKind::Fold);
            }

            // Check-raise: the hero checks, a bet arrives, and the hero raises.
            let hero_checked = hero_first.is_some_and(|hf| hand.actions[actions[hf]].kind == ActionKind::Check);
            if hero_checked {
                if let Some(hf) = hero_first {
                    let bet_after = actions
                        .iter()
                        .enumerate()
                        .find(|(pos, &i)| *pos > hf && is_aggressive(hand.actions[i].kind))
                        .map(|(_, &i)| i);
                    if let Some(_bet_index) = bet_after {
                        let raised_after = actions
                            .iter()
                            .enumerate()
                            .any(|(pos, &i)| {
                                pos > hf
                                    && hand.actions[i].seat == hero.seat
                                    && is_aggressive(hand.actions[i].kind)
                            });
                        if raised_after {
                            h.check_raise_opportunity[index] = true;
                            h.check_raise[index] = true;
                        } else if actions
                            .iter()
                            .enumerate()
                            .any(|(pos, &i)| pos > hf && hand.actions[i].seat == hero.seat)
                        {
                            // The hero had a bet to raise and chose not to.
                            h.check_raise_opportunity[index] = true;
                        }
                    }
                }
            }

            // Probe: betting into the previous street's aggressor after they check.
            if let Some(entering) = entering_aggressor {
                if entering != hero.seat {
                    let aggressor_check = actions.iter().find(|&&i| hand.actions[i].seat == entering);
                    if aggressor_check.is_some_and(|&i| hand.actions[i].kind == ActionKind::Check) {
                        let aggressor_pos =
                            actions.iter().position(|&i| hand.actions[i].seat == entering).unwrap_or(0);
                        if hero_first.is_some_and(|hf| hf > aggressor_pos) {
                            h.probe_opportunity[index] = true;
                            h.probe[index] = hero_first
                                .is_some_and(|hf| hand.actions[actions[hf]].kind == ActionKind::Bet);
                        }
                    }
                }
            }
        }

        // Aggression by street. Preflop is index 0, then flop, turn, river.
        for &i in &actions {
            let a = hand.actions[i];
            if a.seat != hero.seat {
                continue;
            }
            if is_aggressive(a.kind) {
                h.aggressive_actions[index + 1] += 1;
            } else if is_passive(a.kind) {
                h.passive_actions[index + 1] += 1;
            }
        }

        checked_through_previous = dealt && first_bet.is_none();
    }

    // Preflop aggression counts.
    for &i in &preflop {
        let a = hand.actions[i];
        if a.seat != hero.seat {
            continue;
        }
        if is_aggressive(a.kind) {
            h.aggressive_actions[0] += 1;
        } else if is_passive(a.kind) {
            h.passive_actions[0] += 1;
        }
    }

    // ---- showdown and winnings ------------------------------------------
    h.saw_flop = hand.streets_dealt.contains(&Street::Flop)
        && hero_fold_street.is_none_or(|f| street_order(f) >= street_order(Street::Flop));
    h.saw_turn = hand.streets_dealt.contains(&Street::Turn)
        && hero_fold_street.is_none_or(|f| street_order(f) >= street_order(Street::Turn));
    h.saw_river = hand.streets_dealt.contains(&Street::River)
        && hero_fold_street.is_none_or(|f| street_order(f) >= street_order(Street::River));

    let hero_folded = hero_fold_street.is_some();
    h.went_to_showdown = hand.went_to_showdown && !hero_folded;
    // "Won a pot" is pot-based, not net-positive: a player can win a pot and still
    // be down for the hand after rake.
    h.won_at_showdown = h.went_to_showdown && h.won_amount > h.contributed;

    h.sd_winnings = if h.went_to_showdown { h.net_result } else { 0 };
    h.non_sd_winnings = if h.went_to_showdown { 0 } else { h.net_result };

    h.wwsf_opportunity = h.saw_flop;
    h.won_when_saw_flop = h.saw_flop && h.won_amount > h.contributed;

    
    Some(h)
}

impl HeroHandAnalysis {
    /// A zeroed analysis row, so every field can be filled in explicitly where it
    /// matters and left at its neutral value everywhere else.
    pub fn blank() -> HeroHandAnalysis {
        HeroHandAnalysis {
            hand_id: String::new(),
            profile_id: 0,
            player_id: 0,
            position: Position::Mp,
            table_size: 0,
            currency: crate::money::Currency::Eur,
            small_blind: 0,
            big_blind: 0,
            timestamp_utc: DateTime::<Utc>::from_timestamp(0, 0).expect("epoch is valid"),
            net_result: 0,
            adjusted_result: 0,
            luck_delta: 0,
            rake_attributed: 0,
            jackpot_attributed: 0,
            contributed: 0,
            won_amount: 0,
            sd_winnings: 0,
            non_sd_winnings: 0,
            went_to_showdown: false,
            won_at_showdown: false,
            saw_flop: false,
            saw_turn: false,
            saw_river: false,
            wwsf_opportunity: false,
            won_when_saw_flop: false,
            vpip: false,
            pfr: false,
            limp: false,
            rfi: false,
            rfi_opportunity: false,
            three_bet: false,
            three_bet_opportunity: false,
            faced_three_bet: false,
            folded_to_three_bet: false,
            four_bet: false,
            four_bet_opportunity: false,
            faced_four_bet: false,
            folded_to_four_bet: false,
            cold_call: false,
            cold_call_opportunity: false,
            steal_attempt: false,
            steal_opportunity: false,
            faced_steal: false,
            folded_to_steal: false,
            cbet_opportunity: [false; 3],
            cbet: [false; 3],
            faced_cbet: [false; 3],
            folded_to_cbet: [false; 3],
            check_raise_opportunity: [false; 3],
            check_raise: [false; 3],
            probe_opportunity: [false; 3],
            probe: [false; 3],
            delayed_cbet_opportunity: [false; 3],
            delayed_cbet: [false; 3],
            aggressive_actions: [0; 4],
            passive_actions: [0; 4],
        }
    }
}

// ---------------------------------------------------------------------------
// Preflop helpers
// ---------------------------------------------------------------------------

fn indices_of(hand: &ParsedHand, street: Street) -> Vec<usize> {
    hand.actions
        .iter()
        .enumerate()
        .filter(|(_, a)| a.street == street)
        .map(|(i, _)| i)
        .collect()
}

fn position_of(hand: &ParsedHand, seat: u8) -> Option<Position> {
    hand.seat(seat).map(|s| s.position)
}

/// Positions within `preflop` at which the hero raised.
fn hero_raise_positions(preflop: &[usize], hand: &ParsedHand, hero: u8) -> Vec<usize> {
    preflop
        .iter()
        .enumerate()
        .filter(|(_, &i)| hand.actions[i].seat == hero && hand.actions[i].kind == ActionKind::Raise)
        .map(|(pos, _)| pos)
        .collect()
}

/// How many raises happened before position `pos`.
fn raises_before(preflop: &[usize], hand: &ParsedHand, pos: usize) -> usize {
    preflop
        .iter()
        .take(pos)
        .filter(|&&i| hand.actions[i].kind == ActionKind::Raise)
        .count()
}

/// The position within `preflop` of the first opponent raise after `pos`.
fn first_opponent_raise_after(
    preflop: &[usize],
    hand: &ParsedHand,
    hero: u8,
    pos: usize,
) -> Option<usize> {
    preflop
        .iter()
        .enumerate()
        .find(|(p, &i)| *p > pos && hand.actions[i].seat != hero && hand.actions[i].kind == ActionKind::Raise)
        .map(|(p, _)| p)
}

/// The position within `preflop` of the hero's next action after `pos`.
fn next_hero_action_after(
    preflop: &[usize],
    hand: &ParsedHand,
    hero: u8,
    pos: usize,
) -> Option<usize> {
    preflop
        .iter()
        .enumerate()
        .find(|(p, &i)| *p > pos && hand.actions[i].seat == hero && is_player_action(hand.actions[i].kind))
        .map(|(p, _)| p)
}

// ---------------------------------------------------------------------------
// Rake attribution
// ---------------------------------------------------------------------------

/// Split `total` across seats in proportion to their contribution, using the
/// largest-remainder method so the parts always sum exactly to `total`.
///
/// The hero's share is returned, along with the jackpot share, and both are
/// computed by the same rule as every other seat, so reporting never disagrees
/// with itself.
fn attribute_rake(hand: &ParsedHand, hero: u8) -> (i64, i64) {
    let parts: Vec<(u8, i64)> = hand.seats.iter().map(|s| (s.seat, s.contributed)).collect();
    let rake = split_weighted(hand.rake.minor, &parts);
    let jackpot = split_weighted(hand.jackpot_drop.minor, &parts);
    let pick = |shares: &[(u8, i64)]| shares.iter().find(|(s, _)| *s == hero).map(|(_, v)| *v).unwrap_or(0);
    (pick(&rake), pick(&jackpot))
}

fn split_weighted(total: i64, parts: &[(u8, i64)]) -> Vec<(u8, i64)> {
    let contributed: i64 = parts.iter().map(|(_, c)| *c).sum();
    if total <= 0 || contributed <= 0 {
        return parts.iter().map(|(s, _)| (*s, 0)).collect();
    }
    let mut floors: Vec<(u8, i64, f64)> = parts
        .iter()
        .map(|(seat, contribution)| {
            let exact = total as f64 * (*contribution as f64) / contributed as f64;
            let floor = exact.floor();
            (*seat, floor as i64, exact - floor)
        })
        .collect();
    let assigned: i64 = floors.iter().map(|(_, v, _)| *v).sum();
    let mut remainder = total - assigned;
    // Largest remainder first; ties go to the lowest seat so the result is stable.
    floors.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    for entry in floors.iter_mut() {
        if remainder <= 0 {
            break;
        }
        entry.1 += 1;
        remainder -= 1;
    }
    floors.sort_by_key(|(seat, _, _)| *seat);
    floors.into_iter().map(|(seat, value, _)| (seat, value)).collect()
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// A session input row: `(hand_id, timestamp_utc, net_result_minor)`.
#[derive(Debug, Clone, Copy)]
pub struct SessionHand {
    pub key: i64,
    pub t: DateTime<Utc>,
    pub net_minor: i64,
}

/// Group ordered hero hands into sessions using an idle gap. The gap is applied
/// in UTC so daylight saving and travel cannot corrupt grouping. Session ids are
/// the index of the session in the ordered sequence, and the returned ranges are
/// `(session_id, start_index, end_index)` with `end_index` exclusive.
///
/// The boundary rule is strict: a gap exactly at the threshold stays in the same
/// session.
pub fn derive_sessions(hands: &[SessionHand], gap_secs: i64) -> Vec<(i64, usize, usize)> {
    if hands.is_empty() {
        return Vec::new();
    }
    let mut sessions = Vec::new();
    let mut start = 0usize;
    for i in 1..hands.len() {
        let gap = (hands[i].t - hands[i - 1].t).num_seconds();
        if gap.abs() > gap_secs {
            sessions.push((sessions.len() as i64, start, i));
            start = i;
        }
    }
    sessions.push((sessions.len() as i64, start, hands.len()));
    sessions
}

/// Sessions matching a filter, materialized for the session screen.
///
/// Hand-level rows come from the single report query builder, so a session is
/// computed on exactly the same population as every other report.
pub fn sessions_for(db: &Db, filter: &FilterSpec, gap_secs: i64) -> Vec<Session> {
    let mut rows = Vec::new();
    let page = 50_000i64;
    let mut offset = 0i64;
    loop {
        let batch = queries::hand_list(db, filter, page, offset).unwrap_or_default();
        if batch.is_empty() {
            break;
        }
        offset += batch.len() as i64;
        let done = (batch.len() as i64) < page;
        rows.extend(batch);
        if done {
            break;
        }
    }
    rows.sort_by_key(|r| r.ts_utc);

    let hands: Vec<SessionHand> = rows
        .iter()
        .map(|r| SessionHand {
            key: r.ts_utc,
            t: DateTime::<Utc>::from_timestamp(r.ts_utc, 0).unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).expect("epoch")),
            net_minor: r.net_result_minor,
        })
        .collect();

    derive_sessions(&hands, gap_secs)
        .into_iter()
        .map(|(id, start, end)| {
            let slice = &rows[start..end];
            Session {
                id,
                started_at: DateTime::<Utc>::from_timestamp(slice.first().map(|r| r.ts_utc).unwrap_or(0), 0)
                    .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).expect("epoch")),
                ended_at: DateTime::<Utc>::from_timestamp(slice.last().map(|r| r.ts_utc).unwrap_or(0), 0)
                    .unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).expect("epoch")),
                hands: slice.len() as i64,
                net_result: slice.iter().map(|r| r.net_result_minor).sum(),
                currency: crate::money::Currency::Eur,
            }
        })
        .collect()
}
