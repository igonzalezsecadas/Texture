//! Report queries. Every report screen goes through [`aggregate`], which builds
//! its SQL from one [`FilterSpec`], so no two statistics can be computed on a
//! different population.

use crate::domain::{FilterSpec, GroupBy, Position};
use crate::error::StorageError;
use crate::money::Currency;
use crate::storage::hands::{decode_bools, decode_u32s};
use crate::storage::Db;
use chrono::Utc;
use rusqlite::types::Value;
use rusqlite::params_from_iter;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// One aggregated population (a total, or one group within a `GroupBy`).
///
/// Stat definitions (all over the filtered population):
/// - `hands` = COUNT(*) over hero_hands rows.
/// - Money fields (`net_result_minor`, `adjusted_result_minor`, ...) are plain
///   SUMs of the corresponding columns.
/// - `bb_won` = Σ per hand of `net_result_minor / bb_minor` (total net expressed
///   in big blinds; each hand divided by its own stake so mixed-stake groups are
///   honest).
/// - `bb_per_100` = `bb_won / hands * 100`.
/// - `currency_per_100` = total net in major units per 100 hands =
///   `(Σ net_result_minor / 100.0) / hands * 100` (EUR minor units have 2
///   decimals, so this is exactly `Σ net / hands` in major units).
/// - `total_big_blinds` = `Σ bb_minor / 100.0` (Sum of all blinds paid, in major
///   currency units).
/// - Flag counts are plain SUMs of the 0/1 flag columns.
/// - Street-indexed arrays (`cbet_*`, `probe_*`, ...) are `[flop, turn, river]`.
/// - `aggressive_actions`/`passive_actions` are `[preflop, flop, turn, river]`.
/// - `showdown_hands` = SUM(went_to_showdown); `wtsd` = the same numerator,
///   measured against `wtsd_opportunity` = SUM(saw_flop) (went-to-showdown % is
///   conventionally quoted of flop-seen hands).
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
    pub check_raise_opportunity: [i64; 3],
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

/// `hero_hands` column list selected into [`HandRow`]. One source of truth so a
/// schema change breaks loudly instead of silently shifting columns (values are
/// read by column *name*, so the order here is only for readability).
const ROW_COLUMNS: &str = "\
    hand_id, ts_utc, sb_minor, bb_minor, position, currency, \
    net_result_minor, adjusted_result_minor, luck_delta_minor, rake_attributed_minor, \
    sd_winnings_minor, non_sd_winnings_minor, \
    went_to_showdown, won_at_showdown, saw_flop, saw_turn, saw_river, \
    wwsf_opportunity, won_when_saw_flop, \
    vpip, pfr, limp, rfi, rfi_opportunity, three_bet, three_bet_opportunity, \
    faced_three_bet, folded_to_three_bet, four_bet, four_bet_opportunity, \
    faced_four_bet, folded_to_four_bet, cold_call, cold_call_opportunity, \
    steal_attempt, steal_opportunity, faced_steal, folded_to_steal, \
    cbet_opportunity, cbet, faced_cbet, folded_to_cbet, check_raise_opportunity, check_raise, \
    probe_opportunity, probe, delayed_cbet_opportunity, delayed_cbet, \
    aggressive_actions, passive_actions";

