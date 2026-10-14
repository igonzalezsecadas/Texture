# ADR-0002: SQLite in WAL mode with `VACUUM INTO` snapshots

## Status

Accepted

## Context

All hand data must live locally in the same container the user already runs, on a
host bind mount so rebuilding the container never risks it. The problem statement
explicitly rejects operating "an external database server" and a "second process".
Workloads are: one background writer (bounded channel, batched commits, indexes
dropped and rebuilt around bulk import) and a read-heavy web layer that must stay
responsive during imports (spec, Storage). The container filesystem must produce a
backup story the user understands and can verify, not an exotic one.

## Decision

Use bundled SQLite as the only data store, configured as:

- **WAL journal mode**, `synchronous=NORMAL`, a busy timeout, and a tuned page
  cache. One writer connection, many readers. WAL is what lets repeated report
  queries proceed under the writer without readers ever blocking it.
- Batch commits of a few thousand hands during import; drop and rebuild indexes
  around bulk import rather than maintaining them per hand.
- **Snapshots via `VACUUM INTO`** (rather than file copy, WAL checkpoint juggling,
  or the `.backup` API): a snapshot is a single self-contained database file
  written by SQLite itself, safe to run while the writer is active, and restorable
  by replacing the database file. Snapshots run on a configurable schedule with a
  retention policy, plus a manual trigger, both documented in the README.
- Database file and snapshots live on the host bind mount.

## Consequences

- No daemon to run, no replication or credentials to manage; the whole state is
  one directory on the host.
- Readers never block the importer and vice versa — report latency targets
  (interactive at 10M hands with filters) rest on WAL plus indexes, not on the
  process model.
- Snapshot cost is roughly proportional to database size (VACUUM rewrites the
  file); the retention policy exists for this. A short-lived write lock during
  `VACUUM INTO` is acceptable because the importer tolerates a busy timeout.
- Restoring a snapshot is a documented file replacement with the container
  stopped — it must be followable under stress, so the README spells out the
  exact steps (build failure criterion from ticket 08).
- `VACUUM INTO` output does not inherit WAL mode; a restored snapshot should be
  re-set to WAL on first open. Known and handled by schema initialization.
- If a future feature needs real concurrency between database writers, SQLite is
  the wrong tool and this ADR is the one to reopen.
