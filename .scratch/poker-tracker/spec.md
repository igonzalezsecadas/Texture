# Poker Tracker (project "texture") — v1 Spec

Status: ready-for-agent

## Problem Statement

I play No-Limit Hold'em cash games on PokerStars using a EUR account. I want a poker tracker that I own, run, and can inspect end to end. PokerTracker 4 does the job but is heavyweight: it requires an external database server, a desktop application, and a lot of operational ceremony for what is, for me, a single-user reporting tool.

I want a tracker that:

- runs as one Docker container on my own machine, with my PokerStars hand-history folder mounted read-only,
- imports hands automatically while I play, without blocking the UI,
- stores everything locally in SQLite, in the same container, with a backup story I understand,
- shows the complete set of statistics a serious cash player expects — results, BB/100, €/100, all-in EV, rake paid, preflop, postflop, positional, and session reporting — with definitions that match what PokerTracker 4 means by the same names,
- lets me slice every report by the filters I actually use: date, stakes, table size, position, session, and hero alias,
- exposes a minimalist browser UI, not a desktop app and not a single-page app,
- and lets me verify its numbers against PokerTracker 4 rather than trusting them.

## Solution

A single Rust binary, shipped in one container, with three operational modes: a long-running web process that also watches the hand-history folder and imports continuously, a bulk import command, and a re-parse command. All hand data lives in one SQLite database on a host bind mount, in WAL mode, with raw hand text retained so any parse decision can be revisited later.

The web UI is server-rendered with progressive enhancement (htmx) and no JavaScript build toolchain. It presents a results/EV dashboard, preflop, postflop, and positional reports, session reports, a filter bar that applies to every report, and a hand browser with raw text, parse-error visibility, and CSV export.

Correctness is anchored to PokerTracker 4: the same hands are importable into both tools, and a reconciliation procedure compares headline aggregates within a documented tolerance so a divergence is a bug report, not a mystery.

## User Stories

### Setup and operation

1. As a player, I want to run the whole tracker as a single Docker container, so that I do not have to operate a database server or a second process.
2. As an operator, I want to mount my PokerStars hand-history directory read-only, so that the application can never modify or delete my history files.
3. As an operator, I want the SQLite database to live on a host bind mount, so that rebuilding or upgrading the container never risks my data.
4. As an operator, I want a compose file with sensible defaults, so that starting the tracker is one command.
5. As an operator, I want scheduled database snapshots taken automatically, so that a bad migration or an accidental deletion does not destroy years of history.
6. As an operator, I want to trigger a snapshot manually, so that I can take a known-good copy before an upgrade.
7. As a player, I want the web UI published to localhost only, so that I do not need authentication, TLS, or a security review.
8. As an operator, I want to configure the hand-history path, including Wine and compatibility-layer paths, so that the tracker works on my machine without code changes.
9. As a player, I want first-run setup to detect my hero screen name from the hand-history folder name, so that I do not mistype the identity that all my stats depend on.
10. As a player, I want the container to expose a healthcheck and structured logs, so that I can tell whether it is alive and importing.

### Import

11. As a player, I want new hands imported automatically within seconds of being written, so that my stats are current while I play.
12. As a player, I want the importer to run in the background without blocking report queries, so that browsing the UI during a large import stays responsive.
13. As a player, I want a bulk import command for my existing archive, so that I can backfill history in one operation.
14. As a player, I want bulk import to report progress (files, hands, duplicates, failures), so that I can see it is working and estimate completion.
15. As a player, I want a partially written hand file to be skipped until the hand is complete, so that reading a file mid-write never produces a corrupt hand.
16. As a player, I want duplicate hands to be ignored by hand ID, so that re-reading or appending to files never double-counts.
17. As a player, I want unchanged files not to be re-parsed, so that a restart or a re-scan is cheap.
18. As a player, I want hands that cannot be parsed to be quarantined with a reason rather than silently dropped, so that my stats are never quietly wrong.
19. As a player, I want to see how many hands are quarantined and why, so that I can judge whether a report is trustworthy.
20. As a player, I want to re-process quarantined hands after a parser fix, so that previously failed hands are recovered.
21. As a player, I want every parsed hand tagged with the parser version that produced it, so that I know exactly what would change if I re-parsed.
22. As a player, I want re-parsing to be an explicit, resumable command with progress, so that I control when a long, disk-heavy operation runs.
23. As a player, I want only hands I was dealt into to be counted, so that my hand count matches what PokerTracker 4 reports.
24. As a player, I want import to be idempotent, so that running import twice in a row changes nothing.

### Identity and money

