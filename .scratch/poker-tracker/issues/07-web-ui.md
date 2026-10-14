# 07 — Web UI

Status: ready-for-agent
Type: task
Blocked by: 06

## Goal

Implement `src/web/mod.rs`: server-rendered HTML with minijinja and htmx. No SPA,
no JS build step, no client-side state machine.

## Interface (frozen)

`AppState`, `ImporterHandle`, `router(state) -> axum::Router`, `serve(...)`.

## Screens

- `/` dashboard: results, EV, luck, cumulative graph (with all-in adjusted line), filter bar
- `/preflop`, `/postflop`, `/position`: statistic tables
- `/sessions`: session list (start, end, duration labelled approximate, hands, result)
- `/hands`: browser of hands matching current filters, with CSV export
- `/hands/:id`: hand detail, parsed hand and raw text side by side
- `/import`: import status, quarantine list with reasons and a retry action
- `/settings`: hero profile/alias management, session gap, snapshot trigger
- `/healthz`: healthcheck (JSON), `/static/*`: static assets

## Requirements

- The filter bar is present on every report screen and its active state is always
  visible, with a one-click clear.
- Filters are reflected in the URL query string and applied to every report.
- htmx partial updates for filter changes and imports.
- Every statistic in the UI carries a definition entry; load the dictionary from
  `docs/stat-definitions.md` at build time (`include_str!`) or a static map, and
  expose it as a tooltip/expandable.
- Charts: vendor a lightweight charting library into `src/web/static/` (uPlot is
  acceptable) rather than relying on a CDN. Downsampled points come from
  `queries::cumulative_series`.
- CSV export for the filtered aggregate table and for raw hand histories.
- Money formatted from `Money::format()`; never float.

## Tests

`tests/web.rs` with `router(state)` and `tower::ServiceExt::oneshot`: an empty
database renders each screen with HTTP 200; a filtered request applies the filter;
CSV export has the expected header and row count.
