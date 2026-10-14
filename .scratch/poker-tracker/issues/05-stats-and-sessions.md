# 05 — Statistics derivation and sessions

Status: done
Type: task
Blocked by: 01 (types only), 04

## Goal

Implement `src/stats/mod.rs`: derive the hero-centric `HeroHandAnalysis` row for
one hand, and derive sessions at query time.

## Interface (frozen)

- `pub fn analyze_hand(hand: &ParsedHand, profile_id: i64, player_id: i64) -> Option<HeroHandAnalysis>`
- `pub struct SessionHand { key, t, net_minor }`
- `pub fn derive_sessions(hands: &[SessionHand], gap_secs: i64) -> Vec<(i64, usize, usize)>`
- `pub fn sessions_for(db: &Db, filter: &FilterSpec, gap_secs: i64) -> Vec<Session>`

`analyze_hand` returns `None` when the hero's player_id is not among the seats
(only dealt-in hands count).

## Definitions

Implement exactly the definitions in `docs/stat-definitions.md` (ticket 08).
Read that file first; where it and this ticket disagree, the definitions file
wins and you should flag the conflict.

Key rules:
- VPIP/PFR/RFI are opportunity-gated: set both the `*` and `*_opportunity`
  flags, and never count a big-blind free check as a limp.
- C-bet/probe/delayed-c-bet/check-raise are per street in `[flop, turn, river]`.
- Aggression arrays are in `[preflop, flop, turn, river]` order.
- `wwsf_opportunity` is true when the hero saw the flop and the hand was not
  over before the flop completed; `won_when_saw_flop` is true when the hero won
  the pot after seeing the flop.
- `rake_attributed` and `jackpot_attributed` use weighted contribution over all
  seats; attributed amounts must sum to the hand's rake and jackpot drop.
- `adjusted_result` and `luck_delta` come from `ev::all_in_adjusted_result`;
  when there is no adjustment, `adjusted_result == net_result` and `luck_delta == 0`.
- `sd_winnings` / `non_sd_winnings` split the hero's result by whether the hand
  reached showdown.

## Sessions

- Ordered by UTC timestamp; a new session starts when the gap to the previous
  hand exceeds `gap_secs`. The boundary rule is `>` (a gap exactly at the
  threshold is the same session).
- Multi-tabling merges naturally because the gap is time-based.
- Session id is the index in the ordered sequence.

## Tests (`tests/sessions.rs`, plus cases in `tests/reporting.rs`)

- gap exactly at the threshold, just below, just above
- hands ordered non-monotonically due to timezone changes
- a multi-table interleaved sequence
- a hand-authored single-hand analysis where each preflop/postflop flag can be
  checked by hand
