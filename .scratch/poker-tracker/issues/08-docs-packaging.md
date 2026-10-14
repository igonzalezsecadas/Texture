# 08 — Statistic dictionary, packaging, snapshot docs

Status: ready-for-agent
Type: task
Blocked by: none

## Goal

The definitional and operational deliverables that must exist before (or
alongside) the reporting layer.

## Deliverables

1. `docs/stat-definitions.md` — one entry per statistic shown in the UI, using
   PT4/HM3 semantics, including edge-case rules for: limped pots, big-blind free
   checks, all-in without showdown, multiway c-bet attribution, 3-bet/4-bet
   opportunities, steal and fold-to-steal, WWSF/WTSD/W\$SD, aggression factor vs
   frequency, and all-in EV and rake attribution. **This is the reference the
   stats module implements; write it first and make it precise.**
2. `GLOSSARY.md` — the domain vocabulary (hand, seat, hero, alias, profile,
   session, stake, table size, position, quarantine, parser version, weighted
   contribution, all-in adjusted result, luck delta).
3. `docs/adr/` — at least: 0001 single-binary three-modes; 0002 SQLite + WAL +
   VACUUM INTO snapshots; 0003 integer minor units and UTC timestamps; 0004
   hero-centric parsed schema with raw text retained; 0005 server-rendered htmx
   UI with no JS build.
4. `Dockerfile` — multi-stage, non-root user, bundled SQLite, healthcheck against
   `/healthz`, exec-form entrypoint, clean signal handling.
5. `compose.yaml` — one command to start; hand-history dir mounted read-only;
   data dir bind-mounted read-write; port published to `127.0.0.1` only;
   environment for paths and snapshot schedule.
6. `README.md` — quick start, configuration reference (all `TEXTURE_*` env vars),
   import/reparse/snapshot commands, backup and restore procedure, and the
   reconciliation procedure against PokerTracker 4 with the agreed tolerance and
   the known-divergence list.
7. `.dockerignore`.

## Constraints

- The container must not need a database server or a second process.
- Snapshot/restore must be documented precisely enough to follow under stress.
- Do not invent statistics the spec does not list.
