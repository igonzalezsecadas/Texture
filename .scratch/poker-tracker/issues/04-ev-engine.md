# 04 — All-in EV engine

Status: done
Type: task
Blocked by: none

## Goal

Implement `src/ev/mod.rs` so a hand in which the hero was all-in with known cards
gets a deterministic all-in adjusted result, including multiway pots and side
pots.

## Interface (frozen)

- `pub fn side_pots(contributions: &[(u8, i64)]) -> Vec<SidePot>`
- `pub fn equities(hands: &[Vec<Card>], board: &[Card]) -> Vec<f64>`
- `pub fn all_in_adjusted_result(hand: &ParsedHand, hero_seat: u8) -> Option<i64>`

## Rules

- Equity is computed at the point of all-in with cards known. No street-by-street
  equity.
- Multiway and side pots: each pot has a `amount` and a set of `eligible` seats;
  a seat is eligible when it contributed at least that pot's cap.
- `equities` enumerates exhaustively when the number of unknown cards is small
  enough (≤ ~1.7M combinations), otherwise a deterministic Monte Carlo sample
  with a fixed seed. Document the threshold in a comment.
- Split pots divide the pot evenly with the remainder assigned deterministically
  (lowest seat first) and documented.
- Rounding: adjusted result is rounded to the nearest minor unit, half away from
  zero. Document it.
- `all_in_adjusted_result` returns `None` when: hero folded, hero was not
  all-in, opponents' cards are unknown at the all-in, or no showdown occurred.

## Tests (`tests/ev.rs`)

Hand-computed expectations, at minimum:
- two-way all-in preflop, hero ahead (assert exact equity for a known match-up
  via exhaustive enumeration, e.g. AA vs KK)
- two-way all-in with a made hand vs draw where equity is known
- three-way all-in with a side pot: assert each side pot's amount and eligible
  set, and the aggregate adjusted result
- split equity: adjusted result equals half the pot
- all-in with unknown cards → `None`, no adjustment
- `side_pots` property: sum of pot amounts equals total contributions minus
  returned uncalled bets; every seat appears in every pot it is eligible for

## Definition of done

`cargo test --test ev` passes; no `unimplemented!()` left in `src/ev/`.