25. As a player, I want to register one or more hero screen names and alias them into a single profile, so that my lifetime results survive a PokerStars rename.
26. As a player, I want a configurable active hero profile, so that reports default to the right identity.
27. As a player, I want all money stored as integer minor units with an explicit currency code, so that no rounding error ever enters a total.
28. As a player, I want EUR amounts parsed from PokerStars symbols and formatted consistently in the UI, so that displayed values match the hand histories.

### Results and EV

29. As a player, I want a results summary showing hands played, total won, and win rate, so that I can see where I stand in one glance.
30. As a player, I want win rate expressed both as BB/100 and €/100, so that I can compare across stakes and across accounts.
31. As a player, I want a cumulative winnings graph, so that I can see the shape of my results over time.
32. As a player, I want an all-in adjusted (EV) line on that graph, so that I can separate decision quality from short-term luck.
33. As a player, I want an all-in EV figure defined at the point of all-in with cards known, so that the number means what PokerTracker 4 means by the same name.
34. As a player, I want all-in EV to handle multiway pots and side pots correctly, so that the adjusted result is not wrong in exactly the hands that matter most.
35. As a player, I want to see the difference between actual and adjusted winnings ("luck" delta), so that I can quantify variance over a session or a month.
36. As a player, I want rake paid reported per player using weighted contribution, so that rake reporting is fair and consistent across hands.
37. As a player, I want rake also expressed in big blinds, so that I can compare rake cost across stakes.
38. As a player, I want the number of hands and the win rate to remain consistent with the filters applied, so that I never compare two different populations.

### Preflop statistics

39. As a player, I want VPIP, so that I can see how often I voluntarily put money in preflop.
40. As a player, I want PFR, so that I can see how often I raise preflop.
41. As a player, I want the gap between VPIP and PFR (limping rate), so that I can find passive leaks.
42. As a player, I want raise-first-in / open-raise rate, so that I can see how often I open the pot.
43. As a player, I want 3-bet and fold-to-3-bet rates, so that I can assess my preflop aggression and defense.
44. As a player, I want 4-bet and fold-to-4-bet rates, so that I can assess play in raised pots.
45. As a player, I want cold-call rate, so that I can see how often I enter behind a raise without re-raising.
46. As a player, I want steal rate and fold-to-steal (blind defense) rates, so that I can evaluate late-position and blind play.
47. As a player, I want every preflop statistic available per position, so that I can find position-specific leaks.

### Postflop statistics

48. As a player, I want c-bet frequency by street, so that I can see how often I follow through as the preflop aggressor.
49. As a player, I want fold-to-c-bet frequency, so that I can see how I respond to aggression on each street.
50. As a player, I want check-raise frequency by street, so that I can see how often I raise against a bet after checking.
51. As a player, I want probe and delayed c-bet frequencies, so that I can evaluate turn and river play after a checked-through flop.
52. As a player, I want won-when-saw-flop (WWSF), so that I can see how often I take down a pot after seeing the flop.
53. As a player, I want went-to-showdown (WTSD) and won-at-showdown (W$SD), so that I can spot over- and under-showdown tendencies.
54. As a player, I want aggression factor and aggression frequency, overall and by street, so that I can quantify passive versus aggressive play.
55. As a player, I want showdown versus non-showdown winnings separated in the results view, so that I can see where profit comes from.

### Reports, filters, and grouping

56. As a player, I want a positional breakdown of every statistic, so that I can compare, for example, BB versus BTN.
57. As a player, I want to filter every report by date range, so that I can look at a day, a month, or a career.
58. As a player, I want to filter by stake, so that I can separate results at different blind levels.
59. As a player, I want to filter by table size (6-max versus full ring), so that I can separate game formats.
60. As a player, I want to filter by position, so that I can drill into one seat.
61. As a player, I want to filter by session, so that I can review a single play session.
62. As a player, I want to filter by hero alias, so that a renamed account can be viewed separately or merged.
63. As a player, I want to group reports by day, week, month, stake, and position, so that I can see trends rather than totals.
64. As a player, I want the active filter set to be visible and easy to clear, so that I never misread a filtered number as a lifetime number.
65. As a player, I want a documented definition for every statistic shown, so that the number is unambiguous.

### Sessions

66. As a player, I want a session list with start, end, duration, hands, and result, so that I can review my play session by session.
67. As a player, I want a cumulative graph within a session, so that I can see how a session evolved.
68. As a player, I want session boundaries defined by a configurable idle gap (default 30 minutes), so that the definition matches how I think about a session.
69. As a player, I want hands from simultaneous tables merged into one session, so that multi-tabling is reported as one activity.
70. As a player, I want session boundaries computed in UTC while original local timestamps are preserved, so that daylight saving and travel cannot corrupt session grouping.
71. As a player, I want session duration to be labelled as derived from hand timestamps, so that I understand it is an approximation.

