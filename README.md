# texture

A self-hosted PokerStars No-Limit Hold'em cash-game tracker. One Rust binary,
one SQLite database, one container. It imports your hand histories
continuously while you play, computes the full statistic set a serious cash
player expects (results, BB/100, €/100, all-in EV, rake paid, preflop,
postflop, positional, sessions), and serves a minimalist browser UI —
server-rendered HTML with htmx, no JavaScript build step.

It is designed to be *verified*, not trusted: the same hand histories can be
imported into PokerTracker 4, and a documented reconciliation procedure
compares headline aggregates within a stated tolerance (see
[docs/reconciliation.md](docs/reconciliation.md)).

## Status of the Docker path — read this first

**Docker is NOT installed on the machine this project was developed on.** The
native-binary path (`cargo build --release` + run the binary directly) is
verified end to end. The Dockerfile and `compose.yaml` are written to the spec
and sanity-checked by inspection, but have **not been executed**. Treat the
first `docker compose up --build` as the real test; if something breaks, it is
a bug report against this packaging, not a mystery.

### What the native path was verified to do

Run against a real database with real imported hands:

- `import` backfilled a hand-history directory: `2 files, 4 hands, 0 duplicates,
  0 quarantined, 0 failures`.
- `serve` bound localhost, and `/healthz` returned
  `{"hands":4,"quarantined":0,"status":"ok","version":"0.1.0"}`.
- The dashboard reported real numbers, including the all-in adjustment: total won
  `€59.00`, all-in adjusted `€41.87`, luck delta `+€17.13` — the AA-vs-KK preflop
  all-in being adjusted for equity.
- **Continuous import**: appending a hand to a watched file was picked up
automatically (hand count went 4 → 5 within seconds) while the dashboard kept
  responding in ~7 ms.
- **Filters**: a filtered request returned a different population than the
  unfiltered one, and the active filter set was visible and clearable.
- **Export**: `/export/aggregate.csv` produced machine-readable decimals (no
  digit grouping), and `/export/hands.csv` carried the original hand text.
- **Snapshot and restore**: an automatic snapshot was taken on start, and the
  snapshot reopened as a database with matching `hands`, `hero_hands` and
  `raw_hands` counts.
- **Reparse**: `reparse --force` rebuilt all 5 hands from the retained raw text in
  ~0.6 s, and the reports were unchanged afterwards — the compatibility
  guarantee that raw text is the source of truth.
- **Shutdown**: `SIGTERM` stopped the process cleanly.

All of this is also covered by the test suite (`cargo test`, 115 tests), which
runs the real parser, the real storage layer and the real router against
throwaway databases.

## Quick start (Docker)

```sh
mkdir -p hands data
# copy or point your PokerStars hand histories at ./hands
docker compose up -d --build
# UI: http://127.0.0.1:8080
```

What this gives you:

- the hand-history folder mounted **read-only** (`./hands:/hands:ro`) — the
  container cannot modify or delete your history files;
- the database and snapshots on a host bind mount (`./data`) — rebuilding or
  upgrading the container never risks your data;
- the UI published on **127.0.0.1 only** (no authentication, TLS, or LAN
  access by design);
- automatic snapshots every 6 hours, 14 retained;
- a healthcheck against `/healthz`, and clean signal handling on stop.

Database files and snapshots live in `./data` (`texture.sqlite`,
`data/snapshots/texture-YYYYmmdd-HHMMSS.sqlite`).

## Quick start (native binary, verified path)

```sh
cargo build --release
mkdir -p hands data
./target/release/texture import hands          # one-shot backfill
./target/release/texture serve                 # UI + continuous watcher
# UI: http://127.0.0.1:8080 (default bind is 127.0.0.1:8080)
```

## Environment variable reference

Every `TEXTURE_*` variable read by the application (`src/config.rs`), with
defaults:

| Variable | Default | Meaning |
|---|---|---|
| `TEXTURE_DATA_DIR` | `./data` | Directory holding the SQLite database (`texture.sqlite`). Mounted read-write from the host in Docker. |
| `TEXTURE_HAND_HISTORY_DIR` | `./hands` | Directory containing PokerStars hand-history files. Mounted read-only in the container. `serve --hands` overrides this. |
| `TEXTURE_SNAPSHOT_DIR` | `<TEXTURE_DATA_DIR>/snapshots` | Where `VACUUM INTO` snapshots are written. |
| `TEXTURE_BIND_ADDR` | `127.0.0.1:8080` | Address the web UI binds to. Inside Docker this **must** be `0.0.0.0:8080` for the port mapping to work (compose.yaml sets it); the host-side loopback restriction comes from the port publish, not this variable. |
| `TEXTURE_SNAPSHOT_INTERVAL_SECS` | `21600` (6 h) | Seconds between automatic snapshots while `serve` runs. `0` disables automatic snapshots. |
| `TEXTURE_SNAPSHOT_RETENTION` | `14` | How many snapshots to keep; the oldest are pruned after each snapshot. |
| `TEXTURE_HERO_NAME` | *(unset)* | Optional override for the hero screen name, bypassing detection from the hand-history folder name. |
| `TEXTURE_SCAN_INTERVAL_SECS` | `30` | Filesystem poll fallback interval for the watcher, in seconds. |
| `TEXTURE_LOG_JSON` | *(off)* | Set to `1` or `true` to emit logs as JSON. |

