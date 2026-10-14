//! Hand, seat, raw-text, file-state, quarantine, and import-run persistence.
//!
//! All writes here go through [`Db::with_writer`]. Upserts are keyed on the
//! PokerStars hand id, which makes import idempotent.

use crate::domain::{Card, HeroHandAnalysis, ParsedHand, Position, Street};
use crate::error::StorageError;
use crate::money::Currency;
use crate::storage::{Db, schema};
use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    Inserted,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileState {
    pub path: String,
    pub size: i64,
    pub mtime_utc: i64,
    pub content_hash: String,
    pub parse_cursor: i64,
    pub last_imported: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineRow {
    pub id: i64,
    pub source_file: String,
    pub raw_text: String,
    pub reason: String,
    pub parser_version: u32,
    pub first_seen: i64,
    pub last_seen: i64,
    pub retry_count: i64,
}

#[derive(Debug, Clone, Default)]
pub struct ImportRunStats {
    pub files_seen: i64,
    pub hands_imported: i64,
    pub duplicates: i64,
    pub failures: i64,
}

/// A raw hand awaiting (re)parse.
#[derive(Debug, Clone)]
pub struct RawHand {
    pub hand_id: String,
    pub raw_text: String,
    pub source_file: String,
}

// ---------------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------------

pub fn encode_cards(cards: &[Card]) -> String {
    cards.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")
}

pub fn decode_cards(s: &str) -> Vec<Card> {
    s.split_whitespace().filter_map(Card::parse).collect()
}

pub fn encode_streets(streets: &[Street]) -> String {
    streets.iter().map(|s| s.code()).collect::<Vec<_>>().join(",")
}

pub fn decode_streets(s: &str) -> Vec<Street> {
    s.split(',')
        .filter_map(|p| Street::ALL.into_iter().find(|x| x.code() == p.trim()))
        .collect()
}

pub fn encode_bools<const N: usize>(v: &[bool; N]) -> String {
    v.iter().map(|b| if *b { '1' } else { '0' }).collect()
}

pub fn decode_bools<const N: usize>(s: &str, out: &mut [bool; N]) {
    for (i, b) in out.iter_mut().enumerate() {
        *b = s.as_bytes().get(i).map(|c| *c == b'1').unwrap_or(false);
    }
}

pub fn encode_u32s<const N: usize>(v: &[u32; N]) -> String {
    v.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(",")
}

pub fn decode_u32s<const N: usize>(s: &str, out: &mut [u32; N]) {
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = s.split(',').nth(i).and_then(|p| p.parse().ok()).unwrap_or(0);
    }
}

// ---------------------------------------------------------------------------
// Hand persistence
// ---------------------------------------------------------------------------

/// Insert or replace a parsed hand, its seats, its raw text, and (when the hero
/// was dealt in) its hero analysis row. Returns [`UpsertOutcome::Duplicate`] and
/// changes nothing when the hand id already exists.
pub fn upsert_hand(
    db: &Db,
    parsed: &ParsedHand,
    analysis: Option<&HeroHandAnalysis>,
    raw: &str,
    source_file: &str,
) -> Result<UpsertOutcome, StorageError> {
    db.with_writer(|conn| {
        let tx = conn.transaction()?;

        let exists: Option<i64> = tx
            .query_row("SELECT 1 FROM hands WHERE hand_id = ?1", [&parsed.hand_id], |r| r.get(0))
            .optional()?;
        if exists.is_some() {
            // Re-reads are harmless, but the raw text is retained regardless so a
            // later reparse can recover. Nothing else may change.
            tx.execute(
                "INSERT INTO raw_hands(hand_id, raw_text) VALUES (?1, ?2)
                 ON CONFLICT(hand_id) DO UPDATE SET raw_text = excluded.raw_text",
                params![&parsed.hand_id, raw],
            )?;
            tx.commit()?;
            return Ok(UpsertOutcome::Duplicate);
        }

        tx.execute(
            "INSERT INTO hands(
                hand_id, site, game, currency, sb_minor, bb_minor, table_name, table_size,
                button_seat, ts_utc, local_ts, tz_token, board, streets_dealt, pot_minor,
                rake_minor, jackpot_minor, went_to_showdown, parser_version, source_file, imported_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
             )",
            params![
                &parsed.hand_id,
                parsed.site.code(),
                parsed.game.code(),
                parsed.currency.code(),
                parsed.small_blind.minor,
                parsed.big_blind.minor,
                &parsed.table_name,
                parsed.table_size as i64,
                parsed.button_seat as i64,
                parsed.timestamp_utc.timestamp(),
                &parsed.local_timestamp,
                &parsed.tz_token,
                encode_cards(&parsed.board),
                encode_streets(&parsed.streets_dealt),
                parsed.pot_total.minor,
                parsed.rake.minor,
                parsed.jackpot_drop.minor,
                parsed.went_to_showdown as i64,
                schema::PARSER_VERSION as i64,
                source_file,
                Utc::now().timestamp(),
            ],
        )?;

        for seat in &parsed.seats {
            tx.execute(
                "INSERT INTO players(screen_name) VALUES (?1) ON CONFLICT(screen_name) DO NOTHING",
                [&seat.screen_name],
            )?;
            let player_id: i64 = tx.query_row(
                "SELECT id FROM players WHERE screen_name = ?1",
                [&seat.screen_name],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO hand_seats(
                    hand_id, seat, player_id, screen_name, starting_stack_minor, position,
                    contributed_minor, result_minor, shown_cards, sat_out
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    &parsed.hand_id,
                    seat.seat as i64,
                    player_id,
                    &seat.screen_name,
                    seat.starting_stack.minor,
                    seat.position.code(),
                    seat.contributed,
                    seat.result.minor,
                    encode_cards(&seat.shown_cards),
                    seat.sat_out as i64,
                ],
            )?;
        }

        tx.execute(
            "INSERT INTO raw_hands(hand_id, raw_text) VALUES (?1, ?2)",
            params![&parsed.hand_id, raw],
        )?;

        if let Some(a) = analysis {
            insert_hero_analysis(&tx, a)?;
        }

        tx.commit()?;
        Ok(UpsertOutcome::Inserted)
    })
}