### Hands and export

72. As a player, I want to browse hands matching the current filters, so that I can inspect the raw material behind a statistic.
73. As a player, I want to open a hand and see table, stakes, seats, stacks, positions, board, actions, pot, rake, and result, so that I can verify the parse.
74. As a player, I want to see the original hand text for any hand, so that I can compare the parse against the source of truth.
75. As a player, I want to see which hands failed to parse and why, so that I can report or repair them.
76. As a player, I want to export the filtered aggregate table as CSV, so that I can work with the numbers elsewhere.
77. As a player, I want to export the raw hand histories for a filtered set, so that I can share or archive specific hands.

### Future-proofing

78. As a player, I want shown cards and seat identities preserved in the stored hand data, so that an opponent database and HUD could be built later by re-parsing raw history rather than by losing information now.
79. As a player, I want the parser to be isolated behind a defined boundary, so that another poker site could be added later without touching the reporting layer.
80. As a player, I want my headline numbers to be reconcilable against PokerTracker 4 within a documented tolerance, so that I can trust the tracker on day one.

## Implementation Decisions

### Process and packaging

- One Rust binary with three subcommands: `serve` (web UI plus continuous folder watcher and importer), `import` (bulk import of a directory), `reparse` (rebuild parsed data from retained raw text).
- The importer runs as a bounded background task inside `serve`, not as a separate service. It uses a single writer connection; the web layer uses a read pool. Memory is bounded by streaming one hand at a time through a bounded channel and committing in batches.
- Docker packaging favors runtime behavior, not image size: a small multi-stage image is welcome but not a goal. The container runs as a non-root user, handles signals for clean shutdown, mounts the hand-history directory read-only, and mounts the database directory read-write from the host.
- Interface: localhost binding only. No authentication, sessions, CSRF layer, or TLS termination is built.
- Configuration comes from environment variables for deployment concerns (paths, bind address, snapshot schedule) and from a settings table for user-level concerns (active hero profile, session gap, currency confirmation).

### Storage

- SQLite, bundled, WAL mode, `synchronous=NORMAL`, a busy timeout, and a tuned page cache. One writer, many readers. Batch commits of a few thousand hands. Indexes are dropped and rebuilt around bulk import rather than maintained per hand.
- Database and snapshots live on the host bind mount. Snapshots are taken with `VACUUM INTO` on a schedule with retention, plus a manual trigger.
- Money is stored as integer minor units with an explicit ISO currency code on every money-bearing row. Floating point money is forbidden. EUR has two decimal places in this data.
- Time is stored as UTC plus the original local timestamp text and the timezone token as written in the hand history. Session gaps and date filters use UTC. This is deliberately the safe design for daylight saving and travel.
- Raw hand text is stored in the database, one row per hand, together with the parser version, import timestamp, and source file reference. Parsed data can always be rebuilt from it.

### Data model (shape, not final column list)

- Hand identity: the PokerStars hand ID is the natural key. Import is an upsert-by-hand-ID operation.
- Hand header: hand ID, site, game, limit, stakes (small blind, big blind), table name, table size, currency, UTC timestamp, local timestamp, button seat, board, pot, rake, and whether the hand went to showdown.
- Seat rows: one per player per hand, holding seat number, screen name, starting stack, position, whether the player is the hero, voluntary contribution to the pot (required for weighted-contributed rake), result, and shown cards when revealed.
- Action data: the parsed schema is hero-centric. It retains the hero's per-street action summary required by the chosen statistics (for example: raised first in, cold-called, three-bet, c-bet, checked, called, folded to c-bet, check-raised) and the fields needed to attribute those actions to the correct aggressor context. Per-seat, per-action opponent detail is not persisted; the raw text makes it recoverable.
- Hero analysis rows: one row per hand for the active hero profile, holding derived counters and flags (preflop and postflop action outcomes, position, table size, session inputs, all-in adjusted result, luck delta, rake attributed). Reports aggregate over this row plus the hand header. This row is the performance-critical table and is indexed by UTC time, stakes, table size, position, and hero alias.
- All-in EV: computed at import time for hands where the hero was all-in with known cards. Equities are computed per pot including side pots and eligibility, and the adjusted result is stored per hand. Nothing street-by-street is computed. A deterministic equity implementation with a documented rounding convention is required, because reconciliation against PokerTracker 4 depends on it.
- Rake: taken from the hand summary and attributed to players by weighted contribution, computed from seat contribution totals. Attribution is computed at import time and stored, so reports never disagree with each other.
- Identity: a players table with screen names, plus an alias mapping and a hero-profile table. The active hero profile determines which seat rows are tagged as hero.
- Import bookkeeping: a file-state table (path, size, modification time, content hash, parse cursor) and a quarantine table (raw text, failure reason, parser version, first and last seen).
- Quarantine is surfaced, not hidden. A hand that fails to parse appears in the UI with its reason and can be retried after a parser change.

