//! Route handlers + view models. Every screen is server-rendered; htmx only
//! swaps the `#content` fragment for filter changes, and every screen renders
//! identically on a full page load (no JavaScript required).

use crate::domain::{
    ActionKind, Currency, FilterSpec, GroupBy, HeroProfile, Position, Session, Street,
};
use crate::money::Money;
use crate::storage::Db;
use crate::parser::{self, HandParser};
use crate::storage::hands as hands_store;
use rusqlite::OptionalExtension;
use crate::storage::queries as q;
use crate::web::defs;
use crate::web::filters::{group_param, FilterView};
use crate::web::render::env;
use axum::extract::{Path, RawQuery, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Serialize;
use std::collections::HashMap;

use super::AppState;

/// Largest number of points shipped for the cumulative chart. Server-side
/// downsampling guarantees a bounded payload at any history size.
const CHART_MAX_POINTS: usize = 1000;
/// Hands listed per page on the hand browser.
const HANDS_PER_PAGE: i64 = 50;
/// Quarantine rows shown on the import screen.
const QUARANTINE_PAGE: i64 = 100;

pub type WebResult<T> = Result<T, WebError>;
pub struct WebError(pub StatusCode, pub String);

impl From<(StatusCode, String)> for WebError {
    fn from(v: (StatusCode, String)) -> Self {
        WebError(v.0, v.1)
    }
}

impl From<crate::error::StorageError> for WebError {
    fn from(e: crate::error::StorageError) -> Self {
        WebError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

fn internal(err: impl std::fmt::Display) -> WebError {
    WebError(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

fn not_found(what: &str) -> WebError {
    WebError(StatusCode::NOT_FOUND, format!("{what} not found"))
}

fn screen_value(name: &str, obj: impl Serialize, fragment: bool) -> Response {
    let env = env();
    let value = minijinja::Value::from_serialize(&obj);
    let template = if fragment { name } else { "layout.html" };
    match env.get_template(template).and_then(|t| t.render(value)) {
        Ok(html) => Html(html).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("template error: {e}")).into_response(),
    }
}

/// htmx requests swap only the content fragment.
fn wants_fragment(headers: &HeaderMap) -> bool {
    headers
        .get("HX-Request")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn base_json<'a>(
    title: &str,
    path: &str,
    active: &'static str,
    screen: &'static str,
    bar: &FilterBar,
) -> serde_json::Value {
    serde_json::json!({
        "title": title,
        "path": path,
        "active": active,
        "screen": screen,
        "version": crate::VERSION,
        "filter": serde_json::to_value(bar).unwrap_or_default(),
    })
}

// ---------------------------------------------------------------------------
// View models
// ---------------------------------------------------------------------------

#[derive(Serialize, Default)]
struct OptionItem {
    value: String,
    label: String,
    selected: bool,
}

#[derive(Serialize)]
struct ChipItem {
    label: String,
    #[serde(rename = "removeUrl")]
    remove_url: String,
}

/// Everything the shared filter bar partial needs.
#[derive(Serialize, Default)]
struct FilterBar {
    from: String,
    to: String,
    stakes: Vec<String>,
    sizes: Vec<String>,
    positions: Vec<String>,
    sessions: Vec<String>,
    profiles: Vec<String>,
    active: bool,
    chips: Vec<ChipItem>,
    #[serde(rename = "stakeOptions")]
    stake_options: Vec<OptionItem>,
    #[serde(rename = "sizeOptions")]
    size_options: Vec<OptionItem>,
    #[serde(rename = "positionOptions")]
    position_options: Vec<OptionItem>,
    #[serde(rename = "profileOptions")]
    profile_options: Vec<OptionItem>,
    path: String,
}

impl FilterBar {
    fn build(st: &AppState, fv: &FilterView, path: &str, show_profiles: bool) -> FilterBar {
        let selected = |v: &str, list: &[String]| list.iter().any(|s| s == v);
        let stake_options = queries_safe_stakes(&st.db)
            .into_iter()
            .map(|(sb, _bb, label)| OptionItem {
                value: format!("{sb}-{_bb}"),
                label,
                selected: selected(&format!("{sb}-{_bb}"), &fv.stakes),
            })
            .collect();
        let position_options = Position::ALL
            .iter()
            .map(|p| OptionItem {
                value: p.code().to_string(),
                label: p.code().to_string(),
                selected: selected(p.code(), &fv.positions),
            })
            .collect();
        let size_options = [6u8, 9]
            .iter()
            .map(|n| OptionItem {
                value: n.to_string(),
                label: format!("{n}-max"),
                selected: selected(&n.to_string(), &fv.sizes),
            })
            .collect();
        let profile_options = if show_profiles {
            st.db
                .list_profiles()
                .unwrap_or_default()
                .into_iter()
                .map(|p| OptionItem {
                    value: p.id.to_string(),
                    label: p.name,
                    selected: selected(&p.id.to_string(), &fv.profiles),
                })
                .collect()
        } else {
            Vec::new()
        };
        let chips = fv
            .chips()
            .into_iter()
            .map(|c| ChipItem {
                label: c.label,
                remove_url: format!("{}{}", path, c.remove_url),
            })
            .collect();
        FilterBar {
            from: fv.from.clone().unwrap_or_default(),
            to: fv.to.clone().unwrap_or_default(),
            stakes: fv.stakes.clone(),
            sizes: fv.sizes.clone(),
            positions: fv.positions.clone(),
            sessions: fv.sessions.clone(),
            profiles: fv.profiles.clone(),
            active: !fv.is_empty(),
            chips,
            stake_options,
            size_options,
            position_options,
            profile_options,
            path: path.to_string(),
        }
    }
}

/// Stake options are read with `stake_options`; failures produce an empty list
/// rather than a 500 so the filter bar always renders.
fn queries_safe_stakes(db: &Db) -> Vec<(i64, i64, String)> {
    q::stake_options(db).unwrap_or_default()
}

// (SummaryView below supersedes the placeholder above; remove both.)
#[derive(Serialize)]
struct Summary {
    has_data: bool,
    hands: i64,
    net: String,
    positive: bool,
    bb_per_100: String,
    currency_per_100: String,
    adjusted: String,
    luck: String,
    luck_kind: &'static str,
    showdown: String,
    non_showdown: String,
    rake: String,
    rake_bb: Option<String>,
}

#[derive(Serialize)]
struct ChartPayload {
    x: Vec<f64>,
    net: Vec<f64>,
    adjusted: Vec<f64>,
}

#[derive(Serialize)]
struct StatRow {
    name: String,
    value: String,
    counts: String,
    definition: String,
}

#[derive(Serialize)]
struct StatGroup {
    label: String,
    rows: Vec<StatRow>,
}

#[derive(Serialize)]
struct PositionRow {
    label: String,
    hands: i64,
    net: String,
    positive: bool,
    bb_per_100: String,
    vpip: String,
    pfr: String,
    wtsd: String,
}

#[derive(Serialize)]
struct ColumnHeader {
    label: &'static str,
    definition: String,
}

#[derive(Serialize)]
struct SessionRow {
    id: i64,
    start: String,
    end: String,
    /// Approximate: derived from first/last hand timestamps (story 70/71).
    duration: String,
    hands: i64,
    net: String,
    positive: bool,
}

#[derive(Serialize)]
struct HandListRowView {
    hand_id: String,
    ts: String,
    stakes: String,
    table_size: u8,
    position: String,
    net: String,
    positive: bool,
    url: String,
}

#[derive(Serialize)]
struct PageLinks {
    prev: Option<String>,
    next: Option<String>,
    page: usize,
    pages: i64,
    offset: String,
    total: i64,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Total row for the dashboard, or the empty row when nothing matches.
fn summary_from(row: q::AggregateRow, stake_bb: Option<f64>) -> Summary {
    let luck_positive = row.luck_delta_minor > 0;
    let luck_zero = row.luck_delta_minor == 0;
    Summary {
        has_data: row.hands > 0,
        hands: row.hands,
        net: money_of(row.net_result_minor),
        positive: row.net_result_minor >= 0,
        bb_per_100: format!("{:.2}", row.bb_per_100),
        // `currency_per_100` is minor units; the exponent division happens at
        // display time inside `Money::format` via the `per100_money` filter's
        // convention — here formatted directly from minor units.
        currency_per_100: Money::new(row.currency_per_100.round() as i64, Currency::Eur)
            .format(),
        adjusted: money_of(row.adjusted_result_minor),
        luck: {
            let m = Money::new(row.luck_delta_minor.abs(), Currency::Eur).format();
            if luck_zero {
                format!("±{m}")
            } else if luck_positive {
                format!("+{m}")
            } else {
                format!("-{m}")
            }
        },
        luck_kind: if luck_zero {
            "even"
        } else if luck_positive {
            "good"
        } else {
            "bad"
        },
        showdown: money_of(row.sd_winnings_minor),
        non_showdown: money_of(row.non_sd_winnings_minor),
        rake: money_of(row.rake_attributed_minor),
        // Rake in big blinds is Σ rake_i / bb_i; `AggregateRow` does not carry
        // the per-hand normalization, so it is rebuilt from the stake-grouped
        // aggregate (labels match `stake_options`) with per-stake big blinds.
        rake_bb: stake_bb.map(|v| format!("{v:.1} BB")),
    }
}

fn money_of(minor: i64) -> String {
    Money::new(minor, Currency::Eur).format()
}

fn stake_group_rake_bb(db: &Db, spec: &FilterSpec) -> Option<f64> {
    let groups = q::aggregate(db, spec, GroupBy::Stake).ok()?;
    if groups.is_empty() {
        return None;
    }
    let options = q::stake_options(db).ok()?;
    let bb_of: HashMap<String, i64> =
        options.into_iter().map(|(sb, bb, label)| (label, bb.min(sb).max(bb))).collect();
    let mut sum = 0.0;
    let mut any = false;
    for g in &groups {
        if let Some(bb) = bb_of.get(g.key.as_str()) {
            if *bb > 0 {
                // Rake in BB: per-hand rake normalized by the hand's own BB.
                // Within one stake group every hand shares the blind level, so
                // Σ rake_minor / bb is exact for the group.
                sum += g.row.rake_attributed_minor as f64 / *bb as f64;
                any = true;
            }
        }
    }
    any.then_some(sum)
}

fn stat(name: &str, num: i64, den: i64, dict_key: &str) -> StatRow {
    StatRow {
        name: name.to_string(),
        value: crate::web::render::pct_str(num, den),
        counts: format!("{num} / {den}"),
        definition: defs::definition(dict_key),
    }
}


#[allow(dead_code)]
fn unused(_: ()) {}

// ---------------------------------------------------------------------------
// GET / — dashboard
// ---------------------------------------------------------------------------

pub async fn dashboard(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> WebResult<Response> {
    let fv = FilterView::parse(raw.as_deref());
    let bar = FilterBar::build(&st, &fv, "/", false);
    let row = q::aggregate(&st.db, &fv.spec, GroupBy::None)?
        .into_iter()
        .next()
        .map(|g| g.row)
        .unwrap_or_default();
    let summary = summary_from(row, stake_group_rake_bb(&st.db, &fv.spec));
    let series = q::cumulative_series(&st.db, &fv.spec, CHART_MAX_POINTS)?;
    let chart = ChartPayload {
        x: series.iter().map(|p| p.t as f64).collect(),
        // Minor units here; app.js divides by the currency exponent for
        // display only. No money math happens in JavaScript.
        net: series.iter().map(|p| p.net_minor as f64).collect(),
        adjusted: series.iter().map(|p| p.adjusted_minor as f64).collect(),
    };
    let chart_json =
        serde_json::to_string(&chart).map_err(|e| internal(format!("chart json: {e}")))?;

    let mut obj = base_json("Dashboard", "/", "dashboard", "dashboard.html", &bar);
    let obj = obj
        .as_object_mut()
        .ok_or_else(|| internal("base object"))?;
    obj.insert(
        "summary".into(),
        serde_json::to_value(&summary).map_err(|e| internal(e))?,
    );
    obj.insert("chartJson".into(), serde_json::Value::String(chart_json));
    obj.insert("hasChart".into(), serde_json::Value::Bool(!chart.x.is_empty()));

    Ok(screen_value("dashboard.html", obj.clone(), wants_fragment(&headers)))
}

// ---------------------------------------------------------------------------
// GET /preflop, /postflop, /position — statistic tables
// ---------------------------------------------------------------------------

fn preflop_rows(row: &q::AggregateRow) -> Vec<StatRow> {
    let h = row.hands;
    vec![
        stat("VPIP", row.vpip, h, "VPIP (voluntarily put chips in pot)"),
        stat("PFR", row.pfr, h, "PFR (preflop raise)"),
        // Limp % is defined as the VPIP − PFR gap over the same denominator.
        stat("Limp % (VPIP−PFR)", row.vpip - row.pfr, h, "Limp % (VPIP−PFR gap)"),
        stat("Raise first in", row.rfi, row.rfi_opportunity, "Raise First In (RFI / open-raise rate)"),
        stat("3-bet", row.three_bet, row.three_bet_opportunity, "3-Bet"),
        stat("Fold to 3-bet", row.folded_to_three_bet, row.faced_three_bet, "Fold to 3-Bet"),
        stat("4-bet", row.four_bet, row.four_bet_opportunity, "4-Bet"),
        stat("Fold to 4-bet", row.folded_to_four_bet, row.faced_four_bet, "Fold to 4-Bet"),
        stat("Cold call", row.cold_call, row.cold_call_opportunity, "Cold call"),
        stat("Steal attempt", row.steal_attempt, row.steal_opportunity, "Steal attempt"),
        stat("Fold to steal", row.folded_to_steal, row.faced_steal, "Fold to Steal / Blind defense"),
    ]
}

fn af_value(aggressive: i64, passive: i64) -> String {
    if passive == 0 && aggressive > 0 {
        "∞".to_string()
    } else if passive == 0 {
        "—".to_string()
    } else {
        format!("{:.2}", aggressive as f64 / passive as f64)
    }
}

fn agg_value(aggressive: i64, passive: i64) -> String {
    let total = aggressive + passive;
    if total == 0 {
        "—".to_string()
    } else {
        crate::web::render::pct_str(aggressive, total)
    }
}

fn postflop_rows(row: &q::AggregateRow) -> Vec<StatRow> {
    let streets = ["Flop", "Turn", "River"];
    let mut rows = Vec::new();
    for (i, s) in streets.iter().enumerate() {
        rows.push(stat(
            &format!("C-bet {s}"),
            row.cbet[i],
            row.cbet_opportunity[i],
            match i {
                0 => "C-Bet Flop",
                1 => "C-Bet Turn",
                _ => "C-Bet River",
            },
        ));
    }
    for (i, s) in streets.iter().enumerate() {
        rows.push(stat(
            &format!("Fold to c-bet {s}"),
            row.folded_to_cbet[i],
            row.faced_cbet[i],
            "Fold to C-Bet (by street)",
        ));
    }
    for (i, s) in streets.iter().enumerate() {
        rows.push(stat(
            &format!("Check-raise {s}"),
            row.check_raise[i],
            row.check_raise_opportunity[i],
            "Check-Raise (by street)",
        ));
    }
    for (s, i) in [("Turn", 1usize), ("River", 2usize)] {
        rows.push(stat(
            &format!("Probe bet {s}"),
            row.probe[i],
            row.probe_opportunity[i],
            "Probe Bet",
        ));
        rows.push(stat(
            &format!("Delayed c-bet {s}"),
            row.delayed_cbet[i],
            row.delayed_cbet_opportunity[i],
            "Delayed C-Bet",
        ));
    }
    rows.push(stat("WWSF", row.won_when_saw_flop, row.wwsf_opportunity, "WWSF (Won When Saw Flop)"));
    rows.push(stat("WTSD", row.wtsd, row.wtsd_opportunity, "WTSD (Went To Showdown)"));
    rows.push(stat("W$SD", row.won_at_showdown, row.showdown_hands, "W$SD (Won At Showdown)"));

    // Aggression: per street plus the overall flop+turn+river sums (preflop is
    // excluded from the overall figures per the dictionary, A10).
    let names = ["flop", "turn", "river"];
    for (i, s) in names.iter().enumerate() {
        let (n, d) = (row.aggressive_actions[i + 1], row.passive_actions[i + 1]);
        rows.push(StatRow {
            name: format!("Aggression Factor ({s})"),
            value: af_value(n, d),
            counts: format!("{n} / {d}"),
            definition: defs::definition("Aggression Factor (AF)"),
        });
        rows.push(StatRow {
            name: format!("Aggression frequency ({s})"),
            value: agg_value(n, d),
            counts: format!("{n} / {d}"),
            definition: defs::definition("Aggression Frequency (Agg%)"),
        });
    }
    let overall_agg: i64 = row.aggressive_actions[1..].iter().sum();
    let overall_passive: i64 = row.passive_actions[1..].iter().sum();
    rows.push(StatRow {
        name: "Aggression Factor (overall)".into(),
        value: af_value(overall_agg, overall_passive),
        counts: format!("{overall_agg} / {overall_passive}"),
        definition: defs::definition("Aggression Factor (AF)"),
    });
    rows.push(StatRow {
        name: "Aggression frequency (overall)".into(),
        value: agg_value(overall_agg, overall_passive),
        counts: format!("{overall_agg} / {overall_passive}"),
        definition: defs::definition("Aggression Frequency (Agg%)"),
    });
    rows
}

/// One aggregate-group table for preflop/postflop screens.
fn stat_groups(
    db: &Db,
    spec: &FilterSpec,
    group: GroupBy,
    rows_of: impl Fn(&q::AggregateRow) -> Vec<StatRow>,
) -> WebResult<Vec<StatGroup>> {
    let groups = q::aggregate(db, spec, group)?;
    let out: Vec<StatGroup> = groups
        .iter()
        .map(|g| StatGroup { label: g.label.clone(), rows: rows_of(&g.row) })
        .collect();
    Ok(out)
}


// `stat_screen` needs the htmx header, which only the real handlers see; the
// `_dummy` trick below is deliberately replaced by the thin per-route handlers.

pub async fn preflop(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> WebResult<Response> {
    let fv = FilterView::parse(raw.as_deref());
    let bar = FilterBar::build(&st, &fv, "/preflop", true);
    let groups = stat_groups(&st.db, &fv.spec, group_param(&fv), preflop_rows)
        .map_err(WebError::from)?;
    let mut obj = base_json("Preflop statistics", "/preflop", "preflop", "preflop.html", &bar);
    let obj = obj.as_object_mut().ok_or_else(|| internal("base object"))?;
    obj.insert(
        "groups".into(),
        serde_json::to_value(&groups).map_err(|e| internal(e))?,
    );
    obj.insert(
        "groupName".into(),
        serde_json::Value::String(fv.group.clone().unwrap_or_else(|| "none".into())),
    );
    Ok(screen_value("preflop.html", obj.clone(), wants_fragment(&headers)))
}

pub async fn postflop(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> WebResult<Response> {
    let fv = FilterView::parse(raw.as_deref());
    let bar = FilterBar::build(&st, &fv, "/postflop", true);
    let groups = stat_groups(&st.db, &fv.spec, group_param(&fv), postflop_rows)
        .map_err(WebError::from)?;
    let mut obj = base_json("Postflop statistics", "/postflop", "postflop", "postflop.html", &bar);
    let obj = obj.as_object_mut().ok_or_else(|| internal("base object"))?;
    obj.insert(
        "groups".into(),
        serde_json::to_value(&groups).map_err(|e| internal(e))?,
    );
    obj.insert(
        "groupName".into(),
        serde_json::Value::String(fv.group.clone().unwrap_or_else(|| "none".into())),
    );
    Ok(screen_value("postflop.html", obj.clone(), wants_fragment(&headers)))
}

pub async fn position(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> WebResult<Response> {
    let fv = FilterView::parse(raw.as_deref());
    let bar = FilterBar::build(&st, &fv, "/position", true);
    let groups = q::aggregate(&st.db, &fv.spec, GroupBy::Position)?;
    let columns = [
        ColumnHeader { label: "Hands", definition: defs::definition("Hands played") },
        ColumnHeader { label: "Net", definition: defs::definition("Net result (winnings)") },
        ColumnHeader { label: "BB/100", definition: defs::definition("BB/100") },
        ColumnHeader { label: "VPIP", definition: defs::definition("VPIP (voluntarily put chips in pot)") },
        ColumnHeader { label: "PFR", definition: defs::definition("PFR (preflop raise)") },
        ColumnHeader { label: "WTSD", definition: defs::definition("WTSD (Went To Showdown)") },
    ];
    let rows: Vec<PositionRow> = groups
        .iter()
        .map(|g| PositionRow {
            label: g.label.clone(),
            hands: g.row.hands,
            net: money_of(g.row.net_result_minor),
            positive: g.row.net_result_minor >= 0,
            bb_per_100: format!("{:.2}", g.row.bb_per_100),
            vpip: crate::web::render::pct_str(g.row.vpip, g.row.hands),
            pfr: crate::web::render::pct_str(g.row.pfr, g.row.hands),
            wtsd: crate::web::render::pct_str(g.row.wtsd, g.row.wtsd_opportunity),
        })
        .collect();
    let mut obj = base_json("Position report", "/position", "position", "position.html", &bar);
    let obj = obj.as_object_mut().ok_or_else(|| internal("base object"))?;
    obj.insert(
        "columns".into(),
        serde_json::to_value(&columns).map_err(|e| internal(e))?,
    );
    obj.insert(
        "rows".into(),
        serde_json::to_value(&rows).map_err(|e| internal(e))?,
    );
    Ok(screen_value("position.html", obj.clone(), wants_fragment(&headers)))
}

// ---------------------------------------------------------------------------
// GET /sessions
// ---------------------------------------------------------------------------

/// `<stats::sessions_for>` is the one dependency that may still be landing.
/// A defensive `catch_unwind` keeps THIS screen alive (with a visible notice)
/// instead of taking the whole session down while the stats module lands; the
/// session gap logic itself is never reimplemented here.
fn sessions_report(db: &Db, filter: &FilterSpec, gap: i64) -> (Vec<Session>, Option<String>) {
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::stats::sessions_for(db, filter, gap)
    }));
    match r {
        Ok(sessions) => (sessions, None),
        Err(_) => (
            Vec::new(),
            Some("Session computation is not available yet (stats::sessions_for is still being implemented).".into()),
        ),
    }
}

pub async fn sessions(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> WebResult<Response> {
    let fv = FilterView::parse(raw.as_deref());
    let bar = FilterBar::build(&st, &fv, "/sessions", true);
    let gap = st.db.session_gap_secs()?;
    let (sessions, notice) = sessions_report(&st.db, &fv.spec, gap);
    let rows: Vec<SessionRow> = sessions
        .iter()
        .map(|s: &Session| {
            let duration = (s.ended_at.timestamp() - s.started_at.timestamp()).max(0);
            SessionRow {
                id: s.id,
                start: s.started_at.format("%Y-%m-%d %H:%M UTC").to_string(),
                end: s.ended_at.format("%Y-%m-%d %H:%M UTC").to_string(),
                // Duration is first-hand→last-hand wall time; always labelled
                // approximate (story 71).
                duration: format!("≈ {}", crate::web::render::dur_str(duration)),
                hands: s.hands,
                net: money_of(s.net_result),
                positive: s.net_result >= 0,
            }
        })
        .collect();

    let mut obj = base_json("Sessions", "/sessions", "sessions", "sessions.html", &bar);
    let obj = obj.as_object_mut().ok_or_else(|| internal("base object"))?;
    obj.insert(
        "rows".into(),
        serde_json::to_value(&rows).map_err(|e| internal(e))?,
    );
    obj.insert(
        "notice".into(),
        notice
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    );
    obj.insert(
        "gapMinutes".into(),
        serde_json::Value::from(gap / 60),
    );
    Ok(screen_value("sessions.html", obj.clone(), wants_fragment(&headers)))
}

// ---------------------------------------------------------------------------
// GET /hands and GET /hands/:id
// ---------------------------------------------------------------------------

fn page_of(raw: Option<&str>) -> usize {
    raw.and_then(|q| {
        q.split('&').find_map(|p| p.strip_prefix("page=")?.parse::<usize>().ok())
    })
    .unwrap_or(1)
    .max(1)
}

pub async fn hands(
    State(st): State<AppState>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> WebResult<Response> {
    let fv = FilterView::parse(raw.as_deref());
    let page = page_of(raw.as_deref());
    let total = q::hand_count(&st.db, &fv.spec)?;
    let pages = ((total + HANDS_PER_PAGE - 1) / HANDS_PER_PAGE).max(1);
    let offset = (page as i64 - 1) * HANDS_PER_PAGE;
    let rows = q::hand_list(&st.db, &fv.spec, HANDS_PER_PAGE, offset)?;
    let views: Vec<HandListRowView> = rows
        .iter()
        .map(|r: &q::HandListRow| HandListRowView {
            hand_id: r.hand_id.clone(),
            ts: crate::web::render::dt_str(r.ts_utc),
            stakes: r.stakes.clone(),
            table_size: r.table_size,
            position: r.position.clone(),
            net: money_of(r.net_result_minor),
            positive: r.net_result_minor >= 0,
            url: format!("/hands/{}", urlencode(&r.hand_id)),
        })
        .collect();

    let urls = |page: usize| {
        let q = if fv.query().is_empty() {
            format!("page={page}")
        } else {
            format!("{}&page={page}", fv.query())
        };
        format!("{path}?{q}", path = "/hands")
    };
    let prev = (page > 1).then(|| urls(page - 1));
    let next = ((page as i64) < pages).then(|| urls(page + 1));
    let links = PageLinks {
        prev,
        next,
        page,
        pages,
        offset: offset.to_string(),
        total,
    };

    let bar = FilterBar::build(&st, &fv, "/hands", true);
    let mut obj = base_json("Hand browser", "/hands", "hands", "hands.html", &bar);
    let obj = obj.as_object_mut().ok_or_else(|| internal("base object"))?;
    obj.insert(
        "rows".into(),
        serde_json::to_value(&views).map_err(|e| internal(e))?,
    );
    obj.insert(
        "pager".into(),
        serde_json::to_value(&links).map_err(|e| internal(e))?,
    );
    obj.insert(
        "csvUrl".into(),
        serde_json::Value::String(fv.url("/export/hands.csv")),
    );
    obj.insert(
        "showingFrom".into(),
        serde_json::Value::from(if total == 0 { 0i64 } else { offset + 1 }),
    );
    obj.insert(
        "showingTo".into(),
        serde_json::Value::from(offset + views.len() as i64),
    );
    Ok(screen_value("hands.html", obj.clone(), wants_fragment(&headers)))
}

/// Percent-encode a query parameter value. Only the characters that can appear
/// in the filter values (screen names, stake labels, dates) matter here.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[derive(Serialize)]
struct SeatView {
    seat: u8,
    name: String,
    stack: String,
    position: String,
    contributed: String,
    result: String,
    positive: bool,
    cards: String,
}

#[derive(Serialize)]
struct ActionView {
    street: &'static str,
    who: String,
    kind: String,
    amount: String,
}

#[derive(Serialize)]
struct HandDetail {
    hand_id: String,
    table_name: String,
    stakes: String,
    table_size: u8,
    ts: String,
    local_ts: String,
    tz_token: String,
    board: String,
    pot: String,
    rake: String,
    went_to_showdown: bool,
    parser_version: i64,
    seats: Vec<SeatView>,
    actions: Vec<ActionView>,
    #[serde(rename = "actionsAvailable")]
    actions_available: bool,
    #[serde(rename = "actionsNote")]
    actions_note: Option<String>,
    raw: String,
}

pub async fn hand_detail(
    State(st): State<AppState>,
    _headers: HeaderMap,
    Path(hand_id): Path<String>,
) -> WebResult<Response> {
    // The parsed header and seats are read from the stored rows; the full
    // action list is not persisted (raw text makes it recoverable), so the
    // hand detail view re-parses the retained raw text for display only.
    db_in_session(&st, &hand_id)
}

fn db_in_session(state: &AppState, hand_id: &str) -> WebResult<Response> {
    // This function is split this way because the header extraction above is
    // async; everything below is sync database work.
    let db = &state.db;

    let head = db
        .with_reader(|c| {
            c.query_row(
                "SELECT hand_id, table_name, sb_minor, bb_minor, table_size, ts_utc,
                        local_ts, tz_token, board, pot_minor, rake_minor,
                        went_to_showdown, parser_version
                 FROM hands WHERE hand_id = ?1",
                [hand_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, String>(8)?,
                        r.get::<_, i64>(9)?,
                        r.get::<_, i64>(10)?,
                        r.get::<_, i64>(11)?,
                        r.get::<_, i64>(12)?,
                    ))
                },
            )
            .optional()
        })?
        .ok_or_else(|| not_found("hand"))?;
    let (hid, table_name, sb, bb, table_size, ts, local_ts, tz, board, pot, rake, wtsd, parser_version) = head;

    let seats: Vec<SeatView> = db
        .with_reader(|c| {
            let mut stmt = c.prepare(
                "SELECT seat, screen_name, starting_stack_minor, position,
                        contributed_minor, result_minor, shown_cards
                 FROM hand_seats WHERE hand_id = ?1 ORDER BY seat",
            )?;
            let rows = stmt
                .query_map([hand_id], |r| {
                    Ok(SeatRow {
                        seat: r.get::<_, i64>(0)? as u8,
                        name: r.get(1)?,
                        stack: r.get(2)?,
                        position: r.get(3)?,
                        contributed: r.get(4)?,
                        result: r.get(5)?,
                        cards: r.get(6)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<SeatRow>>>()?;
            Ok(rows)
        })?
        .into_iter()
        .map(|s| SeatView {
            seat: s.seat,
            name: s.name.clone(),
            stack: money_of(s.stack),
            position: s.position,
            contributed: money_of(s.contributed),
            result: money_of(s.result),
            positive: s.result >= 0,
            cards: s.cards,
        })
        .collect();

    let raw = hands_store::raw_hand(db, &hid)?.unwrap_or_default();

    struct SeatRow {
        seat: u8,
        name: String,
        stack: i64,
        position: String,
        contributed: i64,
        result: i64,
        cards: String,
    }

    let (actions, actions_note) =
        match parser::default_parser().parse_hand(&raw) {
            Ok(parsed) => {
                let seat_names: HashMap<u8, String> = parsed
                    .seats
                    .iter()
                    .map(|s| (s.seat, s.screen_name.clone()))
                    .collect();
                let actions = parsed
                    .actions
                    .iter()
                    .map(|a| ActionView {
                        street: street_label(a.street),
                        who: seat_names.get(&a.seat).cloned().unwrap_or_else(|| format!("seat {}", a.seat)),
                        kind: action_kind_label(a.kind).to_string(),
                        amount: if a.amount > 0 {
                            money_of(a.amount)
                        } else {
                            String::new()
                        },
                    })
                    .collect();
                (actions, None)
            }
            Err(e) => (
                Vec::new(),
                Some(format!(
                    "The full action list could not be re-derived from the retained raw text ({e}). \
                     See the raw text below — it is the source of truth."
                )),
            ),
        };

    let detail = HandDetail {
        hand_id: hid,
        table_name,
        stakes: format!("{} / {}", money_of(sb), money_of(bb)),
        table_size: table_size as u8,
        ts: crate::web::render::dt_str(ts),
        local_ts,
        tz_token: tz,
        board,
        pot: money_of(pot),
        rake: money_of(rake),
        went_to_showdown: wtsd == 1,
        parser_version,
        seats,
        actions,
        actions_available: true,
        actions_note,
        raw,
    };

    let fv = FilterView::parse(None);
    let bar = FilterBar::build(state, &fv, "/hands", false);
    let mut obj = base_json(
        &format!("Hand {}", detail.hand_id),
        "/hands",
        "hands",
        "hand_detail.html",
        &bar,
    );
    let obj = obj.as_object_mut().ok_or_else(|| internal("base object"))?;
    obj.insert(
        "hand".into(),
        serde_json::to_value(&detail).map_err(|e| internal(e))?,
    );
    Ok(screen_value("hand_detail.html", obj, wants_fragment(&axum::http::HeaderMap::new())))
}

fn street_label(s: Street) -> &'static str {
    match s {
        Street::Preflop => "Preflop",
        Street::Flop => "Flop",
        Street::Turn => "Turn",
        Street::River => "River",
    }
}

fn action_kind_label(k: ActionKind) -> &'static str {
    match k {
        ActionKind::PostSmallBlind => "posts small blind",
        ActionKind::PostBigBlind => "posts big blind",
        ActionKind::PostAnte => "posts ante",
        ActionKind::PostDead => "posts dead blind",
        ActionKind::Fold => "folds",
        ActionKind::Check => "checks",
        ActionKind::Call => "calls",
        ActionKind::Bet => "bets",
        ActionKind::Raise => "raises",
        ActionKind::AllIn => "all-in",
        ActionKind::ReturnUncalled => "returned uncalled bet",
        ActionKind::SitOut => "sits out",
    }
}

// ---------------------------------------------------------------------------
// GET /import (+ POST /import/retry/:id)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ImportView {
    running: bool,
    kind: String,
    files_done: u64,
    files_total: u64,
    hands_imported: u64,
    duplicates: u64,
    failures: u64,
    quarantined: u64,
    current_file: Option<String>,
    finished: bool,
    last_error: Option<String>,
    last_run: Option<LastRun>,
    #[serde(rename = "quarantineCount")]
    quarantine_count: i64,
    quarantine: Vec<QuarantineView>,
}

#[derive(Serialize)]
struct LastRun {
    kind: String,
    started: String,
    finished: Option<String>,
    hands_imported: i64,
    duplicates: i64,
    failures: i64,
}

#[derive(Serialize)]
struct QuarantineView {
    id: i64,
    excerpt: String,
    reason: String,
    #[serde(rename = "parserVersion")]
    parser_version: u32,
    first_seen: String,
    last_seen: String,
    retries: i64,
}

fn excerpt(raw: &str, max: usize) -> String {
    let flat: String = raw.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).collect();
    let flat = flat.trim();
    if flat.chars().count() <= max {
        flat.to_string()
    } else {
        let cut: String = flat.chars().take(max).collect();
        format!("{cut}…")
    }
}

pub async fn import_screen(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> WebResult<Response> {
    let snap = st.importer.snapshot();
    let last_run = hands_store::latest_import_run(&st.db)?.map(|(kind, started, hands, dups, fails, finished)| LastRun {
        kind,
        started: crate::web::render::dt_str(started),
        finished: finished.map(crate::web::render::dt_str),
        hands_imported: hands,
        duplicates: dups,
        failures: fails,
    });
    let quarantine_count = hands_store::quarantine_count(&st.db)?;
    let quarantine: Vec<QuarantineView> = hands_store::list_quarantine(&st.db, QUARANTINE_PAGE, 0)?
        .into_iter()
        .map(|r| QuarantineView {
            id: r.id,
            excerpt: excerpt(&r.raw_text, 160),
            reason: r.reason,
            parser_version: r.parser_version,
            first_seen: crate::web::render::dt_str(r.first_seen),
            last_seen: crate::web::render::dt_str(r.last_seen),
            retries: r.retry_count,
        })
        .collect();

    let view = ImportView {
        running: snap.running,
        kind: snap.kind,
        files_done: snap.progress.files_done,
        files_total: snap.progress.files_total,
        hands_imported: snap.progress.hands_imported,
        duplicates: snap.progress.duplicates,
        failures: snap.progress.failures,
        quarantined: snap.progress.quarantined,
        current_file: snap.progress.current_file,
        finished: snap.progress.finished,
        last_error: snap.last_error,
        last_run,
        quarantine_count,
        quarantine,
    };

    let fv = FilterView::parse(None);
    let bar = FilterBar::build(&st, &fv, "/import", false);
    let mut obj = base_json("Import & quarantine", "/import", "import", "import.html", &bar);
    let obj = obj.as_object_mut().ok_or_else(|| internal("base object"))?;
    obj.insert(
        "import".into(),
        serde_json::to_value(&view).map_err(|e| internal(e))?,
    );
    Ok(screen_value("import.html", obj.clone(), wants_fragment(&headers)))
}

/// POST /import/retry/:id — re-parse one quarantined raw hand with the current
/// parser. Success deletes the quarantine row; failure bumps the retry count
/// and keeps the row visible (spec stories 18–21).
pub async fn retry_quarantine(
    State(st): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Redirect, WebError> {
    let db = st.db.clone();
    // Fetch the raw text before mutating anything.
    let row = db
        .with_reader(|c| {
            c.query_row(
                "SELECT source_file, raw_text, parser_version FROM quarantine WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .optional()
        })?
        .ok_or_else(|| not_found("quarantine row"))?;
    let (source_file, raw_text, _old_version) = row;

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        retry_parse_and_store(&db, &source_file, &raw_text)
    }));
    match result {
        Ok(Ok(())) => {
            // Recovered: the hand now lives in the parsed tables.
            hands_store::delete_quarantine(&db, id)?;
        }
        Ok(Err(reason)) => {
            hands_store::bump_quarantine_retry(&db, id)?;
            let _ = hands_store::quarantine(&db, &source_file, &raw_text, &reason, crate::storage::schema::PARSER_VERSION);
        }
        Err(_) => {
            // A panic (e.g. an analysis dependency still landing) must not
            // crash the request thread.
            hands_store::bump_quarantine_retry(&db, id)?;
        }
    }
    Ok(Redirect::to("/import"))
}

fn retry_parse_and_store(db: &Db, source_file: &str, raw_text: &str) -> Result<(), String> {
    let parsed = parser::default_parser().parse_hand(raw_text).map_err(|e| e.to_string())?;

    // Use exactly the importer's hero resolution, so a retried hand produces the
    // same analysis row it would have produced on a normal import. Identity is
    // resolved through storage, never inside the derivation.
    let analysis = match db.active_profile().ok().flatten() {
        Some(profile) => {
            let players = db.players_for_profile(profile.id).unwrap_or_default();
            crate::stats::analyze_hero(&parsed, profile.id, &players)
        }
        None => None,
    };

    hands_store::upsert_hand(db, &parsed, analysis.as_ref(), raw_text, source_file)
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// GET /settings (+ POST actions)
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct ProfileView {
    id: i64,
    name: String,
    active: bool,
    players: Vec<String>,
}

pub async fn settings(
    State(st): State<AppState>,
    headers: HeaderMap,
) -> WebResult<Response> {
    let active_id = st.db.active_profile().ok().flatten().map(|p: HeroProfile| p.id);
    let profiles: Vec<ProfileView> = st
        .db
        .list_profiles()?
        .into_iter()
        .map(|p| ProfileView {
            id: p.id,
            name: p.name.clone(),
            active: Some(p.id) == active_id,
            players: st
                .db
                .players_for_profile(p.id)
                .unwrap_or_default()
                .into_iter()
                .map(|(name, _)| name)
                .collect(),
        })
        .collect();
    let gap_secs = st.db.session_gap_secs()?;

    let fv = FilterView::parse(None);
    let bar = FilterBar::build(&st, &fv, "/settings", false);
    let mut obj = base_json("Settings", "/settings", "settings", "settings.html", &bar);
    let obj = obj.as_object_mut().ok_or_else(|| internal("base object"))?;
    obj.insert(
        "profiles".into(),
        serde_json::to_value(&profiles).map_err(|e| internal(e))?,
    );
    obj.insert("gapMinutes".into(), serde_json::Value::from(gap_secs / 60));
    obj.insert(
        "snapshotDir".into(),
        serde_json::Value::String(st.cfg.snapshot_dir.display().to_string()),
    );
    Ok(screen_value("settings.html", obj.clone(), wants_fragment(&headers)))
}

pub async fn create_profile(
    State(st): State<AppState>,
    axum::Form(form): axum::Form<HashMap<String, String>>,
) -> Result<Redirect, WebError> {
    let name = form.get("name").cloned().unwrap_or_default();
    if !name.trim().is_empty() {
        st.db.create_profile(name.trim())?;
    }
    Ok(Redirect::to("/settings"))
}

pub async fn add_profile_player(
    State(st): State<AppState>,
    Path(profile_id): Path<i64>,
    axum::Form(form): axum::Form<HashMap<String, String>>,
) -> Result<Redirect, WebError> {
    let name = form.get("screen_name").cloned().unwrap_or_default();
    if !name.trim().is_empty() {
        let player_id = st.db.upsert_player(name.trim())?;
        st.db.add_profile_player(profile_id, player_id)?;
    }
    Ok(Redirect::to("/settings"))
}

pub async fn activate_profile(
    State(st): State<AppState>,
    Path(profile_id): Path<i64>,
) -> Result<Redirect, WebError> {
    st.db.set_active_profile(profile_id)?;
    Ok(Redirect::to("/settings"))
}

pub async fn set_session_gap(
    State(st): State<AppState>,
    axum::Form(form): axum::Form<HashMap<String, String>>,
) -> Result<Redirect, WebError> {
    let minutes: i64 = form
        .get("gap_minutes")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(30)
        .max(1);
    st.db.set_setting(crate::storage::SETTING_SESSION_GAP_SECS, &(minutes * 60).to_string())?;
    Ok(Redirect::to("/settings"))
}

/// POST /settings/snapshot — manual `VACUUM INTO` snapshot (definition-of-done
/// item 8). Blocking I/O goes through `spawn_blocking`.
pub async fn take_snapshot_now(
    State(st): State<AppState>,
) -> Result<Redirect, WebError> {
    let db = st.db.clone();
    let dir = st.cfg.snapshot_dir.clone();
    let result = tokio::task::spawn_blocking(move || crate::snapshot::take_snapshot(&db, &dir))
        .await
        .map_err(|e| internal(e))?;
    match result {
        Ok(path) => {
            tracing::info!(snapshot = %path.display(), "manual snapshot taken");
            Ok(Redirect::to("/settings?snapshot=ok"))
        }
        Err(e) => {
            tracing::error!(error = %e, "manual snapshot failed");
            Ok(Redirect::to("/settings?snapshot=failed"))
        }
    }
}

// ---------------------------------------------------------------------------
// GET /healthz
// ---------------------------------------------------------------------------

/// The Docker HEALTHCHECK hits this. It must return 200 fast and must not
/// touch the importer lock at all.
pub async fn healthz(State(st): State<AppState>) -> WebResult<Response> {
    let hands = hands_store::hand_count_total(&st.db)?;
    let quarantined = hands_store::quarantine_count(&st.db)?;
    let body = serde_json::json!({
        "status": "ok",
        "version": crate::VERSION,
        "hands": hands,
        "quarantined": quarantined,
    });
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response())
}

// ---------------------------------------------------------------------------
// CSV exports
// ---------------------------------------------------------------------------

/// GET /export/aggregate.csv — the filtered aggregate table as CSV.
///
/// Money columns are exported in MAJOR units through `Money::format_plain`
/// (display-time exponent division, exactly like the HTML screens);
/// `currency_per_100` is stored in minor units and rounded to whole minor
/// units before the same conversion.
pub async fn export_aggregate_csv(
    State(st): State<AppState>,
    RawQuery(raw): RawQuery,
) -> WebResult<Response> {
    let fv = FilterView::parse(raw.as_deref());
    let groups = q::aggregate(&st.db, &fv.spec, group_param(&fv))?;
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record([
        "group",
        "hands",
        "net_result",
        "adjusted_result",
        "rake_attributed",
        "big_blinds_won",
        "bb_per_100",
        "currency_per_100",
    ])
    .map_err(internal)?;
    for g in &groups {
        let r = &g.row;
        // `currency_per_100` is minor units; the exponent division happens only
        // at presentation. CSV gets a machine-readable decimal with no digit
        // grouping, so the file can be consumed as numbers.
        let per100 = Money::new(r.currency_per_100.round() as i64, Currency::Eur).format_machine();
        w.write_record([
            g.label.as_str(),
            &r.hands.to_string(),
            &Money::new(r.net_result_minor, Currency::Eur).format_machine(),
            &Money::new(r.adjusted_result_minor, Currency::Eur).format_machine(),
            &Money::new(r.rake_attributed_minor, Currency::Eur).format_machine(),
            &format!("{:.2}", r.bb_won),
            &format!("{:.2}", r.bb_per_100),
            &per100,
        ])
        .map_err(internal)?;
    }
    let body = w
        .into_inner()
        .map_err(internal)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .map_err(|e| internal(e))?;
    Ok(csv_response(body, "aggregate.csv"))
}

/// GET /export/hands.csv — the retained raw texts of the filtered set.
pub async fn export_hands_csv(
    State(st): State<AppState>,
    RawQuery(raw): RawQuery,
) -> WebResult<Response> {
    let fv = FilterView::parse(raw.as_deref());
    let mut w = csv::Writer::from_writer(Vec::new());
    w.write_record(["hand_id", "ts_utc", "raw_text"]).map_err(internal)?;

    // Page through the filtered hand list (session filtering inside
    // `hand_list` is handled there), then batch-fetch the raw texts.
    const PAGE: i64 = 1000;
    let mut offset = 0i64;
    loop {
        let rows = q::hand_list(&st.db, &fv.spec, PAGE, offset)?;
        if rows.is_empty() {
            break;
        }
        offset += rows.len() as i64;
        let ids: Vec<String> = rows.iter().map(|r| r.hand_id.clone()).collect();
        for chunk in ids.chunks(400) {
            let raws = raw_hands_by_ids(&st.db, chunk)?;
            for (id, raw) in chunk.iter().zip(raws) {
                w.write_record([id, "", raw.as_str()]).map_err(internal)?;
            }
        }
    }
    let body = w
        .into_inner()
        .map_err(internal)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .map_err(|e| internal(e))?;
    Ok(csv_response(body, "hands.csv"))
}

/// Raw texts for a list of ids (empty string when a raw row is missing — it
/// never should be, but an export must not halt on one hole).
fn raw_hands_by_ids(db: &Db, ids: &[String]) -> WebResult<Vec<String>> {
    let clauses: Vec<String> = (0..ids.len()).map(|i| format!("?{}", i + 1)).collect();
    let sql = format!(
        "SELECT hand_id, raw_text FROM raw_hands WHERE hand_id IN ({})",
        clauses.join(",")
    );
    let params: Vec<&dyn rusqlite::ToSql> = ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
    let found: HashMap<String, String> = db.with_reader(|c| {
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt
            .query_map(params.as_slice(), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().collect())
    })?;
    Ok(ids.iter().map(|id| found.get(id).cloned().unwrap_or_default()).collect())
}

fn csv_response(body: String, filename: &str) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                Box::leak(format!("attachment; filename=\"{filename}\"").into_boxed_str()),
            ),
        ],
        body,
    )
        .into_response()
}