/// One materialized hero hand used by the in-Rust folds. `session` is filled by
/// [`apply_session_filter`], never read from SQLite.
#[derive(Debug, Clone)]
struct HandRow {
    hand_id: String,
    ts: i64,
    sb: i64,
    bb: i64,
    position: String,
    currency: String,
    net: i64,
    adjusted: i64,
    luck: i64,
    rake: i64,
    went_to_showdown: i64,
    won_at_showdown: i64,
    saw_flop: i64,
    wwsf_opportunity: i64,
    won_when_saw_flop: i64,
    sd_winnings: i64,
    non_sd_winnings: i64,
    vpip: i64,
    pfr: i64,
    limp: i64,
    rfi: i64,
    rfi_opportunity: i64,
    three_bet: i64,
    three_bet_opportunity: i64,
    faced_three_bet: i64,
    folded_to_three_bet: i64,
    four_bet: i64,
    four_bet_opportunity: i64,
    faced_four_bet: i64,
    folded_to_four_bet: i64,
    cold_call: i64,
    cold_call_opportunity: i64,
    steal_attempt: i64,
    steal_opportunity: i64,
    faced_steal: i64,
    folded_to_steal: i64,
    cbet_opportunity: String,
    cbet: String,
    faced_cbet: String,
    folded_to_cbet: String,
    check_raise_opportunity: String,
    check_raise: String,
    probe_opportunity: String,
    probe: String,
    delayed_cbet_opportunity: String,
    delayed_cbet: String,
    aggressive_actions: String,
    passive_actions: String,
    session: i64,
}

/// One `FilterSpec` → one SQL predicate, built once and reused by every query in
/// this module. `sessions` is deliberately excluded: session ids are derived at
/// query time (see [`apply_session_filter`]) and applied in Rust.
#[derive(Debug, Default)]
struct Predicate {
    clauses: Vec<String>,
    params: Vec<Value>,
}

impl Predicate {
    fn where_sql(&self) -> String {
        if self.clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.clauses.join(" AND "))
        }
    }

    fn push_cmp(&mut self, col: &str, op: &str, v: Value) {
        let n = self.params.len() + 1;
        self.clauses.push(format!("{col} {op} ?{n}"));
        self.params.push(v);
    }

    fn push_in(&mut self, col: &str, vals: Vec<Value>) {
        if vals.is_empty() {
            return;
        }
        let base = self.params.len();
        let phs: Vec<String> =
            (0..vals.len()).map(|i| format!("?{}", base + i + 1)).collect();
        self.clauses.push(format!("{col} IN ({})", phs.join(", ")));
        self.params.extend(vals);
    }

    /// Extends the predicate with `AND col IN (...)` using fresh placeholders.
    fn and_in(&mut self, col: &str, vals: Vec<Value>) {
        if vals.is_empty() || self.clauses.is_empty() {
            self.push_in(col, vals);
        } else {
            let base = self.params.len();
            let phs: Vec<String> =
                (0..vals.len()).map(|i| format!("?{}", base + i + 1)).collect();
            self.clauses.push(format!("{col} IN ({})", phs.join(", ")));
            self.params.extend(vals);
        }
    }
}

fn build_predicate(filter: &FilterSpec) -> Predicate {
    let mut p = Predicate::default();
    if let Some(from) = filter.from {
        // `from` is inclusive on UTC hand time.
        p.push_cmp("ts_utc", ">=", Value::Integer(from.timestamp()));
    }
    if let Some(to) = filter.to {
        // `to` is exclusive.
        p.push_cmp("ts_utc", "<", Value::Integer(to.timestamp()));
    }
    if !filter.stakes.is_empty() {
        let base = p.params.len();
        let parts: Vec<String> = filter
            .stakes
            .iter()
            .enumerate()
            .map(|(i, (_sb, _bb))| {
                format!(
                    "(sb_minor = ?{} AND bb_minor = ?{})",
                    base + 2 * i + 1,
                    base + 2 * i + 2
                )
            })
            .collect();
        p.clauses.push(format!("({})", parts.join(" OR ")));
        for (sb, bb) in &filter.stakes {
            p.params.push(Value::Integer(*sb));
            p.params.push(Value::Integer(*bb));
        }
    }
    p.push_in(
        "table_size",
        filter
            .table_sizes
            .iter()
            .map(|s| Value::Integer(*s as i64))
            .collect(),
    );
    p.push_in(
        "position",
        filter
            .positions
            .iter()
            .map(|pos| Value::Text(pos.code().to_string()))
            .collect(),
    );
    p.push_in(
        "profile_id",
        filter.profiles.iter().map(|id| Value::Integer(*id)).collect(),
    );
    p
}

fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<HandRow> {
    Ok(HandRow {
        hand_id: r.get("hand_id")?,
        ts: r.get("ts_utc")?,
        sb: r.get("sb_minor")?,
        bb: r.get("bb_minor")?,
        position: r.get("position")?,
        currency: r.get("currency")?,
        net: r.get("net_result_minor")?,
        adjusted: r.get("adjusted_result_minor")?,
        luck: r.get("luck_delta_minor")?,
        rake: r.get("rake_attributed_minor")?,
        went_to_showdown: r.get("went_to_showdown")?,
        won_at_showdown: r.get("won_at_showdown")?,
        saw_flop: r.get("saw_flop")?,
        sd_winnings: r.get("sd_winnings_minor")?,
        non_sd_winnings: r.get("non_sd_winnings_minor")?,
        wwsf_opportunity: r.get("wwsf_opportunity")?,
        won_when_saw_flop: r.get("won_when_saw_flop")?,
        vpip: r.get("vpip")?,
        pfr: r.get("pfr")?,
        limp: r.get("limp")?,
        rfi: r.get("rfi")?,
        rfi_opportunity: r.get("rfi_opportunity")?,
        three_bet: r.get("three_bet")?,
        three_bet_opportunity: r.get("three_bet_opportunity")?,
        faced_three_bet: r.get("faced_three_bet")?,
        folded_to_three_bet: r.get("folded_to_three_bet")?,
        four_bet: r.get("four_bet")?,
        four_bet_opportunity: r.get("four_bet_opportunity")?,
        faced_four_bet: r.get("faced_four_bet")?,
        folded_to_four_bet: r.get("folded_to_four_bet")?,
        cold_call: r.get("cold_call")?,
        cold_call_opportunity: r.get("cold_call_opportunity")?,
        steal_attempt: r.get("steal_attempt")?,
        steal_opportunity: r.get("steal_opportunity")?,
        faced_steal: r.get("faced_steal")?,
        folded_to_steal: r.get("folded_to_steal")?,
        cbet_opportunity: r.get("cbet_opportunity")?,
        cbet: r.get("cbet")?,
        faced_cbet: r.get("faced_cbet")?,
        folded_to_cbet: r.get("folded_to_cbet")?,
        check_raise_opportunity: r.get("check_raise_opportunity")?,
        check_raise: r.get("check_raise")?,
        probe_opportunity: r.get("probe_opportunity")?,
        probe: r.get("probe")?,
        delayed_cbet_opportunity: r.get("delayed_cbet_opportunity")?,
        delayed_cbet: r.get("delayed_cbet")?,
        aggressive_actions: r.get("aggressive_actions")?,
        passive_actions: r.get("passive_actions")?,
        session: 0,
    })
}

