//! Schema definition and migrations.
//!
//! Conventions:
//! - Money columns are named `*_minor` and are INTEGER minor units.
//! - Time columns are named `*_utc` and are INTEGER Unix seconds.
//! - Boolean columns are INTEGER 0/1.
//! - `raw_hands.raw_text` is retained forever; parsed data must always be
//!   rebuildable from it.

/// Bump when the parser's output for existing hands could change. Every parsed
/// hand records the version that produced it so a reparse is auditable.
pub const PARSER_VERSION: u32 = 1;

pub const MIGRATIONS: &[&str] = &[
    // 1: initial schema
    r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS players (
    id          INTEGER PRIMARY KEY,
    screen_name TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS hero_profiles (
    id     INTEGER PRIMARY KEY,
    name   TEXT NOT NULL UNIQUE,
    active INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS hero_profile_players (
    profile_id INTEGER NOT NULL REFERENCES hero_profiles(id) ON DELETE CASCADE,
    player_id  INTEGER NOT NULL REFERENCES players(id) ON DELETE CASCADE,
    PRIMARY KEY (profile_id, player_id)
);

CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS hands (
    hand_id        TEXT PRIMARY KEY,
    site           TEXT NOT NULL,
    game           TEXT NOT NULL,
    currency       TEXT NOT NULL,
    sb_minor       INTEGER NOT NULL,
    bb_minor       INTEGER NOT NULL,
    table_name     TEXT NOT NULL,
    table_size     INTEGER NOT NULL,
    button_seat    INTEGER NOT NULL,
    ts_utc         INTEGER NOT NULL,
    local_ts       TEXT NOT NULL,
    tz_token       TEXT NOT NULL,
    board          TEXT NOT NULL,
    streets_dealt  TEXT NOT NULL,
    pot_minor      INTEGER NOT NULL,
    rake_minor     INTEGER NOT NULL,
    jackpot_minor  INTEGER NOT NULL DEFAULT 0,
    went_to_showdown INTEGER NOT NULL,
    parser_version INTEGER NOT NULL,
    source_file    TEXT NOT NULL,
    imported_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_hands_ts ON hands(ts_utc);
CREATE INDEX IF NOT EXISTS idx_hands_stakes ON hands(sb_minor, bb_minor);
CREATE INDEX IF NOT EXISTS idx_hands_tablesize ON hands(table_size);

CREATE TABLE IF NOT EXISTS hand_seats (
    hand_id      TEXT NOT NULL REFERENCES hands(hand_id) ON DELETE CASCADE,
    seat         INTEGER NOT NULL,
    player_id    INTEGER NOT NULL REFERENCES players(id),
    screen_name  TEXT NOT NULL,
    starting_stack_minor INTEGER NOT NULL,
    position     TEXT NOT NULL,
    contributed_minor    INTEGER NOT NULL,
    result_minor         INTEGER NOT NULL,
    shown_cards  TEXT NOT NULL,
    sat_out      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (hand_id, seat)
);
CREATE INDEX IF NOT EXISTS idx_seats_player ON hand_seats(player_id);

-- Hero analysis materialized at import time. Denormalizes the query dimensions
-- (time, stakes, table size) so reports never need a join to filter.
CREATE TABLE IF NOT EXISTS hero_hands (
    hand_id      TEXT PRIMARY KEY REFERENCES hands(hand_id) ON DELETE CASCADE,
    profile_id   INTEGER NOT NULL REFERENCES hero_profiles(id),
    player_id    INTEGER NOT NULL REFERENCES players(id),
    ts_utc       INTEGER NOT NULL,
    sb_minor     INTEGER NOT NULL,
    bb_minor     INTEGER NOT NULL,
    currency     TEXT NOT NULL,
    table_size   INTEGER NOT NULL,
    position     TEXT NOT NULL,

    net_result_minor      INTEGER NOT NULL,
    adjusted_result_minor INTEGER NOT NULL,
    luck_delta_minor      INTEGER NOT NULL,
    rake_attributed_minor INTEGER NOT NULL,
    jackpot_attributed_minor INTEGER NOT NULL,
    contributed_minor     INTEGER NOT NULL,
    won_amount_minor      INTEGER NOT NULL,
    sd_winnings_minor     INTEGER NOT NULL,
    non_sd_winnings_minor INTEGER NOT NULL,

    went_to_showdown INTEGER NOT NULL,
    won_at_showdown  INTEGER NOT NULL,
    saw_flop         INTEGER NOT NULL,
    saw_turn         INTEGER NOT NULL,
    saw_river        INTEGER NOT NULL,
    wwsf_opportunity INTEGER NOT NULL,
    won_when_saw_flop INTEGER NOT NULL,

    vpip INTEGER NOT NULL,
    pfr  INTEGER NOT NULL,
    limp INTEGER NOT NULL,
    rfi  INTEGER NOT NULL,
    rfi_opportunity INTEGER NOT NULL,
    three_bet INTEGER NOT NULL,
    three_bet_opportunity INTEGER NOT NULL,
    faced_three_bet INTEGER NOT NULL,
    folded_to_three_bet INTEGER NOT NULL,
    four_bet INTEGER NOT NULL,
    four_bet_opportunity INTEGER NOT NULL,
    faced_four_bet INTEGER NOT NULL,
    folded_to_four_bet INTEGER NOT NULL,
    cold_call INTEGER NOT NULL,
    cold_call_opportunity INTEGER NOT NULL,
    steal_attempt INTEGER NOT NULL,
    steal_opportunity INTEGER NOT NULL,
    faced_steal INTEGER NOT NULL,
    folded_to_steal INTEGER NOT NULL,

    cbet_opportunity TEXT NOT NULL,
    cbet TEXT NOT NULL,
    faced_cbet TEXT NOT NULL,
    folded_to_cbet TEXT NOT NULL,
    check_raise TEXT NOT NULL,
    probe_opportunity TEXT NOT NULL,
    probe TEXT NOT NULL,
    delayed_cbet_opportunity TEXT NOT NULL,
    delayed_cbet TEXT NOT NULL,

    aggressive_actions TEXT NOT NULL,
    passive_actions TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_hero_ts ON hero_hands(ts_utc);
CREATE INDEX IF NOT EXISTS idx_hero_profile_ts ON hero_hands(profile_id, ts_utc);
CREATE INDEX IF NOT EXISTS idx_hero_stakes ON hero_hands(bb_minor);
CREATE INDEX IF NOT EXISTS idx_hero_tablesize ON hero_hands(table_size);
CREATE INDEX IF NOT EXISTS idx_hero_position ON hero_hands(position);

CREATE TABLE IF NOT EXISTS raw_hands (
    hand_id  TEXT PRIMARY KEY,
    raw_text TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS file_state (
    path          TEXT PRIMARY KEY,
    size          INTEGER NOT NULL,
    mtime_utc     INTEGER NOT NULL,
    content_hash  TEXT NOT NULL,
    parse_cursor  INTEGER NOT NULL DEFAULT 0,
    last_imported INTEGER
);

CREATE TABLE IF NOT EXISTS quarantine (
    id             INTEGER PRIMARY KEY,
    source_file    TEXT NOT NULL,
    raw_text       TEXT NOT NULL,
    reason         TEXT NOT NULL,
    parser_version INTEGER NOT NULL,
    first_seen     INTEGER NOT NULL,
    last_seen      INTEGER NOT NULL,
    retry_count    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_quarantine_file ON quarantine(source_file);

CREATE TABLE IF NOT EXISTS import_runs (
    id           INTEGER PRIMARY KEY,
    started_at   INTEGER NOT NULL,
    finished_at  INTEGER,
    kind         TEXT NOT NULL,
    files_seen   INTEGER NOT NULL DEFAULT 0,
    hands_imported INTEGER NOT NULL DEFAULT 0,
    duplicates   INTEGER NOT NULL DEFAULT 0,
    failures     INTEGER NOT NULL DEFAULT 0,
    note         TEXT
);

CREATE TABLE IF NOT EXISTS snapshots (
    id         INTEGER PRIMARY KEY,
    path       TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    size_bytes INTEGER NOT NULL
);
"#,
];
