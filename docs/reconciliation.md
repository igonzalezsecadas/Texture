# Reconciliation against PokerTracker 4

The tracker's numbers are verified, not assumed: the same hand histories are
imported into both this tracker and PokerTracker 4, and headline aggregates
are compared for the same filters. A divergence outside the recorded
tolerance is **a bug report, not a curiosity**.

## Tolerance (proposed — needs operator sign-off)

The following tolerance is the **proposed default**, recorded here so
reconciliation is actionable. It is flagged as a decision: the operator
(owner of the PT4 install) must accept or amend it before the first
reconciliation, and the accepted values replace these.

| Metric | Tolerance (proposed) | Rationale |
|---|---|---|
| Hands | exact (0) | Both tools count only hands the hero was dealt into; any difference is a real defect. |
| Net result | €0.00 — match to the cent | Money is stored as integer minor units; there is no legitimate source of rounding in either tool. |
| BB/100 | ±0.01 | Both tools derive it from the same hands and blinds; a visible difference means different rounding or populations. |
| €/100 | ±0.01 | Same reasoning as BB/100. |
| All-in adjusted (EV) result | ±0.01 BB per all-in hand, evaluated cumulatively (i.e. ≤ 0.01 × n_allin BB total) | Equity is computed by two different implementations with different rounding conventions; per-hand differences average out, per-sample differences grow with the all-in count. |
| Rake paid | €0.00 — match to the cent | Both tools read the same summary rake; total rake must agree exactly. Only *attribution across players* may diverge (see known divergences), which does not affect the hero's attributed total unless the hero's share is computed differently — see the divergence list. |

Anything outside tolerance is treated as a defect: file an issue with the
date range, filters, both tools' numbers, and the delta.

## Procedure

1. **Choose the comparison window.** Pick a date range (in UTC), stakes, and
   table size, and apply the *same* filter set in both tools. Start with a
   window of 200–1000 hands: large enough that averages are meaningful, small
   enough that a discrepancy is traceable to individual hands.
2. **Import into both tools.**
   - Tracker: `texture import <hands-dir>` (or the running `serve` watcher).
     Confirm the import summary: hands, duplicates, quarantined. The
     quarantine count must be understood — an unexplained quarantine
     invalidates the comparison for that window.
   - PT4: import the same folder (same files, same window). PT4's PokerStars
     import covers the same formats.
3. **Set matching filters.**
   - Tracker: filter bar (date range, stakes, table size, hero alias).
   - PT4: report filters — date range, stakes, table size; ensure both count
     only hero hands.
4. **Export/read the PT4 aggregates.** From PT4's summary report, read hands,
   net result, BB/100, €/100, all-in adjusted result (PT4: "EV Won" or
   "All-in EV"), and rake paid.
5. **Read the tracker's aggregates** from the dashboard for the same filters.
6. **Compare and record.** Fill in the table below (keep one row per
   reconciliation run):

   | Window | Filters | Metric | Tracker | PT4 | Δ | Within tolerance |
   |---|---|---|---|---|---|---|
   | e.g. 2025-09-01..30 | €0.25/€0.50, 6-max | hands | | | | |
   | | | net result | | | | |
   | | | BB/100 | | | | |
   | | | €/100 | | | | |
   | | | all-in adj. | | | | |
   | | | rake paid | | | | |

7. **If everything is within tolerance,** record the run (date, window,
   parser version) and treat the tracker as verified for that window.
8. **If anything is outside tolerance,** narrow it down: bisect the window by
   date, then by stakes/table size, then find the offending hand(s) via the
   hand browser (parsed view + raw text side by side). The offending hand is
   a fixture: add it to the parser's test corpus and fix the bug.

## What is compared

- **Hands** — the count of hands the hero was dealt into (postflop or
  preflop, folded or not).
- **Net result** — hero's actual money won/lost, in EUR, to the cent.
- **BB/100** — net result in big blinds per 100 hands, using the hand's big
  blind.
- **€/100** — net result in euros per 100 hands.
- **All-in adjusted (EV) result** — the adjusted result computed at the point
  of all-in with cards known, handling multiway pots and side pots; the
  tracker's definition matches PT4's all-in EV semantics at the point of the
  all-in.
- **Rake paid** — the hand's rake from the summary, attributed to players by
  weighted contribution (the tracker) — total rake must match PT4 exactly;
  attribution to the hero is compared with the tolerance above.

## Known divergences (accepted, not defects)

These are agreed causes of small, explainable divergence. They are accepted
and recorded — anything *outside* the tolerance above is still a defect.

1. **Session boundaries and session count.** The tracker derives sessions at
   query time from a configurable idle gap (default 30 minutes) computed in
   UTC. PT4 uses its own session logic. Session count, and any session-level
   aggregation that depends on boundaries, will legitimately differ. Headline
   aggregates over a filter window do not.
2. **Session duration approximation.** Session duration is derived from the
   first and last hand timestamps in the session and is labelled approximate
   in the UI. PT4 may compute duration differently. Duration is not part of
   the reconciliation.
3. **Rake attribution method.** The tracker attributes rake to players by
   weighted contribution (each player's share of the pot proportional to
   their contribution). PT4's attribution method differs; per-player rake may
   diverge even though total rake matches exactly. If the hero's attributed
   rake is outside tolerance, check whether the difference is method, not
   parse error.
4. **Equity rounding.** All-in adjusted results come from two independent
   equity implementations with different internal precision and rounding
   conventions. Per-hand adjusted results can differ by small fractions of a
   big blind; cumulative divergence grows with the number of all-in hands,
   which is why the all-in tolerance is expressed per all-in hand rather than
   absolute.

## Notes

- Rake composition should be checked against real files: if a table's summary
  shows a separate jackpot drop apart from rake, both the summary parser and
  the rake statistic must account for it explicitly rather than folding it in
  silently. This would otherwise show up as a rake-paid divergence.
- Run one reconciliation after every parser upgrade and after every bulk
  re-parse, before trusting reports.
- All comparisons use UTC date filters in both tools; UTC is what both store,
  and it avoids daylight-saving artifacts.
