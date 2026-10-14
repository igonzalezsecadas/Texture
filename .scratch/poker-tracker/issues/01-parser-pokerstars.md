# 01 — PokerStars hand-history parser

Status: done
Type: task
Blocked by: none

## Goal

Implement `texture::parser::pokerstars::parse_hand` so it turns one complete
PokerStars NLHE cash-game hand history into a `domain::ParsedHand`.

## Interface (frozen, do not change)

`src/domain.rs` defines `ParsedHand`, `ParsedSeat`, `Action`, `ActionKind`,
`Card`, `Position`, `Street`, `Money`, `Currency`. `src/error.rs::ParseError` is
the error type. `src/parser/pokerstars.rs::split_complete_hands` is already
implemented — do not rewrite it except to fix bugs.

## Scope

- Parse the header line: site, hand id, game (Hold'em No Limit), stakes symbol
  forms (`€0.25/€0.50 EUR`, `$1/$2 USD`), timestamp, timezone token.
- Parse table line: table name, table size (`6-max`, `9-max`), button seat.
- Parse seats: seat number, screen name, starting stack, sit-out markers.
- Parse blinds/antes/dead posts, including posted dead blinds.
- Parse street actions with correct `amount` and `to_amount` semantics, and
  `is_first_aggression`.
- Parse board cards, showdown reveals, mucked cards, collected pots.
- Parse the summary: total pot, rake, and any separately reported jackpot drop.
- Compute per-seat `contributed` (excluding returned uncalled bets) and
  `result` (net), and per-hand `pot_total`, `rake`, `jackpot_drop`,
  `went_to_showdown`, `streets_dealt`.
- Assign `Position` from button seat, table size and seat order.
- Convert the written local timestamp to `timestamp_utc` using the timezone
  token, preserving `local_timestamp` and `tz_token` verbatim.

## Required edge-case fixtures

Create one `.txt` + expected `.json` pair per case under `fixtures/parser/`:
no showdown; everyone folds to the BB; limped multiway pot; 3-bet pot; 4-bet
pot; two-way all-in preflop; three-way all-in with a side pot; short all-in for
less than the bet; uncalled bet returned; disconnect/timeout; split pot; mucked
cards; posted dead blinds; each currency form; multi-street hand reaching
river; check-raise.

Anonymize names. The `.json` is `serde_json` of `ParsedHand` (add
`#[derive(Serialize)]`, already present).

## Tests

`tests/parser_golden.rs`: for each fixture, parse the `.txt` and compare against
the `.json`. Compare structurally (serde_json::Value), with a documented
tolerance of none — exact.

Also assert on parsed output, never internals:
- contributions sum to pot plus returned uncalled bets
- rake never exceeds pot
- button/position assignment for 6-max and 9-max
- `streets_dealt` correctness

## Definition of done

`cargo test --test parser_golden` passes; `cargo build` clean; no `unimplemented!()`
left in `src/parser/`.