### Import behavior

- The watcher reacts to file creation and modification events and re-scans on startup. File-state bookkeeping means unchanged files are skipped, and hand-ID upserts make re-reads harmless.
- Only hands terminated by a complete summary section are imported; an incomplete trailing hand is held until the file grows. This is what makes continuous import while playing safe.
- Failure policy: quarantine the hand, continue importing, and count it. Import never halts a whole file because of one bad hand, and never silently drops a hand.
- Bulk import and continuous import share one code path so their behavior cannot diverge.

### Sessions

- Sessions are derived at query time from ordered hero hands using the configurable idle gap, computed in UTC. Multi-tabling merges naturally because the gap is time-based, not table-based. Session duration is derived from the first and last hand timestamps and is labelled as approximate in the UI.
- If session queries are measured to be too slow at the top of the supported scale, a materialized session table may be added with an explicit rebuild command. It is not built preemptively.

### Reports and queries

- A single filter specification (date range, stakes, table size, position, session, hero alias) is translated into one query builder used by every report, so a stat cannot be computed on a different population than its neighbors.
- Report queries run against the hero analysis rows and hand headers. The target is interactive latency at up to ten million hands with the full filter set applied.
- Statistics use PokerTracker 4 / Hold'em Manager 3 standard semantics as the reference. Every statistic shown in the UI carries a definition entry; where a definition has edge cases (limped pots, big-blind checks, all-in without showdown, multiway c-bet attribution), the rule is written down before implementation and is treated as part of the interface.
- Aggression is reported as both factor and frequency, because the two diverge in small samples and players use both.
- Charts are downsampled server-side (largest-triangle-three-buckets or equivalent) to a bounded number of points, so a cumulative graph over millions of hands renders as a fixed, small payload. The client draws them with a lightweight charting library.

### Web interface

- Server-rendered HTML with templates and htmx for partial updates. No single-page application, no JavaScript build step, no client-side state machine.
- Screens: dashboard (results, EV, luck, graphs, filters), preflop, postflop, position, sessions, hands browser, hand detail, import status and quarantine, settings.
- The filter bar is present on every report screen and its active state is always visible.
- A hand detail view shows the parsed hand and the original raw text side by side.

### Reconciliation

- A reconciliation procedure compares the tracker against PokerTracker 4 for the same date range, stakes, and table size: hands, net result, BB/100, €/100, all-in adjusted result, and rake paid.
- A tolerance is agreed and recorded, together with a list of known and accepted divergence causes (session boundaries, duration heuristics, rounding). Anything outside the tolerance is treated as a defect, not a curiosity.

## Testing Decisions

Good tests here observe external behavior: a raw hand-history file goes in, and either a defined parsed record comes out, or an aggregated number for a defined filter set comes out. Tests should not assert on internal parser structure, connection management, or query construction. The highest-value seam is the parser boundary (raw text to parsed hand) followed by the reporting boundary (filter specification plus stored hands to aggregate results). Fewer seams is better: the storage layer is exercised through the import path rather than mocked, using an in-memory or temporary database.

Modules to test:

