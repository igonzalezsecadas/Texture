# 09 — Web UI refresh and browser hand-history upload

Status: done
Type: task
Blocked by: none

## Goal

Two user-facing improvements on top of the v1 UI:

1. A complete visual redesign that keeps the UI minimal and server-rendered but
   makes it feel fresher and more elegant: a warm neutral palette with a
   matching dark theme, a fixed sidebar, serif display headings, tabular
   figures, clearer cards, and refined tables/forms.
2. An upload path on the Import screen: drop one or more PokerStars `.txt`
   hand histories (or paste hand text) and import only the hands that are not
   already in the database. Duplicate detection is by PokerStars hand ID, so
   re-uploading is idempotent.

## Interface (frozen)

- `POST /import/upload`, `multipart/form-data` with repeated `files` parts and
  an optional `pasted` part. Returns the Import screen with an upload summary.
  Body limit: 64 MB (`MAX_UPLOAD_BYTES`).
- `importer::import_text(db, text, source, opts, progress) -> TextImportOutcome`
- `importer::detect_hero_from_text(text) -> Option<String>`
- `TextImportOutcome { hands_seen, hands_imported, duplicates, failures,
  quarantined, incomplete_trailing, duration_ms }`

## Behavior

- Uploads share the file importer's parser, hero resolution, quarantine, and
  hand-ID upsert rules. They do **not** touch `file_state`; provenance is
  recorded as `upload:<filename>`.
- A first upload with no active hero profile detects the hero from `Dealt to`
  lines and registers it before importing, so analysis rows exist immediately.
- Parse failures are quarantined and counted; a summary-less trailing hand is
  reported as an incomplete tail and not quarantined.
- The upload is recorded in `import_runs` as kind `upload`.
- The filter form parameter names were corrected to match `FilterView`
  (`stake`/`size`/`pos`/`profile`), so the stake and position selectors now
  actually reach the query. The redesign preserves the visible active-filter
  state, “Clear all”, and the `active-filters` hook the tests rely on.
- No JavaScript build step was added. CSS/JS remain embedded static assets; the
  chart and the dropzone are progressive enhancements.

## Tests

- `tests/upload.rs`: text import is idempotent; malformed hands are quarantined;
  a truncated tail is held back; hero analysis rows are written; hero detection.
- `tests/web.rs`: multipart upload adds new hands once and reports duplicates on
  re-upload; pasted text works; first upload registers the hero; an empty upload
  is a validation error.
- Existing web tests still pass (all screens, definitions, filters, sessions,
  CSV, hand detail, static assets).

## Comments

- Redesign is CSS + template only apart from the two numeric fields added to the
  dashboard summary for the showdown split bar. The uploaded-hand summary and
  source table are new view models on the Import screen.
- Docs updated: README (Web UI, upload workflow, provenance note, test count).
- Verified visually with headless Firefox at 1440px in light and dark themes and
  at 420px; the mobile nav wraps rather than hiding destinations.