Log filtering additionally uses the standard `RUST_LOG` (tracing `EnvFilter`).
Default filter: `texture=info,tower_http=info,warn`.

User-level preferences (active hero profile, session idle gap, currency
confirmation) are **not** environment variables; they live in the settings
table inside the database and are edited from the UI's settings screen.

## CLI reference

The binary has four subcommands. All of them read the environment above; only
`serve` runs indefinitely.

### `texture serve`

Runs the web UI, the continuous hand-history watcher, and the importer as one
process. The importer is a bounded background task using a single SQLite
writer connection; report queries use a read pool, so browsing during a large
import stays responsive. Reacts to file creation/modification, re-scans on
startup, skips unchanged files, and holds partially written hands until the
summary section is complete (reading a file mid-write is safe).

```sh
texture serve                          # watch TEXTURE_HAND_HISTORY_DIR
texture serve --hands /path/to/hands   # override the watched directory
```

Shutdown: SIGINT or SIGTERM (Ctrl-C, `docker stop`) triggers a clean shutdown —
the watcher stops and any in-flight snapshot is flushed before exit.

### `texture import [PATH] [--force]`

Bulk-import a hand-history directory. Shares one code path with continuous
import, so behavior cannot diverge. Duplicate hands are ignored by PokerStars
hand ID (re-import is idempotent); hands that fail to parse are quarantined
with a reason, counted, and never halt the file.

```sh
texture import                    # import TEXTURE_HAND_HISTORY_DIR
texture import /path/to/archive   # import a specific directory
texture import --force            # re-read files already recorded in file_state
```

Progress is reported every 2 seconds: files done/total, hands imported,
duplicates, failures. The final line reports totals including the quarantine
count.

### `texture reparse [--force]`

Rebuild parsed rows from the retained raw hand text. Use after a parser fix or
parser-version bump: every parsed hand is tagged with the parser version that
produced it. Without `--force`, only hands at an older parser version are
re-processed; with `--force`, everything is re-parsed. Progress is printed
every ~1000 hands.

```sh
texture reparse          # re-parse hands whose parser version is stale
texture reparse --force  # re-parse everything
```

### `texture snapshot`

Take a database snapshot now (a `VACUUM INTO` copy, consistent while the
application keeps running) and prune old ones per `TEXTURE_SNAPSHOT_RETENTION`.

```sh
texture snapshot
# snapshot written to ./data/snapshots/texture-20251014-210501.sqlite
```

## Workflows

### Initial backfill

