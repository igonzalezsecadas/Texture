# ADR-0003: Money as integer minor units; time as UTC plus original local text

## Status

Accepted

## Context

Two data classes get silently corrupted by convenience representations. Money put
in floating point accumulates rounding error across millions of hands and loses
the exact reconciliation against PokerTracker 4 the correctness path depends on.
Timestamps put in a single naive value break the moment the player crosses a DST
boundary or a time zone while playing: session gaps (default 30 minutes) and date
filters then group hands from different "days" into one, or split them wrongly,
and the corruption is unrecoverable from a single stored value.

The hand histories give us exactly two things: a local wall-clock timestamp with a
named time zone token (e.g. `CET`), and nothing else — UTC is derivable but is the
only stable basis for comparison.

## Decision

- **Money.** Every money-bearing row stores an integer count of *minor units*
  (EUR: cents, exponent 2) plus an explicit ISO currency code column. Routines in
  `src/money.rs` own parse, format, and arithmetic. Floating-point money is
  forbidden anywhere in the crate. v1 assumes one EUR account
  (`Currency::Eur`), but every row still carries the currency so the schema never
  needs a migration to hold a second one.
- **Time.** Every hand stores `timestamp_utc` (the comparison value), the
  original local timestamp text exactly as written in the hand history, and the
  timezone token as written (e.g. `CET`, `WEST`). All session-gap computation,
  date filters, chart x-axes, and report grouping use UTC. The original local
  text is for display only (it is what the hand detail view shows).
- Equity-derived money (all-in adjusted result) is computed in exact rational
  arithmetic from enumeration counts and rounded half-to-even to whole minor
  units once, at import, per side pot — the rounding convention is documented in
  `docs/stat-definitions.md` and is part of the reconciliation contract.

## Consequences

- Money totals are exact: contributions sum to pot minus uncalled bets, attributed
  rake sums to the hand rake, and parse/format round-trips are lossless — all
  property-test-friendly invariants (spec, Testing decisions).
- Session grouping and date filters are immune to daylight saving and travel,
  because the ordering and gap metric live in UTC while the human-facing time text
  is preserved verbatim (spec stories 68–71).
- Reports that display "the day" present a UTC day; a hand played at 23:30 CET
  shows its local text on the hand page but lands on the (possibly different) UTC
  date for grouping. This is documented behavior, not a bug.
- Any new money-bearing column must repeat the currency-code pattern; a reviewer
  can grep for the absence of `f64` money as a lint.
- If multi-currency support ever arrives (out of v1 scope), it is a reporting
  problem only — the storage layer is already currency-aware.
