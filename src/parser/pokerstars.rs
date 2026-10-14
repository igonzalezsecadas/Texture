//! PokerStars hand-history parser.
//!
//! Format reference (PokerStars writes one hand per block):
//!
//! ```text
//! PokerStars Hand #123456789:  Hold'em No Limit (€0.25/€0.50 EUR) - 2024/01/02 13:45:06 CET [2024/01/02 7:45:06 ET]
//! Table 'Alpha' 6-max Seat #3 is the button
//! Seat 1: villain1 (€50 in chips)
//! ...
//! *** HOLE CARDS ***
//! Dealt to hero [As Kd]
//! ...
//! *** SUMMARY ***
//! Total pot €10 | Rake €0.50
//! ...
//! ```
//!
//! The parser is the only place in the crate allowed to know this format. See
//! `fixtures/parser/` for the golden corpus.

use crate::domain::ParsedHand;
use crate::domain::Site;
use crate::error::ParseError;
use crate::parser::HandParser;

pub struct PokerStarsParser;

const HEADER_PREFIX: &str = "PokerStars Hand #";
const HAND_SEPARATOR: &str = "PokerStars Hand #";
const SUMMARY_MARKER: &str = "*** SUMMARY ***";

impl HandParser for PokerStarsParser {
    fn site(&self) -> Site {
        Site::PokerStars
    }

    fn can_parse(&self, text: &str) -> bool {
        text.trim_start().starts_with(HEADER_PREFIX)
    }

    fn parse_hand(&self, raw: &str) -> Result<ParsedHand, ParseError> {
        parse_hand(raw)
    }
}

/// Parse one complete PokerStars hand history.
pub fn parse_hand(_raw: &str) -> Result<ParsedHand, ParseError> {
    // Implemented by the parser workstream. See docs/stat-definitions.md and
    // fixtures/parser/ for the golden corpus this must satisfy.
    unimplemented!("PokerStarsParser::parse_hand")
}

/// Split a file's text into complete hand blocks, returning `(hands, consumed)`
/// where `consumed` is the byte offset of the start of the trailing incomplete
/// hand (or `text.len()` when every hand is complete).
pub fn split_complete_hands(text: &str) -> (Vec<&str>, usize) {
    let mut hands = Vec::new();
    let mut starts: Vec<usize> = Vec::new();
    if text.starts_with(HAND_SEPARATOR) || text.starts_with('\u{feff}') {
        // Record the first start, tolerating a BOM.
        let first = if text.starts_with('\u{feff}') { '\u{feff}'.len_utf8() } else { 0 };
        starts.push(first);
    }
    let mut search_from = starts.first().copied().unwrap_or(0);
    while let Some(rel) = text[search_from..].find(HAND_SEPARATOR) {
        let idx = search_from + rel;
        if idx == 0 || (idx > 0 && text[..idx].ends_with('\n')) {
            if !starts.contains(&idx) {
                starts.push(idx);
            }
        }
        search_from = idx + HAND_SEPARATOR.len();
    }
    if starts.is_empty() {
        return (hands, text.len());
    }

    let mut consumed = text.len();
    for (i, start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(text.len());
        let block = &text[*start..end];
        if block.trim().is_empty() {
            continue;
        }
        if block.contains(SUMMARY_MARKER) {
            hands.push(block);
        } else {
            consumed = *start;
            break;
        }
    }
    (hands, consumed)
}