fn insert_hero_analysis(tx: &rusqlite::Transaction<'_>, a: &HeroHandAnalysis) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO hero_hands(
            hand_id, profile_id, player_id, ts_utc, sb_minor, bb_minor, currency, table_size, position,
            net_result_minor, adjusted_result_minor, luck_delta_minor, rake_attributed_minor,
            jackpot_attributed_minor, contributed_minor, won_amount_minor, sd_winnings_minor,
            non_sd_winnings_minor,
            went_to_showdown, won_at_showdown, saw_flop, saw_turn, saw_river,
            wwsf_opportunity, won_when_saw_flop,
            vpip, pfr, limp, rfi, rfi_opportunity, three_bet, three_bet_opportunity,
            faced_three_bet, folded_to_three_bet, four_bet, four_bet_opportunity,
            faced_four_bet, folded_to_four_bet, cold_call, cold_call_opportunity,
            steal_attempt, steal_opportunity, faced_steal, folded_to_steal,
            cbet_opportunity, cbet, faced_cbet, folded_to_cbet, check_raise_opportunity, check_raise,
            probe_opportunity, probe, delayed_cbet_opportunity, delayed_cbet,
            aggressive_actions, passive_actions
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9,
            ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
            ?19, ?20, ?21, ?22, ?23, ?24, ?25,
            ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36, ?37, ?38, ?39, ?40, ?41,
            ?42, ?43, ?44, ?45, ?46, ?47, ?48, ?49, ?50, ?51, ?52, ?53, ?54, ?55, ?56
         )",
        params![
            &a.hand_id,
            a.profile_id,
            a.player_id,
            a.timestamp_utc.timestamp(),
            a.small_blind,
            a.big_blind,
            a.currency.code(),
            a.table_size as i64,
            a.position.code(),
            a.net_result,
            a.adjusted_result,
            a.luck_delta,
            a.rake_attributed,
            a.jackpot_attributed,
            a.contributed,
            a.won_amount,
            a.sd_winnings,
            a.non_sd_winnings,
            a.went_to_showdown as i64,
            a.won_at_showdown as i64,
            a.saw_flop as i64,
            a.saw_turn as i64,
            a.saw_river as i64,
            a.wwsf_opportunity as i64,
            a.won_when_saw_flop as i64,
            a.vpip as i64,
            a.pfr as i64,
            a.limp as i64,
            a.rfi as i64,
            a.rfi_opportunity as i64,
            a.three_bet as i64,
            a.three_bet_opportunity as i64,
            a.faced_three_bet as i64,
            a.folded_to_three_bet as i64,
            a.four_bet as i64,
            a.four_bet_opportunity as i64,
            a.faced_four_bet as i64,
            a.folded_to_four_bet as i64,
            a.cold_call as i64,
            a.cold_call_opportunity as i64,
            a.steal_attempt as i64,
            a.steal_opportunity as i64,
            a.faced_steal as i64,
            a.folded_to_steal as i64,
            encode_bools(&a.cbet_opportunity),
            encode_bools(&a.cbet),
            encode_bools(&a.faced_cbet),
            encode_bools(&a.folded_to_cbet),
            encode_bools(&a.check_raise_opportunity),
            encode_bools(&a.check_raise),
            encode_bools(&a.probe_opportunity),
            encode_bools(&a.probe),
            encode_bools(&a.delayed_cbet_opportunity),
            encode_bools(&a.delayed_cbet),
            encode_u32s(&a.aggressive_actions),
            encode_u32s(&a.passive_actions),
        ],
    )?;
    Ok(())
}

