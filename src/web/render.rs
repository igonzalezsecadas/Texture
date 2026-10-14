//! Template rendering. Every template is embedded with `include_str!` and
//! registered on a process-wide minijinja [`Environment`], so the container
//! never depends on a runtime template path.

use minijinja::{Environment, Value};
use std::sync::OnceLock;
use crate::money::{Currency, Money};

// Template sources (embedded — never read from disk at runtime).
const LAYOUT: &str = include_str!("templates/layout.html");
const FILTER_BAR: &str = include_str!("templates/filter_bar.html");
const DASHBOARD: &str = include_str!("templates/dashboard.html");
const STAT_TABLE: &str = include_str!("templates/stat_table.html");
const PREFLOP: &str = include_str!("templates/preflop.html");
const POSTFLOP: &str = include_str!("templates/postflop.html");
const POSITION: &str = include_str!("templates/position.html");
const SESSIONS: &str = include_str!("templates/sessions.html");
const HANDS: &str = include_str!("templates/hands.html");
const HAND_DETAIL: &str = include_str!("templates/hand_detail.html");
const IMPORT: &str = include_str!("templates/import.html");
const SETTINGS: &str = include_str!("templates/settings.html");

/// The process-wide template environment. Registration is infallible by
/// construction (embedded strings, checked by the web tests).
pub fn env() -> &'static Environment<'static> {
    static ENV: OnceLock<Environment<'static>> = OnceLock::new();
    ENV.get_or_init(|| {
        let mut env = Environment::new();
        // Hand text, screen names and table names are user data, so `.html`
        // templates are escaped by default. `|safe` opts out for the chart
        // payload, which is JSON inside a script tag.
        env.set_auto_escape_callback(minijinja::default_auto_escape_callback);
        let register = |env: &mut Environment<'static>, name: &'static str, src: &'static str| {
            env.add_template(name, src)
                .unwrap_or_else(|e| panic!("invalid template {name}: {e}"));
        };
        register(&mut env, "layout.html", LAYOUT);
        register(&mut env, "filter_bar.html", FILTER_BAR);
        register(&mut env, "dashboard.html", DASHBOARD);
        register(&mut env, "stat_table.html", STAT_TABLE);
        register(&mut env, "preflop.html", PREFLOP);
        register(&mut env, "postflop.html", POSTFLOP);
        register(&mut env, "position.html", POSITION);
        register(&mut env, "sessions.html", SESSIONS);
        register(&mut env, "hands.html", HANDS);
        register(&mut env, "hand_detail.html", HAND_DETAIL);
        register(&mut env, "import.html", IMPORT);
        register(&mut env, "settings.html", SETTINGS);

        env.add_filter("money", money);
        env.add_filter("money_plain", money_plain);
        env.add_filter("per100_money", per100_money);
        env.add_filter("pct", pct);
        env.add_filter("rate2", rate2);
        env.add_filter("dt", dt);
        env.add_filter("date", date);
        env.add_filter("dur", dur);
        env.add_filter("usize", usize_of);
        env
    })
}

// ---------------------------------------------------------------------------
// Formatting filters
// ---------------------------------------------------------------------------

/// Aggregated minor units → `€1,234.56`. v1 assumes a single EUR account
/// (ADR-0003); the money is formatted exclusively through [`Money::format`],
/// never with floats.
fn money(minor: i64) -> String {
    Money::new(minor, Currency::Eur).format()
}

/// Same, without the currency symbol (for CSV).
fn money_plain(minor: i64) -> String {
    Money::new(minor, Currency::Eur).format_plain()
}

/// `AggregateRow::currency_per_100` is stored in MINOR units per the statistic
/// dictionary. The division by the currency exponent (100 for EUR) happens
/// HERE, at display time — and only via [`Money::format`], which rounds the
/// minor-unit value to whole minor units before scaling. No float money ever
/// exists outside this presentation step.
fn per100_money(minor_per_100: f64) -> String {
    Money::new(minor_per_100.round() as i64, Currency::Eur).format()
}

/// `pct(numerator, denominator)` → `12.3%`, or `—` when the denominator is 0.
fn pct(numerator: i64, denominator: i64) -> String {
    if denominator == 0 {
        return "—".to_string();
    }
    format!("{:.1}%", numerator as f64 / denominator as f64 * 100.0)
}

/// A unitless rate (BB/100, BB won, AF) to two decimals; `—` when undefined.
fn rate2(value: f64, defined: bool) -> String {
    if defined {
        format!("{value:.2}")
    } else {
        "—".to_string()
    }
}

/// Unix seconds → `2024-01-03 14:22 UTC`.
fn dt(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| ts.to_string())
}

/// Unix seconds → `2024-01-03`.
fn date(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| ts.to_string())
}

/// Seconds → `1d 2h 03m`-style human duration.
fn dur(secs: i64) -> String {
    let s = secs.max(0);
    let days = s / 86_400;
    let hours = (s % 86_400) / 3_600;
    let mins = (s % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {mins:02}m")
    } else if hours > 0 {
        format!("{hours}h {mins:02}m")
    } else {
        format!("{mins}m")
    }
}

fn usize_of(v: Value) -> Value {
    Value::from(v.to_string())
}

// ---------------------------------------------------------------------------
// Public formatting helpers for view models
// ---------------------------------------------------------------------------
//
// The view models pre-format their numbers so a template never sees a raw float
// for money, and never has to know the currency exponent.

/// `12.3%`, or an em dash when the denominator is zero (stat undefined).
pub fn pct_str(numerator: i64, denominator: i64) -> String {
    pct(numerator, denominator)
}

/// Human duration in seconds; the UI labels session durations as approximate.
pub fn dur_str(secs: i64) -> String {
    dur(secs)
}

/// Unix seconds to a UTC timestamp.
pub fn dt_str(ts: i64) -> String {
    dt(ts)
}
