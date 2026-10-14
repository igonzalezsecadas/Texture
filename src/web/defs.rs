//! The statistic dictionary (`docs/stat-definitions.md`), embedded at build
//! time and parsed into a lookup. Every statistic shown in the UI carries its
//! dictionary definition (spec story 65; definition-of-done item 5).

use std::collections::HashMap;
use std::sync::OnceLock;

/// Embedded copy of the authoritative dictionary. From `src/web/`, two
/// `..` steps reach the repo root.
const DICTIONARY_MD: &str = include_str!("../../docs/stat-definitions.md");

/// `(dictionary heading, definition text)` for every `### Heading` section
/// whose body carries a `**Definition:**` line.
pub type Dictionary = HashMap<&'static str, String>;

fn load() -> Dictionary {
    let mut map: Dictionary = HashMap::new();
    let mut heading: Option<&'static str> = None;
    let mut body = String::new();
    let mut capturing = false;

    let mut flush = |heading: &mut Option<&'static str>, body: &mut String, capturing: &mut bool| {
        if let (Some(h), true) = (*heading, *capturing) {
            map.insert(h, body.trim().to_string());
        }
        *heading = None;
        body.clear();
        *capturing = false;
    };

    for line in DICTIONARY_MD.lines() {
        if let Some(h) = line.strip_prefix("### ") {
            flush(&mut heading, &mut body, &mut capturing);
            heading = Some(h.trim());
            continue;
        }
        if let Some(rest) = line.strip_prefix("**Definition:**") {
            capturing = true;
            body.push_str(rest.trim());
            body.push(' ');
            continue;
        }
        if capturing {
            // A definition runs until the next `**Section:**` marker, the next
            // heading, or a section separator. Bullet lines inside the
            // definition ("- *Edge case*: ...") belong to it.
            let t = line.trim_start();
            if t.starts_with("**")
                || t.starts_with('#')
                || t.starts_with("---")
                || t.starts_with("## ")
            {
                flush(&mut heading, &mut body, &mut capturing);
            } else if !t.is_empty() {
                body.push_str(t);
                body.push(' ');
            }
        }
    }
    flush(&mut heading, &mut body, &mut capturing);
    map
}

/// The process-wide dictionary. Parsing is cheap and pure; `OnceLock` keeps
/// every handler from repeating it.
pub fn dictionary() -> &'static Dictionary {
    static DICT: OnceLock<Dictionary> = OnceLock::new();
    DICT.get_or_init(load)
}

/// Definition for a dictionary heading (e.g. `"VPIP (voluntarily put chips in
/// pot)"`). Empty string when the heading is missing — which is a bug in the
/// dictionary keys, surfaced loudly in the UI as "no definition".
pub fn definition(heading: &str) -> String {
    dictionary().get(heading).cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_v1_statistic_has_a_definition() {
        let d = dictionary();
        // A representative sample spanning all report screens; these must match
        // the headings in docs/stat-definitions.md exactly.
        for key in [
            "Hands played",
            "Net result (winnings)",
            "BB/100",
            "€/100",
            "All-in adjusted (EV) result",
            "Luck delta (\"actual minus adjusted\")",
            "Rake attributed (weighted contribution)",
            "Showdown winnings",
            "Non-showdown winnings",
            "VPIP (voluntarily put chips in pot)",
            "PFR (preflop raise)",
            "Limp % (VPIP−PFR gap)",
            "Raise First In (RFI / open-raise rate)",
            "3-Bet",
            "Fold to 3-Bet",
            "4-Bet",
            "Fold to 4-Bet",
            "Cold call",
            "Steal attempt",
            "Fold to Steal / Blind defense",
            "C-Bet Flop",
            "C-Bet Turn",
            "C-Bet River",
            "Fold to C-Bet (by street)",
            "Check-Raise (by street)",
            "Probe Bet",
            "Delayed C-Bet",
            "WWSF (Won When Saw Flop)",
            "WTSD (Went To Showdown)",
            "W$SD (Won At Showdown)",
            "Aggression Factor (AF)",
            "Aggression Frequency (Agg%)",
        ] {
            let def = d.get(key).unwrap_or_else(|| panic!("missing definition: {key}"));
            assert!(!def.is_empty(), "empty definition: {key}");
        }
    }
}
