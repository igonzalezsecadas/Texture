//! Query-string filters. The filter bar's state lives in the URL; every report
//! screen parses the same query string into one [`FilterSpec`] which is then
//! handed to every query in `storage::queries`, so no two statistics can be
//! computed on a different population.
//!
//! Parameter mapping (all optional; repeated params and comma-separated lists
//! are both accepted):
//!
//! | param     | example            | maps to                    |
//! |-----------|--------------------|----------------------------|
//! | `from`    | `2024-01-01`       | `FilterSpec::from` (inclusive, UTC midnight) |
//! | `to`      | `2024-02-01`       | `FilterSpec::to` (EXCLUSIVE, UTC midnight) |
//! | `stake`   | `25-50`            | `FilterSpec::stakes` `(sb_minor, bb_minor)` pairs |
//! | `size`    | `6` / `9`          | `FilterSpec::table_sizes`  |
//! | `pos`     | `CO`,`BTN`         | `FilterSpec::positions` (PT4-style codes) |
//! | `session` | `3`                | `FilterSpec::sessions` (filter-local session ids) |
//! | `profile` | `1`                | `FilterSpec::profiles` (hero profile ids) |
//!
//! `group` and `page` are presentation parameters and are not part of
//! `FilterSpec`; they travel alongside it in the query string.

use crate::domain::{FilterSpec, GroupBy, Position};

/// The filter bar's view of the world: the parsed [`FilterSpec`] plus the raw
/// strings it was built from (so the form can re-render its values).
#[derive(Debug, Clone, Default)]
pub struct FilterView {
    pub spec: FilterSpec,
    pub from: Option<String>,
    pub to: Option<String>,
    pub stakes: Vec<String>,
    pub sizes: Vec<String>,
    pub positions: Vec<String>,
    pub sessions: Vec<String>,
    pub profiles: Vec<String>,
    /// Presentation parameter: grouping for report tables and CSV export.
    pub group: Option<String>,
}

impl FilterView {
    /// Parse a raw query string (no leading `?`).
    pub fn parse(query: Option<&str>) -> FilterView {
        let mut fv = FilterView::default();
        let Some(query) = query else { return fv };
        for (k, v) in parse_pairs(query) {
            match k.as_str() {
                "from" => {
                    fv.from = Some(v.clone());
                    if let Some(ts) = parse_date_inclusive(&v) {
                        fv.spec.from = Some(ts);
                    }
                }
                "to" => {
                    fv.to = Some(v.clone());
                    if let Some(ts) = parse_date_exclusive(&v) {
                        fv.spec.to = Some(ts);
                    }
                }
                "stake" => {
                    for part in v.split(',') {
                        let part = part.trim();
                        if part.is_empty() || fv.stakes.contains(&part.to_string()) {
                            continue;
                        }
                        fv.stakes.push(part.to_string());
                        if let Some(pair) = parse_stake(part) {
                            fv.spec.stakes.push(pair);
                        }
                    }
                }
                "size" => {
                    for part in v.split(',') {
                        let part = part.trim();
                        if part.is_empty() || fv.sizes.contains(&part.to_string()) {
                            continue;
                        }
                        fv.sizes.push(part.to_string());
                        if let Ok(n) = part.parse::<u8>() {
                            fv.spec.table_sizes.push(n);
                        }
                    }
                }
                "pos" => {
                    for part in v.split(',') {
                        let part = part.trim();
                        if part.is_empty() || fv.positions.contains(&part.to_string()) {
                            continue;
                        }
                        fv.positions.push(part.to_string());
                        if let Some(p) = Position::from_code(part) {
                            fv.spec.positions.push(p);
                        }
                    }
                }
                "session" => {
                    for part in v.split(',') {
                        let part = part.trim();
                        if part.is_empty() || fv.sessions.contains(&part.to_string()) {
                            continue;
                        }
                        fv.sessions.push(part.to_string());
                        if let Ok(n) = part.parse::<i64>() {
                            fv.spec.sessions.push(n);
                        }
                    }
                }
                "profile" => {
                    for part in v.split(',') {
                        let part = part.trim();
                        if part.is_empty() || fv.profiles.contains(&part.to_string()) {
                            continue;
                        }
                        fv.profiles.push(part.to_string());
                        if let Ok(n) = part.parse::<i64>() {
                            fv.spec.profiles.push(n);
                        }
                    }
                }
                "group" => fv.group = Some(v),
                _ => {} // unknown params are ignored, never fatal
            }
        }
        fv
    }

