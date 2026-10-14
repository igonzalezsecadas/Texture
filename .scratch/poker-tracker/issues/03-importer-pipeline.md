# 03 — Import pipeline (bulk + watcher)

Status: done
Type: task
Blocked by: 01, 02

## Goal

Implement `src/importer/mod.rs`: bulk directory import, single-file import, the
continuous watcher, and reparse. Bulk and continuous import share one code path.

## Interface (frozen)

See `src/importer/mod.rs`. `ImportProgress`, `ImportOutcome`, `ImportOptions`,
`import_directory`, `import_file`, `reparse`, `run_watcher`, `detect_hero_name`,
`list_files` are already declared.

## Behavior contract

- Unchanged files are skipped using `file_state(path, size, mtime, content_hash,
  parse_cursor)`.
- Only hands terminated by a summary section are imported; the trailing
  incomplete hand is held until the file grows. Use
  `parser::split_complete_hands` and its `consumed` offset as the parse cursor.
- Parse failures quarantine the hand with a reason and never stop the file.
- Upserts are keyed on hand id; re-reads are harmless.
- Writer batches: commit every few hundred hands.
- `reparse` walks `raw_hands`, re-parses with the current `PARSER_VERSION`,
  rebuilds derived rows, and is resumable (skip hands already at the current
  version unless `force`).
- Watcher: `notify` on the hand-history dir, debounced, plus a startup scan and
  a polling fallback every `cfg.scan_interval_secs`. Reports progress through
  `web::ImporterHandle`.
- Never panic on a missing or unreadable file: report and continue.

## Tests (`tests/import_pipeline.rs`)

- importing the same directory twice changes nothing
- appending hands to an existing file imports only the new hands
- a file truncated mid-hand imports nothing until the hand is completed
- a malformed hand is quarantined with a reason and does not stop the file
- a quarantine retry after a parser fix imports the hand
- a missing/unreadable file is reported, not fatal

## Definition of done

`cargo test --test import_pipeline` passes; no `unimplemented!()` left in
`src/importer/`.
