# ADR-0004: Hero-centric parsed schema with raw hand text retained

## Status

Accepted

## Context

Every statistic in `docs/stat-definitions.md` is about the hero. Full per-seat,
per-action detail would make a large database and slow queries, but only a
fraction of it — the hero's actions, the aggressor context, seat contributions,
and hand outcomes — is needed to compute the defined statistics. The spec still
requires an escape hatch: an opponent database and HUD may come later, and
PokerStars purges old hand histories on disk, so anything not stored now may be
unrecoverable later.

## Decision

Store three layers per hand:

1. **Raw text** — the hand history verbatim, one row per hand, with parser
   version, import timestamp, and source file reference. Raw text is a
   compatibility guarantee, not a caching convenience; any future change that
   drops it, shown cards, or stable hand identities would make a future opponent
   database require an impossible re-import.
2. **Parsed schema** — the parser's only job is to turn raw text into a
   `ParsedHand`: header (hand id, site, game, limit, stakes, table name, table
   size, currency, UTC + local timestamps, button seat, board, pot, rake, jackpot
   drop, showdown flag), full seat rows (stack, position, contribution, result,
   shown cards, sat-out), and the complete action list. The parser is isolated
   behind this boundary so another site could be added later without touching
   reporting (spec story 79 / out-of-scope note).
3. **Hero analysis rows** — one row per hand per active hero profile
   (`HeroHandAnalysis`): the derived counters and flags every statistic needs
   (preflop action outcomes, per-street postflop outcomes including aggression
   context, position, table size, money fields, all-in adjusted result, luck
   delta, attributed rake). This is the performance-critical table, indexed by
   UTC time, stakes, table size, position, and hero alias, and reports aggregate
   over it plus the hand header. Per-seat opponent action detail is *not*
   persisted in the parsed layer; the raw text makes it recoverable.

Identity is a separate small layer: `players` (screen names), an alias mapping,
and the hero-profile table; the active profile determines which seat rows are
tagged as hero. Attribute everything at import time that would otherwise force
reports to re-derive (rake attribution, EV, derived flags), so reports read
stored values and can never disagree with each other.

## Consequences

- Reports aggregate over a narrow, indexed row: the filter set applies uniformly
  and the 10M-hand latency target is credible.
- Re-parsing (`reparse`, ADR-0001) is always possible and always honest: it
  rebuilds layers 2–3 from layer 1 and stamps new parser versions.
- The parser/schema boundary means parser fixes are zero-risk to reporting, and
  parser changes are replayable via the round of quarantine retries (spec story
  20).
- Disk cost of raw text is real but bounded for a single serious player's archive;
  it is the price of the future-proofing guarantee the spec makes load-bearing.
- Adding per-seat derived statistics later means re-parsing raw text, not
  re-importing — the property that distinguishes this design from losing data.
- The `HeroHandAnalysis` contract is frozen between workstreams; changes to it
  (e.g. the flagged `check_raise_opportunity` gap, A13 in
  `docs/stat-definitions.md`) are deliberate contract changes.
