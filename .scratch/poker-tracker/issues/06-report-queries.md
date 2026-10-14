# 06 — Report queries

Status: done
Type: task
Blocked by: 02, 05

## Goal

Implement `src/storage/queries.rs`: one filter-driven query builder used by every
report.

## Interface (frozen)

`aggregate`, `cumulative_series`, `hand_list`, `hand_count`, `stake_options`,
`AggregateRow`, `AggregateGroup`, `SeriesPoint`, `HandListRow`.

## Rules

- One `FilterSpec` → one SQL predicate. `FilterSpec::is_empty()` must match the
  unfiltered lifetime total.
- Aggregate over `hero_hands` (already denormalized: `ts_utc`, stakes,
  `table_size`, `position`, `profile_id`), so no join to `hands` is needed for
  filters.
- Grouping: `GroupBy::None`, `Day`, `Week`, `Month`, `Stake`, `Position`.
  Day/week/month use UTC.
- `cumulative_series` downsamples server-side (largest-triangle-three-buckets or
  equivalent) to at most `max_points` points, always including the first and last
  cumulative values, so a graph over millions of hands is a bounded payload.
- `stake_options` returns distinct `(sb_minor, bb_minor, label)`.

## Tests

In `tests/reporting.rs`: fixture hands with known results, feed a `FilterSpec`,
assert each statistic on a hand-authored population. At least one test per filter
dimension, one combined-filter test, and one test that an empty filter set matches
the unfiltered lifetime total.
