# Glossary

Domain vocabulary of the Poker Tracker ("texture"). Field names referenced here
are the frozen contract in `src/domain.rs`. Every statistic's exact definition and
edge cases live in `docs/stat-definitions.md` — not here.

- **hand** — one deal of No-Limit Hold'em cash poker at one table, from the first
  blind post to the pot award. The atomic record: raw text in, parsed hand out
  (`ParsedHand`), one hero analysis row.
- **hand id** — the PokerStars hand identity string; the natural key of all hand
  data. Import is an upsert-by-hand-id operation, so re-reads and appended files
  never double-count.
- **seat** — one chair in a hand: seat number, screen name, starting stack,
  position, starting cards when shown, total contribution, and result
  (`ParsedSeat`).
- **hero** — the seat being reported on; the owner of all statistics in v1. The
  hero is a *seat* in a hand, not a global player.
- **hero profile** — the active reporting identity: a named profile containing one
  or more screen names (`HeroProfile`). Reports always answer "which seat rows are
  the hero" via the active profile.
- **alias** — a second screen name resolving to the same hero profile (e.g.
  after a PokerStars rename). Aliases merge lifetime results; filtering by alias
  can view one screen name in isolation.
- **screen name** — the poker-site identity string of one seat/player
  (`Player.screen_name`). The canonical spelling to use everywhere; not
  "nickname".
- **session** — a derived grouping of the hero's hands: hands sorted by UTC time
  and split when the gap between consecutive hands is at least the configured
  idle gap (default 30 minutes). Multi-tabling merges naturally; duration is
  approximate (`Session`).
- **stake** — a (small blind, big blind) blind pair in minor units. The unit by
  which results are separated and BB-normalized (`FilterSpec::stakes`).
- **table size** — the maximum seat count of the table (6 or 9 in v1), which
  drives position mapping and steal-seat definitions. Filterable
  (`FilterSpec::table_sizes`).
- **position** — the seat's role relative to the button for this hand
  (`Position`: SB, BB, UTG, UTG1, MP, HJ, CO, BTN — see the 6-max/9-max mappings
  in `docs/stat-definitions.md`). Filterable and groupable.
- **quarantine** — the holding area for hands that fail to parse: raw text,
  failure reason, parser version, first/last seen. Quarantined hands are counted
  and surfaced in the UI, never silently dropped; they can be retried after a
  parser fix.
- **parser version** — the version tag stamped on every parsed hand row at import
  time. It marks exactly which parse decisions produced the stored data and is
  what `reparse` reasons about.
- **weighted contribution** — a seat's share of a hand's rake:
  `seat.contributed / Σ seats.contributed`, where `contributed` excludes returned
  uncalled bets. Attribution is computed once at import and stored
  (`rake_attributed`), so all rake reports read the same number.
- **all-in adjusted result** — the hero's EV-corrected net result for a hand:
  in every side pot the hero was in and that was decided by all-in participants
  with known cards, the actual award is replaced by the expected award at the
  all-in moment (`adjusted_result`). Nothing street-by-street is computed. Defined
  precisely in `docs/stat-definitions.md`.
- **luck delta** — `net_result − adjusted_result` per hand; positive means the
  hero ran above the all-in EV line. Zero whenever the hand had no all-in
  adjustment.
- **kill/parse cursor** — the file-state record used by the importer to skip work:
  path, size, modification time, content hash, and the parse cursor (the position
  in the file up to which all hands are known complete and parsed). A hash match
  *kills* (skips) re-reading the file; the cursor lets a partially written final
  hand wait until the file grows.
- **reparse** — the explicit, resumable command that rebuilds all parsed and
  derived data for a hand set from the retained raw text, replacing rows whose
  stored parser version is older than the current parser.
- **snapshot** — a consistent copy of the database taken with SQLite `VACUUM INTO`
  on a schedule or on demand, retained on the host bind mount; the restore path
  for a bad migration or accidental deletion.
- **filter set** — the one filter specification (`FilterSpec`: date range, stakes,
  table sizes, positions, session ids, hero profiles) applied identically by every
  report, so no statistic is computed on a different population than its
  neighbors. An empty filter set matches the unfiltered lifetime total.

## Terms this project deliberately avoids

- **"Nickname" / "username"** — use **screen name** (site-exact string).
- **"Player" (for results/stats)** — v1 reports hero rows only; "player" is
  reserved for the identity table and for future opponent data.
- **"EV winnings" / "EV line" as a standalone stat name** — display wording is
  **all-in adjusted result**; "EV" alone is ambiguous about the adjustment point.
- **"Profit"** — use **net result** (rake is a cost inside it, not an addendum).
- **"Game"** (for a deal of cards) — use **hand**; "game" means the variant
  (No-Limit Hold'em).
- **"Dollars" / bare money displays** — amounts are integer **minor units** with
  an explicit ISO currency code; UI money is always formatted with the symbol.
- **HUD, opponent stats, player pool, notes** — explicitly out of scope for v1
  (schema preserves the raw material; do not reintroduce these into reports).
- **"VIP points", "rakeback", "cashback"** — Stars Rewards are invisible in hand
  histories; only **rake paid** exists in this domain.
