//! Report queries. Every report screen goes through [`aggregate`], which builds
//! its SQL from one [`FilterSpec`], so no two statistics can be computed on a
//! different population.

use crate::domain::{FilterSpec, GroupBy};
use crate::error::StorageError;
use crate::storage::Db;
use serde::Serialize;

/// One aggregated population (a total, or one group within a `GroupBy`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct AggregateRow {
    pub hands: i64,
    pub net_result_minor: i64,
    pub adjusted_result_minor: i64,
    pub luck_delta_minor: i64,
    pub rake_attributed_minor: i64,
    pub bb_won: f64,
    pub bb_per_100: f64,
    pub currency_per_100: f64,
    pub total_big_blinds: f64,

    pub showdown_hands: i64,
    pub sd_winnings_minor: i64,
    pub non_sd_winnings_minor: i64,

    pub vpip: i64,
    pub pfr: i64,
    pub limp: i64,
    pub rfi: i64,
    pub rfi_opportunity: i64,
    pub three_bet: i64,
    pub three_bet_opportunity: i64,
    pub faced_three_bet: i64,
    pub folded_to_three_bet: i64,
    pub four_bet: i64,
    pub four_bet_opportunity: i64,
    pub faced_four_bet: i64,
    pub folded_to_four_bet: i64,
    pub cold_call: i64,
    pub cold_call_opportunity: i64,
    pub steal_attempt: i64,
    pub steal_opportunity: i64,
    pub faced_steal: i64,
    pub folded_to_steal: i64,

    pub cbet_opportunity: [i64; 3],
    pub cbet: [i64; 3],
    pub faced_cbet: [i64; 3],
    pub folded_to_cbet: [i64; 3],
    pub check_raise: [i64; 3],
    pub probe_opportunity: [i64; 3],
    pub probe: [i64; 3],
    pub delayed_cbet_opportunity: [i64; 3],
    pub delayed_cbet: [i64; 3],

    pub aggressive_actions: [i64; 4],
    pub passive_actions: [i64; 4],

    pub wwsf_opportunity: i64,
    pub won_when_saw_flop: i64,
    pub wtsd_opportunity: i64,
    pub wtsd: i64,
    pub won_at_showdown: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AggregateGroup {
    pub key: String,
    pub label: String,
    pub row: AggregateRow,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeriesPoint {
    pub t: i64,
    pub hands: i64,
    pub net_minor: i64,
    pub adjusted_minor: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HandListRow {
    pub hand_id: String,
    pub ts_utc: i64,
    pub stakes: String,
    pub table_size: u8,
    pub position: String,
    pub net_result_minor: i64,
    pub currency: String,
}

/// Aggregate hero hands matching `filter`, grouped by `group`.
pub fn aggregate(db: &Db, filter: &FilterSpec, group: GroupBy) -> Result<Vec<AggregateGroup>, StorageError> {
    let _ = (db, filter, group);
    unimplemented!("storage::queries::aggregate")
}

/// Cumulative winnings series, downsampled server-side to at most `max_points`.
pub fn cumulative_series(db: &Db, filter: &FilterSpec, max_points: usize) -> Result<Vec<SeriesPoint>, StorageError> {
    let _ = (db, filter, max_points);
    unimplemented!("storage::queries::cumulative_series")
}

pub fn hand_list(db: &Db, filter: &FilterSpec, limit: i64, offset: i64) -> Result<Vec<HandListRow>, StorageError> {
    let _ = (db, filter, limit, offset);
    unimplemented!("storage::queries::hand_list")
}

pub fn hand_count(db: &Db, filter: &FilterSpec) -> Result<i64, StorageError> {
    let _ = (db, filter);
    unimplemented!("storage::queries::hand_count")
}

/// Distinct stake options available, for the filter bar.
pub fn stake_options(db: &Db) -> Result<Vec<(i64, i64, String)>, StorageError> {
    let _ = db;
    unimplemented!("storage::queries::stake_options")
}