/// Load matching rows ordered by `(ts_utc, hand_id)`.
fn load_rows(db: &Db, predicate: &Predicate) -> Result<Vec<HandRow>, StorageError> {
    let sql = format!(
        "SELECT {ROW_COLUMNS} FROM hero_hands{} ORDER BY ts_utc, hand_id",
        predicate.where_sql()
    );
    db.with_reader(|c| {
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt
            .query_map(params_from_iter(predicate.params.iter()), map_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// Load rows matching `filter`, including the session filter.
///
/// Session ids are derived at query time from the ordered matching hands: a new
/// session starts whenever `gap > gap_secs` between consecutive hands (strictly
/// greater; a gap exactly equal to the threshold does NOT split). This mirrors
/// `src/stats/mod.rs::derive_sessions`, which was not yet implemented when this
/// module landed — if that function lands with a different boundary rule, both
/// must be reconciled. Ids are the 0-based index of the session in the ordered
/// sequence for the population selected by the *other* filter dimensions, so
/// they are stable for a given filter but change when other dimensions change.
fn load_filtered(db: &Db, filter: &FilterSpec) -> Result<Vec<HandRow>, StorageError> {
    let predicate = build_predicate(filter);
    let mut rows = load_rows(db, &predicate)?;
    if !filter.sessions.is_empty() {
        let gap = db.session_gap_secs()?;
        apply_session_filter(&mut rows, gap, &filter.sessions);
    }
    Ok(rows)
}

/// Annotates `rows` (already ordered by ts) with session indices and keeps only
/// the wanted ones.
fn apply_session_filter(rows: &mut Vec<HandRow>, gap_secs: i64, wanted: &[i64]) {
    let mut session: i64 = 0;
    let mut prev_ts: Option<i64> = None;
    for r in rows.iter_mut() {
        if let Some(prev) = prev_ts {
            if r.ts - prev > gap_secs {
                session += 1;
            }
        }
        prev_ts = Some(r.ts);
        r.session = session;
    }
    let wanted: HashSet<i64> = wanted.iter().copied().collect();
    rows.retain(|r| wanted.contains(&r.session));
}

/// Human stake label, e.g. `€0.25/€0.50`.
fn stakes_label(sb: i64, bb: i64, currency_code: &str) -> String {
    let currency = Currency::from_code(currency_code).unwrap_or(Currency::Eur);
    format!(
        "{}/{}",
        crate::money::Money::new(sb, currency).format(),
        crate::money::Money::new(bb, currency).format()
    )
}

/// Group key/label for one row.
fn group_of(group: GroupBy, r: &HandRow) -> (String, String) {
    // Unrepresentable timestamps cannot come from a parsed hand; fall back to
    // the epoch so grouping stays total.
    let dt = chrono::DateTime::from_timestamp(r.ts, 0)
        .unwrap_or(chrono::DateTime::<Utc>::UNIX_EPOCH);
    match group {
        GroupBy::None => ("all".to_string(), "All".to_string()),
        GroupBy::Day => {
            let s = dt.format("%Y-%m-%d").to_string();
            (s.clone(), s)
        }
        GroupBy::Week => {
            let s = dt.format("%G-W%V").to_string();
            (s.clone(), s)
        }
        GroupBy::Month => {
            let s = dt.format("%Y-%m").to_string();
            (s.clone(), s)
        }
        GroupBy::Stake => {
            let label = stakes_label(r.sb, r.bb, &r.currency);
            (label.clone(), label)
        }
        GroupBy::Position => {
            let s = r.position.clone();
            (s.clone(), s)
        }
    }
}

#[derive(Default)]
struct Acc {
    row: AggregateRow,
    /// Running Σ bb_minor for `total_big_blinds`.
    bb_sum: f64,
}

/// Fold one hand into an accumulator. All counts are plain SUMs; see the
/// documented definitions on [`AggregateRow`].
fn fold(r: &HandRow, row: &mut AggregateRow, bb_sum: &mut f64) {
    row.hands += 1;
    row.net_result_minor += r.net;
    row.adjusted_result_minor += r.adjusted;
    row.luck_delta_minor += r.luck;
    row.rake_attributed_minor += r.rake;
    if r.bb > 0 {
        row.bb_won += r.net as f64 / r.bb as f64;
    }
    *bb_sum += r.bb as f64;
    row.showdown_hands += r.went_to_showdown;
    row.sd_winnings_minor += r.sd_winnings;
    row.non_sd_winnings_minor += r.non_sd_winnings;

    row.vpip += r.vpip;
    row.pfr += r.pfr;
    row.limp += r.limp;
    row.rfi += r.rfi;
    row.rfi_opportunity += r.rfi_opportunity;
    row.three_bet += r.three_bet;
    row.three_bet_opportunity += r.three_bet_opportunity;
    row.faced_three_bet += r.faced_three_bet;
    row.folded_to_three_bet += r.folded_to_three_bet;
    row.four_bet += r.four_bet;
    row.four_bet_opportunity += r.four_bet_opportunity;
    row.faced_four_bet += r.faced_four_bet;
    row.folded_to_four_bet += r.folded_to_four_bet;
    row.cold_call += r.cold_call;
    row.cold_call_opportunity += r.cold_call_opportunity;
    row.steal_attempt += r.steal_attempt;
    row.steal_opportunity += r.steal_opportunity;
    row.faced_steal += r.faced_steal;
    row.folded_to_steal += r.folded_to_steal;

    let mut b3 = [false; 3];
    macro_rules! fold_street_array {
        ($col:expr, $out:expr) => {{
            decode_bools($col, &mut b3);
            for (i, b) in b3.into_iter().enumerate() {
                $out[i] += b as i64;
            }
        }};
    }
    fold_street_array!(&r.cbet_opportunity, row.cbet_opportunity);
    fold_street_array!(&r.cbet, row.cbet);
    fold_street_array!(&r.faced_cbet, row.faced_cbet);
    fold_street_array!(&r.folded_to_cbet, row.folded_to_cbet);
    fold_street_array!(&r.check_raise_opportunity, row.check_raise_opportunity);
    fold_street_array!(&r.check_raise, row.check_raise);
    fold_street_array!(&r.probe_opportunity, row.probe_opportunity);
    fold_street_array!(&r.probe, row.probe);
    fold_street_array!(&r.delayed_cbet_opportunity, row.delayed_cbet_opportunity);
    fold_street_array!(&r.delayed_cbet, row.delayed_cbet);

    let mut u32_4 = [0u32; 4];
    decode_u32s(&r.aggressive_actions, &mut u32_4);
    for (i, n) in u32_4.into_iter().enumerate() {
        row.aggressive_actions[i] += n as i64;
    }
    decode_u32s(&r.passive_actions, &mut u32_4);
    for (i, n) in u32_4.into_iter().enumerate() {
        row.passive_actions[i] += n as i64;
    }

    row.wwsf_opportunity += r.wwsf_opportunity;
    row.won_when_saw_flop += r.won_when_saw_flop;
    row.wtsd_opportunity += r.saw_flop;
    row.wtsd += r.went_to_showdown;
    row.won_at_showdown += r.won_at_showdown;
}

fn finalize(acc: &mut Acc) -> AggregateRow {
    let mut row = std::mem::take(&mut acc.row);
    row.total_big_blinds = acc.bb_sum / 100.0;
    if row.hands > 0 {
        row.bb_per_100 = row.bb_won / row.hands as f64 * 100.0;
        // €/100 per docs/stat-definitions.md: Σ net_result / hands × 100,
        // stored in MINOR units (divide by the currency exponent at display
        // time): net_result_minor / hands * 100, i.e. `Σ net / hands` in
        // major units is the same ratio unscaled.
        row.currency_per_100 = row.net_result_minor as f64 / row.hands as f64 * 100.0;
    }
    row
}

/// Aggregate hero hands matching `filter`, grouped by `group`.
pub fn aggregate(
    db: &Db,
    filter: &FilterSpec,
    group: GroupBy,
) -> Result<Vec<AggregateGroup>, StorageError> {
    let rows = load_filtered(db, filter)?;

    let mut accs: BTreeMap<String, (String, Acc)> = BTreeMap::new();
    let mut stake_meta: HashMap<String, (i64, i64)> = HashMap::new();
    for r in &rows {
        let (key, label) = group_of(group, r);
        if group == GroupBy::Stake {
            stake_meta.insert(key.clone(), (r.sb, r.bb));
        }
        let acc = accs
            .entry(key)
            .or_insert_with(|| (label, Acc::default()));
        fold(r, &mut acc.1.row, &mut acc.1.bb_sum);
    }

    let mut groups: Vec<AggregateGroup> = accs
        .into_iter()
        .map(|(key, (label, mut acc))| AggregateGroup {
            key,
            label,
            row: {
                finalize(&mut acc)
            },
        })
        .collect();

    match group {
        GroupBy::Stake => {
            groups.sort_by_key(|g| stake_meta.get(&g.key).copied().unwrap_or((0, 0)));
        }
        GroupBy::Position => {
            let ordinal = |code: &str| -> i64 {
                Position::from_code(code)
                    .map(|p| Position::ALL.iter().position(|x| *x == p).unwrap_or(255) as i64)
                    .unwrap_or(255)
            };
            groups.sort_by_key(|g| ordinal(&g.key));
        }
        // BTreeMap already yields chronological (ISO-sortable) and
        // lexicographic ordering for the rest.
        _ => {}
    }
    Ok(groups)
}

/// Cumulative net/adjusted series over hands ordered by `(ts_utc, hand_id)`,
/// downsampled server-side with largest-triangle-three-buckets to at most
/// `max_points`, always keeping the first and last cumulative values.
pub fn cumulative_series(
    db: &Db,
    filter: &FilterSpec,
    max_points: usize,
) -> Result<Vec<SeriesPoint>, StorageError> {
    let rows = load_filtered(db, filter)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    // Full cumulative series: one point per hand boundary.
    let mut t: Vec<i64> = Vec::with_capacity(rows.len());
    let mut hands: Vec<i64> = Vec::with_capacity(rows.len());
    let mut net: Vec<i64> = Vec::with_capacity(rows.len());
    let mut adjusted: Vec<i64> = Vec::with_capacity(rows.len());
    let mut c_net: i64 = 0;
    let mut c_adj: i64 = 0;
    for (i, r) in rows.iter().enumerate() {
        c_net += r.net;
        c_adj += r.adjusted;
        t.push(r.ts);
        hands.push(i as i64 + 1);
        net.push(c_net);
        adjusted.push(c_adj);
    }

    let n = rows.len();
    let picked: Vec<usize> = if n <= max_points {
        (0..n).collect()
    } else if max_points < 3 {
        // Fewer than three slots cannot do better than the endpoints.
        vec![0, n - 1]
    } else {
        lttb(&t, &net, max_points)
    };

    Ok(picked
        .into_iter()
        .map(|i| SeriesPoint {
            t: t[i],
            hands: hands[i],
            net_minor: net[i],
            adjusted_minor: adjusted[i],
        })
        .collect())
}

/// Largest-triangle-three-buckets over `(xs, ys)` with `threshold >= 3` and
/// `xs.len() > threshold`. Index 0 and the last index are always selected.
fn lttb(xs: &[i64], ys: &[i64], threshold: usize) -> Vec<usize> {
    let n = xs.len();
    debug_assert!(n > threshold && threshold >= 3);
    let mut picked = Vec::with_capacity(threshold);
    picked.push(0usize);

    let buckets = threshold - 2;
    let every = (n - 2) as f64 / buckets as f64;
    let mut a = 0usize; // index of previously selected point

    for b in 0..buckets {
        // Bucket b holds the candidates for the next selected point:
        // [floor(b*every)+1, floor((b+1)*every)+1). every > 1 because
        // n-2 > buckets, so every bucket is non-empty even when the last
        // one is clipped to `n - 1`.
        let start = (b as f64 * every) as usize + 1;
        let end = (((b + 1) as f64 * every) as usize + 1).min(n - 1);
        // The bucket after it, averaged, forms the third triangle vertex.
        let avg_start = end;
        let avg_end = (((b + 2) as f64 * every) as usize + 1).min(n - 1);
        let (avg_x, avg_y) = if avg_start < avg_end {
            let cnt = (avg_end - avg_start) as f64;
            let (sx, sy) =
                xs[avg_start..avg_end].iter().zip(&ys[avg_start..avg_end]).fold(
                    (0.0f64, 0.0f64),
                    |(x, y), (xi, yi)| (x + *xi as f64, y + *yi as f64),
                );
            (sx / cnt, sy / cnt)
        } else {
            (xs[n - 1] as f64, ys[n - 1] as f64)
        };

        let ax = xs[a] as f64;
        let ay = ys[a] as f64;
        let mut best = start;
        let mut best_area = f64::MIN;
        for i in start..end {
            let area =
                ((ax - avg_x) * (ys[i] as f64 - ay) - (ax - xs[i] as f64) * (avg_y - ay))
                    .abs();
            if area > best_area {
                best_area = area;
                best = i;
            }
        }
        if best < n {
            picked.push(best);
            a = best;
        }
    }
    picked.push(n - 1);
    picked
}

pub fn hand_list(
    db: &Db,
    filter: &FilterSpec,
    limit: i64,
    offset: i64,
) -> Result<Vec<HandListRow>, StorageError> {
    let predicate = build_predicate(filter);
    // With a session filter the population is only known in Rust (session ids
    // are derived, not stored), so the allowed hand ids are injected as an
    // extra `IN (...)` clause before paginating in SQL.
    let sql;
    let mut params: Vec<Value> = predicate.params.clone();
    if filter.sessions.is_empty() {
        sql = format!(
            "SELECT hand_id, ts_utc, sb_minor, bb_minor, table_size, position, \
             net_result_minor, currency FROM hero_hands{} \
             ORDER BY ts_utc DESC, hand_id ASC LIMIT ?{} OFFSET ?{}",
            predicate.where_sql(),
            predicate.params.len() + 1,
            predicate.params.len() + 2
        );
        params.push(Value::Integer(limit));
        params.push(Value::Integer(offset));
    } else {
        let gap = db.session_gap_secs()?;
        let mut rows = load_rows(db, &predicate)?;
        apply_session_filter(&mut rows, gap, &filter.sessions);
        let ids: Vec<Value> =
            rows.into_iter().map(|r| Value::Text(r.hand_id)).collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut p2 = predicate;
        p2.and_in("hand_id", ids);
        sql = format!(
            "SELECT hand_id, ts_utc, sb_minor, bb_minor, table_size, position, \
             net_result_minor, currency FROM hero_hands{} \
             ORDER BY ts_utc DESC, hand_id ASC LIMIT ?{} OFFSET ?{}",
            p2.where_sql(),
            p2.params.len() + 1,
            p2.params.len() + 2
        );
        params = p2.params;
        params.push(Value::Integer(limit));
        params.push(Value::Integer(offset));
    }

    db.with_reader(|c| {
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt
            .query_map(params_from_iter(params.iter()), |r| {
                Ok(HandListRow {
                    hand_id: r.get(0)?,
                    ts_utc: r.get(1)?,
                    stakes: stakes_label(r.get::<_, i64>(2)?, r.get::<_, i64>(3)?, &r.get::<_, String>(7)?),
                    table_size: r.get::<_, i64>(4)? as u8,
                    position: r.get(5)?,
                    net_result_minor: r.get(6)?,
                    currency: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

pub fn hand_count(db: &Db, filter: &FilterSpec) -> Result<i64, StorageError> {
    if filter.sessions.is_empty() {
        let predicate = build_predicate(filter);
        let sql = format!("SELECT COUNT(*) FROM hero_hands{}", predicate.where_sql());
        Ok(db.with_reader(|c| c.query_row(&sql, params_from_iter(predicate.params.iter()), |r| r.get(0)))?)
    } else {
        let rows = load_filtered(db, filter)?;
        Ok(rows.len() as i64)
    }
}

/// Distinct stake options available, for the filter bar.
pub fn stake_options(db: &Db) -> Result<Vec<(i64, i64, String)>, StorageError> {
    db.with_reader(|c| {
        let mut stmt = c.prepare(
            "SELECT DISTINCT sb_minor, bb_minor, currency FROM hero_hands \
             ORDER BY sb_minor, bb_minor",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let sb: i64 = r.get(0)?;
                let bb: i64 = r.get(1)?;
                let currency: String = r.get(2)?;
                Ok((sb, bb, stakes_label(sb, bb, &currency)))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}