    /// `true` when the URL carries no filter at all: reports must present this
    /// unambiguously as the lifetime total.
    pub fn is_empty(&self) -> bool {
        self.spec.is_empty()
    }

    /// The query string (without a leading `?`) for links preserving the
    /// current filters.
    pub fn query(&self) -> String {
        self.query_with(|_k| true)
    }

    /// The query string minus one key (for per-chip removal links).
    pub fn query_without(&self, key: &str) -> String {
        self.query_with(|k| k != key)
    }

    fn query_with(&self, keep: impl Fn(&str) -> bool + Copy) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut push = |k: &str, v: &str| {
            if keep(k) && !v.is_empty() {
                parts.push(format!("{}={}", k, encode(v)));
            }
        };
        if let Some(f) = &self.from {
            push("from", f);
        }
        if let Some(t) = &self.to {
            push("to", t);
        }
        for s in &self.stakes {
            push("stake", s);
        }
        for s in &self.sizes {
            push("size", s);
        }
        for p in &self.positions {
            push("pos", p);
        }
        for s in &self.sessions {
            push("session", s);
        }
        for p in &self.profiles {
            push("profile", p);
        }
        if let Some(g) = &self.group {
            push("group", g);
        }
        parts.join("&")
    }

    /// A URL for `path` that keeps the current filters.
    pub fn url(&self, path: &str) -> String {
        let q = self.query();
        if q.is_empty() {
            path.to_string()
        } else {
            format!("{path}?{q}")
        }
    }

    /// Active-filter chips for the always-visible state: one entry per active
    /// filter dimension, each with a one-click removal link.
    pub fn chips(&self) -> Vec<Chip> {
        let mut chips = Vec::new();
        let mut chip = |key: &'static str, value: Option<String>| {
            if let Some(v) = value.filter(|v| !v.is_empty()) {
                chips.push(Chip {
                    label: format!("{key}: {v}"),
                    remove_url: self.url_of_query(self.query_without(key)),
                });
            }
        };
        chip("from", self.from.clone());
        chip("to", self.to.clone());
        chip("stake", Some(self.stakes.join(", ")));
        chip("size", Some(self.sizes.join(", ")));
        chip("pos", Some(self.positions.join(", ")));
        chip("session", Some(self.sessions.join(", ")));
        chip("profile", Some(self.profiles.join(", ")));
        chips
    }

    fn url_of_query(&self, q: String) -> String {
        if q.is_empty() {
            // `path` is filled in by the caller via a template default; use a
            // relative link target so the chip clears on the current screen.
            String::new()
        } else {
            format!("?{q}")
        }
    }
}

#[derive(Debug, Clone)]
pub struct Chip {
    pub label: String,
    /// Link (starting with `?`) that removes this one filter.
    pub remove_url: String,
}

/// Parse `k=v&k=v` pairs with minimal percent-decoding (the values this
/// application generates are URL-safe by construction, but hand-edited URLs
/// must not break the parser).
fn parse_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (decode(k), decode(v)),
            None => (decode(p), String::new()),
        })
        .collect()
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(s.get(i + 1..i + 3).unwrap_or("zz"), 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            ' ' => out.push_str("%20"),
            '&' | '=' | '%' | '+' | '#' | '?' => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// `from` is an inclusive UTC midnight.
fn parse_date_inclusive(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let d = chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()?;
    Some(
        chrono::TimeZone::from(&chrono::Utc)
            .with_ymd_and_hms(d.year(), d.month(), d.day(), 0, 0, 0)
            .single()?,
    )
}

/// `to` is EXCLUSIVE UTC midnight of the given date.
fn parse_date_exclusive(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    parse_date_inclusive(s)
}

/// `"25-50"` → `(25, 50)` minor units (small blind, big blind).
fn parse_stake(s: &str) -> Option<(i64, i64)> {
    let (sb, bb) = s.trim().split_once('-')?;
    Some((sb.trim().parse().ok()?, bb.trim().parse().ok()?))
}

/// Presentation grouping (`group=day|week|month|stake|position|none`).
pub fn group_param(fv: &FilterView) -> GroupBy {
    match fv.group.as_deref() {
        Some("day") => GroupBy::Day,
        Some("week") => GroupBy::Week,
        Some("month") => GroupBy::Month,
        Some("stake") => GroupBy::Stake,
        Some("position") => GroupBy::Position,
        _ => GroupBy::None,
    }
}
