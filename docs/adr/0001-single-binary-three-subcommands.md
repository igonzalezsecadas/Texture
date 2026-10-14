# ADR-0001: One Rust binary with three subcommands (`serve`, `import`, `reparse`)

## Status

Accepted

## Context

The tracker must run as a single Docker container for one user. The work splits
into three operational modes that share almost all of their code: continuous
import while playing, backfilling an existing hand-history archive, and rebuilding
parsed data after a parser change. The first draft idea was separate service
processes (an importer daemon plus a web server plus a CLI).

Consequences of split processes: two binaries or a supervised pair, duplicated
configuration and signal handling, an extra failure mode inside the container, and
- the thing the spec most wants to avoid — a "database server or a second
process" (see ADR-0002 for why SQLite is the store). Splitting also invites
divergence between bulk import and continuous import unless they share one code
path (spec, Import behavior).

## Decision

Ship exactly one Rust binary with three subcommands:

- `serve` — the long-running web process. It binds localhost, serves the UI, and
  runs the folder watcher and importer as a bounded background task inside the
  same process: one writer connection streaming one hand at a time through a
  bounded channel, committing in batches. This mode also honors the healthcheck
  and structured-log requirements.
- `import` — bulk import of a directory with visible progress (files, hands,
  duplicates, failures). Shares the same parse-file-import code path as `serve`.
- `reparse` — an explicit, resumable, long-running rebuild of parsed data from the
  raw hand text retained in the database, with progress. Deliberately isolated
  from `serve` so a disk-heavy operation never runs implicitly.

Configuration: environment variables for deployment concerns (paths, bind
address, snapshot schedule), the settings table for user-level concerns (active
hero profile, session gap).

## Consequences

- One image, one config surface, one process to keep alive; the compose file and
  healthcheck stay trivial.
- Bulk and continuous import cannot diverge behaviorally — same code path by
  construction (spec requirement 17/24: unchanged-file skipping and hand-ID
  upserts are exercised identically by both).
- A large `import` or `reparse` competes with `serve` for disk I/O if run in the
  same container simultaneously; acceptable for a single user, and `reparse` is
  opt-in for exactly this reason.
- Any future mode (e.g. an export command) is one more subcommand, not a new
  deployment artifact.
- The single-writer design (ADR-0002) is also what allows the web layer to use a
  read pool without coordinating with the importer — they are in one process by
  choice, not by accident.
