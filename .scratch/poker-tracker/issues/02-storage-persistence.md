# 02 — Storage persistence

Status: in-progress (orchestrator)
Type: task
Blocked by: none

## Goal

Implement `src/storage/hands.rs`: hand/seat/raw persistence, file state,
quarantine, and import-run bookkeeping. All writes keyed on the PokerStars hand
id so import is idempotent.

## Interface (frozen)

See `src/storage/hands.rs`. `upsert_hand` must return `Duplicate` and change
nothing when the hand id exists. `raw_hands` is always written. `hand_seats`
rows are always written. `hero_hands` is written only when `analysis` is
`Some`.

## Tests

`tests/import_pipeline.rs` (shared with ticket 03): importing the same directory
twice changes nothing; hand-id upsert is idempotent.
