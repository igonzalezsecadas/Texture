//! Hand-history parsing. Isolated behind a [`HandParser`] boundary so another
//! site could be added without touching the reporting layer.

pub mod pokerstars;

use crate::domain::{ParsedHand, Site};
use crate::error::ParseError;

/// A parser for one poker site's hand-history format.
pub trait HandParser: Send + Sync {
    fn site(&self) -> Site;
    /// Cheap sniff: does this text plausibly belong to this site?
    fn can_parse(&self, text: &str) -> bool;
    /// Parse exactly one complete hand history.
    fn parse_hand(&self, raw: &str) -> Result<ParsedHand, ParseError>;
}

/// The active parser.
pub fn default_parser() -> impl HandParser {
    pokerstars::PokerStarsParser
}

/// Parse one complete PokerStars hand history with the default parser.
pub use pokerstars::parse_hand;

/// Split a hand-history file's text into complete hand histories and the byte
/// offset where the trailing incomplete hand begins.
///
/// A hand is complete only when its terminating summary section is present. This
/// is what makes importing a file that is being written to while playing safe.
pub fn split_complete_hands(text: &str) -> (Vec<&str>, usize) {
    pokerstars::split_complete_hands(text)
}