/// Replace an existing hero analysis row (used by reparse).
pub fn replace_hero_analysis(db: &Db, a: &HeroHandAnalysis) -> Result<(), StorageError> {
    db.with_writer(|conn| {
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM hero_hands WHERE hand_id = ?1", [&a.hand_id])?;
        insert_hero_analysis(&tx, a)?;
        tx.commit()?;
        Ok(())
    })
}

/// Delete every derived row for a hand (hands, seats, hero analysis) but keep the
/// retained raw text, so a reparse can rebuild it.
pub fn clear_derived(db: &Db, hand_id: &str) -> Result<(), StorageError> {
    db.with_writer(|conn| {
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM hand_seats WHERE hand_id = ?1", [hand_id])?;
        tx.execute("DELETE FROM hero_hands WHERE hand_id = ?1", [hand_id])?;
        tx.execute("DELETE FROM hands WHERE hand_id = ?1", [hand_id])?;
        tx.commit()?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// File state
// ---------------------------------------------------------------------------

pub fn get_file_state(db: &Db, path: &str) -> Result<Option<FileState>, StorageError> {
    db.with_reader(|c| {
        c.query_row(
            "SELECT path, size, mtime_utc, content_hash, parse_cursor, last_imported
             FROM file_state WHERE path = ?1",
            [path],
            |r| {
                Ok(FileState {
                    path: r.get(0)?,
                    size: r.get(1)?,
                    mtime_utc: r.get(2)?,
                    content_hash: r.get(3)?,
                    parse_cursor: r.get(4)?,
                    last_imported: r.get(5)?,
                })
            },
        )
        .optional()
    })
}

pub fn put_file_state(db: &Db, state: &FileState) -> Result<(), StorageError> {
    db.with_writer(|c| {
        c.execute(
            "INSERT INTO file_state(path, size, mtime_utc, content_hash, parse_cursor, last_imported)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path) DO UPDATE SET
                size = excluded.size,
                mtime_utc = excluded.mtime_utc,
                content_hash = excluded.content_hash,
                parse_cursor = excluded.parse_cursor,
                last_imported = excluded.last_imported",
            params![
                &state.path,
                state.size,
                state.mtime_utc,
                &state.content_hash,
                state.parse_cursor,
                state.last_imported,
            ],
        )
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Quarantine
// ---------------------------------------------------------------------------

pub fn quarantine(
    db: &Db,
    source_file: &str,
    raw: &str,
    reason: &str,
    parser_version: u32,
) -> Result<(), StorageError> {
    db.with_writer(|c| {
        let now = Utc::now().timestamp();
        // Collapse identical failures for the same raw text into one row.
        let existing: Option<i64> = c
            .query_row(
                "SELECT id FROM quarantine WHERE source_file = ?1 AND raw_text = ?2",
                params![source_file, raw],
                |r| r.get(0),
            )
            .optional()?;
        match existing {
            Some(id) => {
                c.execute(
                    "UPDATE quarantine SET last_seen = ?1, reason = ?2, parser_version = ?3 WHERE id = ?4",
                    params![now, reason, parser_version as i64, id],
                )?;
            }
            None => {
                c.execute(
                    "INSERT INTO quarantine(source_file, raw_text, reason, parser_version, first_seen, last_seen)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    params![source_file, raw, reason, parser_version as i64, now],
                )?;
            }
        }
        Ok(())
    })?;
    Ok(())
}

pub fn list_quarantine(db: &Db, limit: i64, offset: i64) -> Result<Vec<QuarantineRow>, StorageError> {
    db.with_reader(|c| {
        let mut stmt = c.prepare(
            "SELECT id, source_file, raw_text, reason, parser_version, first_seen, last_seen, retry_count
             FROM quarantine ORDER BY last_seen DESC LIMIT ?1 OFFSET ?2",
        )?;
        let rows = stmt
            .query_map(params![limit, offset], |r| {
                Ok(QuarantineRow {
                    id: r.get(0)?,
                    source_file: r.get(1)?,
                    raw_text: r.get(2)?,
                    reason: r.get(3)?,
                    parser_version: r.get::<_, i64>(4)? as u32,
                    first_seen: r.get(5)?,
                    last_seen: r.get(6)?,
                    retry_count: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

pub fn quarantine_count(db: &Db) -> Result<i64, StorageError> {
    db.with_reader(|c| c.query_row("SELECT COUNT(*) FROM quarantine", [], |r| r.get(0)))
}

pub fn bump_quarantine_retry(db: &Db, id: i64) -> Result<(), StorageError> {
    db.with_writer(|c| c.execute("UPDATE quarantine SET retry_count = retry_count + 1 WHERE id = ?1", [id]))?;
    Ok(())
}

pub fn delete_quarantine(db: &Db, id: i64) -> Result<(), StorageError> {
    db.with_writer(|c| c.execute("DELETE FROM quarantine WHERE id = ?1", [id]))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Import runs
// ---------------------------------------------------------------------------

pub fn start_import_run(db: &Db, kind: &str) -> Result<i64, StorageError> {
    db.with_writer(|c| {
        c.execute(
            "INSERT INTO import_runs(started_at, kind) VALUES (?1, ?2)",
            params![Utc::now().timestamp(), kind],
        )?;
        Ok(c.last_insert_rowid())
    })
}

pub fn finish_import_run(db: &Db, id: i64, stats: &ImportRunStats) -> Result<(), StorageError> {
    db.with_writer(|c| {
        c.execute(
            "UPDATE import_runs SET finished_at = ?1, files_seen = ?2, hands_imported = ?3,
             duplicates = ?4, failures = ?5 WHERE id = ?6",
            params![
                Utc::now().timestamp(),
                stats.files_seen,
                stats.hands_imported,
                stats.duplicates,
                stats.failures,
                id
            ],
        )
    })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Raw hands / reparse support
// ---------------------------------------------------------------------------

pub fn raw_hand(db: &Db, hand_id: &str) -> Result<Option<String>, StorageError> {
    db.with_reader(|c| {
        c.query_row("SELECT raw_text FROM raw_hands WHERE hand_id = ?1", [hand_id], |r| r.get(0))
            .optional()
    })
}

pub fn raw_hand_count(db: &Db) -> Result<i64, StorageError> {
    db.with_reader(|c| c.query_row("SELECT COUNT(*) FROM raw_hands", [], |r| r.get(0)))
}

/// Raw hands whose stored parser version is below `version`, or all raw hands
/// when `force`. `source_file` comes from the derived row when it still exists.
pub fn raw_hands_needing_reparse(
    db: &Db,
    version: u32,
    force: bool,
    limit: i64,
    offset: i64,
) -> Result<Vec<RawHand>, StorageError> {
    db.with_reader(|c| {
        let sql = if force {
            "SELECT r.hand_id, r.raw_text, COALESCE(h.source_file, '')
             FROM raw_hands r LEFT JOIN hands h ON h.hand_id = r.hand_id
             ORDER BY r.hand_id LIMIT ?1 OFFSET ?2"
        } else {
            "SELECT r.hand_id, r.raw_text, COALESCE(h.source_file, '')
             FROM raw_hands r LEFT JOIN hands h ON h.hand_id = r.hand_id
             WHERE h.hand_id IS NULL OR h.parser_version < ?3
             ORDER BY r.hand_id LIMIT ?1 OFFSET ?2"
        };
        let mut stmt = c.prepare(sql)?;
        let map = |r: &rusqlite::Row<'_>| {
            Ok(RawHand { hand_id: r.get(0)?, raw_text: r.get(1)?, source_file: r.get(2)? })
        };
        let rows = if force {
            stmt.query_map(params![limit, offset], map)?.collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map(params![limit, offset, version as i64], map)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(rows)
    })
}

/// Total number of hands still needing a reparse (for progress reporting).
pub fn reparse_pending(db: &Db, version: u32, force: bool) -> Result<i64, StorageError> {
    db.with_reader(|c| {
        if force {
            c.query_row("SELECT COUNT(*) FROM raw_hands", [], |r| r.get(0))
        } else {
            c.query_row(
                "SELECT COUNT(*) FROM raw_hands r LEFT JOIN hands h ON h.hand_id = r.hand_id
                 WHERE h.hand_id IS NULL OR h.parser_version < ?1",
                [version as i64],
                |r| r.get(0),
            )
        }
    })
}

// ---------------------------------------------------------------------------
// Small accessors used by reporting/UI
// ---------------------------------------------------------------------------

pub fn hand_count_total(db: &Db) -> Result<i64, StorageError> {
    db.with_reader(|c| c.query_row("SELECT COUNT(*) FROM hands", [], |r| r.get(0)))
}

pub fn latest_import_run(db: &Db) -> Result<Option<(String, i64, i64, i64, i64, Option<i64>)>, StorageError> {
    db.with_reader(|c| {
        c.query_row(
            "SELECT kind, started_at, hands_imported, duplicates, failures, finished_at
             FROM import_runs ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
        )
        .optional()
    })
}

pub fn currency_code(c: Currency) -> &'static str {
    c.code()
}

pub fn ts_seconds(t: DateTime<Utc>) -> i64 {
    t.timestamp()
}

pub fn position_code(p: Position) -> &'static str {
    p.code()
}