- **Parser.** Golden-file tests over a fixture corpus of real PokerStars hands: one file per edge case, anonymized screen names, expected parsed hand recorded alongside. Edge cases that must each have a fixture: hand ending without showdown, everyone folds to the big blind preflop, limped and multiway pots, three-bet and four-bet pots, two-way all-in preflop, three-way all-in with a side pot, a short all-in for less than the bet, uncalled bet returned, a player disconnecting or timing out, a split pot, mucked cards at showdown, posted dead blinds, and a hand written in each currency form the client can emit. Unit claims (button/position assignment, blind posting, pot and side-pot construction, uncalled-bet subtraction) are asserted on parsed output, not on intermediate functions.
- **Money and pot arithmetic.** Property tests: contributions must sum to the pot minus uncalled bets; rake must never exceed the pot; attributed rake across players must sum to the hand's rake; amounts round-trip through parse and format without loss.
- **All-in EV.** Hand-computed expectations for a small set of all-in scenarios (two-way, multiway with side pots, split equity, all-in with unknown cards where no adjustment is possible). Where the scenario is simple enough, assert exact equities; otherwise assert documented ranges. The acceptance anchor is the aggregate comparison against PokerTracker 4.
- **Import pipeline.** Behavior tests for: importing the same directory twice changes nothing; appending hands to an existing file imports only the new hands; a file truncated mid-hand imports nothing until the hand is completed; a malformed hand is quarantined with a reason and does not stop the file; a quarantine retry after a parser fix imports the hand; a missing or unreadable file is reported, not fatal.
- **Reporting.** Fixture hands with known results, feed a filter specification, assert each statistic on a small hand-authored population where the correct answer can be computed by hand. Include at least one test per filter dimension and one test that combines filters, plus a test that an empty filter set matches the unfiltered lifetime total.
- **Sessions.** Gap exactly at the threshold, gaps above and below, hands ordered non-monotonically due to timezone changes, and a multi-table sequence interleaved by timestamp.
- **Performance.** A synthetic corpus scaled to the supported upper bound. Assert import throughput stays in the agreed background-import envelope, report queries stay within a latency budget at ten million hands with filters applied, chart endpoints return a bounded number of points regardless of the number of hands, and importer memory stays bounded during a full re-parse.
- **Reconciliation (manual or optional CI).** Compare aggregates against an exported PokerTracker 4 summary for the same filters and record the delta. This test is the reason the tolerance and known-divergence list exist.

Prior art: the fixture corpus starts from the player's own history. PokerTracker 4 can export hand histories, which is the fastest source of both the import backfill and anonymized golden files. Every odd hand encountered later is added as a fixture.

## Out of Scope

- A live HUD or any overlay on live tables, and any opponent statistics, notes, or player search in the UI. The schema preserves the data needed to add them later, but v1 shows hero statistics only.
- Tournaments, sit-and-gos, Spin & Go, bounties, and ICM-adjusted EV.
- Zoom and other fast-fold formats.
- Omaha and any non-hold'em games.
- Run-it-twice and all-in cash-out hands.
- Sites other than PokerStars. The parser is isolated so this could change, but no other site is implemented or tested.
- Multi-user support, authentication, remote or LAN access, TLS, and hosting other people's hand histories.
- Play money, multiple currencies, and foreign-exchange conversion. The schema is currency-aware; the application assumes one EUR account.
- Rakeback reconstruction. Stars Rewards and chest-based rewards do not appear in hand histories, so only rake paid can be reported.
- A street-by-street hand replayer and animated playback. The hand detail view and raw text stand in for v1.
- A single-page application, a mobile app, and any JavaScript build pipeline.

## Further Notes

- Hand-history availability is a real risk. the PokerStars client purges old hand histories, so the on-disk folder may hold less history than expected. PokerTracker 4's archive and its export function are the recovery path for the back catalog and the starting point for the fixture corpus. This should be confirmed early, because it affects both bulk import volume and test coverage.
- The accepted performance axes are import throughput, query latency at scale, chart rendering, and bounded memory. Image size and cold start are explicitly not goals, so build simplicity beats exotic packaging.
- The decision to keep reports hero-only while leaving the HUD door open is only coherent because raw hand text is retained. That constraint is load-bearing: any future change that drops raw text, shown cards, or stable hand identities would make a future opponent database require an impossible re-import. It should be treated as a compatibility guarantee, not an optimization opportunity.
- Expected divergence from PokerTracker 4: session boundaries and session count, since the gap heuristic is local to this tool; the session duration approximation; rake attribution method; and equity rounding. These are recorded as accepted causes, with the aggregate reconciliation setting the real bar.
- Rake composition should be verified against real files: if any of the player's tables include a separate jackpot drop shown apart from rake, the summary parser and the rake statistic must account for it rather than folding it in silently.
- The statistic dictionary (one entry per statistic, with its exact definition and edge-case rules) is a deliverable of its own and should be written before the reporting layer is implemented, not after.

### Definition of done for v1

1. The container starts with a read-only hand-history mount and a host database mount, and the UI is reachable on localhost.
2. A full backfill of the player's available history completes from the UI or CLI with visible progress, a quarantine count, and no silently dropped hands.
3. Continuous import picks up new hands within seconds while the UI stays responsive.
4. Dashboard, preflop, postflop, position, session, and hand-browser screens all honor the same filter set, including date, stakes, table size, position, session, and hero alias.
5. Every statistic shown has a written definition using PT4/HM3 semantics.
6. The cumulative graph renders a bounded payload at the supported upper bound with the all-in adjusted line.
7. Headline aggregates reconcile with PokerTracker 4 within the recorded tolerance for the same filters, and any divergence outside it is explained on the known-divergence list.
8. A snapshot can be taken and restored, and this is documented.
