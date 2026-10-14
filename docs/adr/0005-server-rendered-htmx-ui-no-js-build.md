# ADR-0005: Server-rendered htmx UI with no JavaScript build step

## Status

Accepted

## Context

The product is a single-user, localhost-only reporting tool. The interface needs:
a dashboard, preflop/postflop/position/session reports, a hand browser with raw
text side by side, import/quarantine status, settings, and a filter bar whose
active state is always visible — all over a data set of up to ten million hands.
The spec explicitly rules out "a single-page application, a mobile app, and any
JavaScript build pipeline", and the operator wants to maintain the whole thing
end to end without a node toolchain or client-state debugging.

## Decision

- Server-rendered HTML from templates; **htmx** for partial updates (filter bar
  re-issuing reports, import status polling, quarantine retry) with no client-side
  state machine. Progressive enhancement only: every screen works as plain HTML.
- No JavaScript build step, no bundler, no npm dependency; htmx is vendored as a
  single static asset. Charts use one lightweight charting library, also a static
  asset fed by chart endpoints.
- Chart endpoints are **downsampled server-side** (largest-triangle-three-buckets
  or equivalent) to a bounded number of points, so a cumulative graph over
  millions of hands renders as a fixed, small payload regardless of hand count.
- Interface binds to localhost only. No authentication, sessions, CSRF layer, or
  TLS is built — the deployment model (single user, published to 127.0.0.1 only)
  is what makes this acceptable, and that is a compose-level guarantee
  (`127.0.0.1` port publishing), not a property of the app to rely on happening.
- The filter bar appears on every report screen and its active state is always
  visible with one-click clear (spec stories 57–64); it is the single filter
  specification (ADR-0004's `FilterSpec`) rendered as HTML.

## Consequences

- The whole stack stays Rust plus templates plus two static assets; no node
  toolchain in the build, image, or developer workflow.
- Interactivity feels like link/navigate/partial-swap, not like an SPA; that is a
  deliberate trade for inspectability and simplicity.
- CSV export and raw hand export are plain routes over the same filter set
  (spec stories 76–77), so "what you see is what you export" holds by construction.
- If a feature ever demands real client state (a live HUD, drag filters), it
  would reopen this ADR — the parse/data layers are what such a feature would
  consume, and they are unaffected by this decision.
- Accessibility and no-JS behavior come nearly free from server rendering.