1. Get your hand-history folder (see [Getting hand histories](#getting-hand-histories)).
2. `texture import hands` (or `docker compose exec texture import` in Docker).
3. Watch the progress line; at the end, note `hands imported`, `duplicates`,
   and `quarantined`. Quarantined hands are visible in the UI with their
   failure reason — decide whether a report is trustworthy from that count.
4. Run `texture import` a second time to confirm idempotency: it should report
   only duplicates and change nothing.

### Playing live

Run `texture serve` (or `docker compose up -d`). New hands appear in reports
within seconds of being written by the PokerStars client; the UI stays
responsive during imports.

### After a parser upgrade

1. Check the quarantine screen: a parser fix usually recovers previously
   failed hands.
2. `texture reparse` to rebuild parsed data for hands at an older parser
   version. Use `--force` if instructed by release notes.
3. Re-run reconciliation (see [docs/reconciliation.md](docs/reconciliation.md))
   on a known range before trusting new numbers.

### Snapshots

- Automatic: every `TEXTURE_SNAPSHOT_INTERVAL_SECS` while `serve` runs; the
  first fires immediately on startup, giving you a known-good copy early.
- Manual: `texture snapshot`, or the manual trigger in the UI.
- Take a manual snapshot before every upgrade or reparse.

## Backup and restore

Snapshots are produced by SQLite's `VACUUM INTO`: a consistent, compact,
single-file copy of the database taken while the application keeps running.
No WAL bookkeeping, no copy-the-sidecars dance. Each snapshot is
`texture-YYYYmmdd-HHMMSS.sqlite` in `TEXTURE_SNAPSHOT_DIR`, and retention
(`TEXTURE_SNAPSHOT_RETENTION`) prunes the oldest after each snapshot.

### Backup

- **Routine:** automatic snapshots land in `data/snapshots/` on the host bind
  mount. Include that directory in whatever off-machine backup you use.
- **Before an upgrade or a big reparse:** `texture snapshot` (or the UI
  trigger) for a known-good point.
- **Cold copy alternative:** if `serve` is stopped, copying `texture.sqlite`
  alone is safe. If you copy it while `serve` is running, copy `texture.sqlite`
  **plus** `texture.sqlite-wal` and `texture.sqlite-shm` together — or better,
  use `texture snapshot`, which needs none of that care.

### Restore

Followable under stress:

1. Stop the tracker: `docker compose stop texture` (or Ctrl-C the process).
2. Confirm the database is not in use: no `texture.sqlite-wal` growth; the
   process is gone.
3. Copy the chosen snapshot over the live database:

   ```sh
   cp data/snapshots/texture-20251014-210501.sqlite data/texture.sqlite
   rm -f data/texture.sqlite-wal data/texture.sqlite-shm   # stale WAL must go
   ```

   Deleting the `-wal`/`-shm` files is essential: a leftover WAL from the old
   database applied onto a restored file is corruption waiting to happen.
4. Keep the displaced live database until you have verified the restore
   (`mv data/texture.sqlite data/texture.sqlite.pre-restore`).
5. Start again (`docker compose start texture` / `texture serve`) and check
   the dashboard totals against a remembered value or the last reconciliation.

Note on snapshots vs. the future: snapshots are whole-database copies, taken
on a schedule with retention, plus a manual trigger. That is the entire v1
backup story, and it is deliberate — no external database server means no
external backup tooling.

## Getting hand histories

PokerStars writes hand histories to a folder you control:

1. In the PokerStars client: **Settings → Account History → Saving/Trading
   History** — enable *Keep my hand history in* and choose a folder
   (e.g. `~/PokerStars/hands`); enable the writing of hand histories to disk.
2. Point texture at that folder: `TEXTURE_HAND_HISTORY_DIR`, or symlink it to
   `./hands`. Under Wine or a compatibility layer, the path is whatever the
   Wine prefix maps the folder to — pass that path; no code changes are
   needed for non-native paths.
3. Mount it **read-only** in Docker (compose.yaml already does
   `./hands:/hands:ro`).

Important caveats:

- **The PokerStars client purges old hand histories.** The on-disk folder may
  hold less history than you expect. The recovery path for the back catalog
  is PokerTracker 4's archive and its export function: export hands from PT4,
  put them in a folder, and `texture import` it. That is also the fastest
  source of the initial backfill.
- Only complete hands (with a summary section) are imported. A hand being
  written while the watcher reads the file is skipped until it is complete —
  no corrupt hands, ever.
- Hands you were not dealt into are not counted, so your hand count matches
  PokerTracker 4's.
- Import is idempotent: re-reading or appending to files never double-counts,
  because the PokerStars hand ID is the natural key.

## Verifying the numbers

Do not trust the tracker; verify it.

1. **Zero dropped hands.** After any import or reparse, `hands imported` +
   `quarantined` + `duplicates` must account for every hand in the source
   files. Quarantined hands are never silently dropped — check the quarantine
   screen and satisfy yourself that each reason is understood.
2. **Idempotency.** Import the same directory twice: the second run changes
   nothing (all duplicates).
3. **Filter consistency.** Apply the same filters (date range, stakes, table
   size, position, session, hero alias) to every report; the filter bar always
   shows the active filter set, so a filtered number is never mistaken for a
   lifetime number.
4. **Reconciliation against PokerTracker 4.** The real bar: import the same
   hands into PT4, compare hands, net result, BB/100, €/100, all-in adjusted
   result, and rake paid within the recorded tolerance. The full procedure,
   the agreed tolerance, and the known-divergence list are in
   [docs/reconciliation.md](docs/reconciliation.md).
5. **Spot-check parses.** For any statistic, open a hand in the hand browser:
   the parsed view and the original raw text are shown side by side, so every
   number can be traced to its source.

## Design notes

- **Money** is stored as integer minor units with an explicit ISO currency
  code on every money-bearing row. Floating-point money is forbidden.
- **Time** is stored as UTC plus the original local timestamp text; session
  gaps and date filters use UTC, so daylight saving and travel cannot corrupt
  session grouping.
- **Raw hand text** is stored in the database, one row per hand, with parser
  version and source file reference. Any parse decision can be revisited; the
  parser is isolated so another site could be added without touching the
  reporting layer.
- **Sessions** are derived at query time from ordered hero hands using a
  configurable idle gap (default 30 minutes), computed in UTC; simultaneous
  tables merge into one session because the gap is time-based. Session
  duration is derived from first/last hand timestamps and is labelled as
  approximate in the UI.
- **Snapshots** are `VACUUM INTO` copies with retention — see
  [Backup and restore](#backup-and-restore).

## Scope (v1)

Cash games only: PokerStars, NLHE, EUR. No tournaments, no Zoom, no Omaha, no
HUD, no opponent statistics, no multi-user support or authentication, no
remote/LAN access. See the project spec for the full out-of-scope list.
