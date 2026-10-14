//! The domain vocabulary shared by the parser, the storage layer, and reporting.
//!
//! This module is the frozen contract between workstreams. The parser's only job
//! is to turn raw hand text into a [`ParsedHand`]. Everything downstream consumes
//! [`ParsedHand`], [`HeroHandAnalysis`], and [`FilterSpec`].

use crate::money::{Currency, Money};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Site {
    PokerStars,
}

impl Site {
    pub const fn code(self) -> &'static str {
        match self {
            Site::PokerStars => "PokerStars",
        }
    }
    pub fn from_code(code: &str) -> Option<Site> {
        match code {
            "PokerStars" => Some(Site::PokerStars),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Game {
    NoLimitHoldem,
}

impl Game {
    pub const fn code(self) -> &'static str {
        match self {
            Game::NoLimitHoldem => "NLHE",
        }
    }
    pub fn from_code(code: &str) -> Option<Game> {
        match code {
            "NLHE" => Some(Game::NoLimitHoldem),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
}

impl Street {
    pub const ALL: [Street; 4] = [Street::Preflop, Street::Flop, Street::Turn, Street::River];
    pub fn index(self) -> usize {
        match self {
            Street::Preflop => 0,
            Street::Flop => 1,
            Street::Turn => 2,
            Street::River => 3,
        }
    }
    pub const fn code(self) -> &'static str {
        match self {
            Street::Preflop => "preflop",
            Street::Flop => "flop",
            Street::Turn => "turn",
            Street::River => "river",
        }
    }
}

/// A table position, ordered from the worst seat (small blind) to the best (button).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Position {
    Sb,
    Bb,
    Utg,
    Utg1,
    Mp,
    Hj,
    Co,
    Btn,
}

impl Position {
    pub const ALL: [Position; 8] = [
        Position::Sb,
        Position::Bb,
        Position::Utg,
        Position::Utg1,
        Position::Mp,
        Position::Hj,
        Position::Co,
        Position::Btn,
    ];

    pub const fn code(self) -> &'static str {
        match self {
            Position::Sb => "SB",
            Position::Bb => "BB",
            Position::Utg => "UTG",
            Position::Utg1 => "UTG1",
            Position::Mp => "MP",
            Position::Hj => "HJ",
            Position::Co => "CO",
            Position::Btn => "BTN",
        }
    }

    pub fn from_code(code: &str) -> Option<Position> {
        Position::ALL.into_iter().find(|p| p.code().eq_ignore_ascii_case(code))
    }
}

/// A playing card. `rank` is 2..=14, suit is 'c' | 'd' | 'h' | 's'.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Card {
    pub rank: u8,
    pub suit: char,
}

impl Card {
    pub fn parse(s: &str) -> Option<Card> {
        let mut chars = s.trim().chars();
        let r = chars.next()?;
        let suit = chars.next()?.to_ascii_lowercase();
        let rank = match r.to_ascii_uppercase() {
            '2'..='9' => r.to_digit(10)? as u8,
            'T' => 10,
            'J' => 11,
            'Q' => 12,
            'K' => 13,
            'A' => 14,
            _ => return None,
        };
        if !matches!(suit, 'c' | 'd' | 'h' | 's') {
            return None;
        }
        Some(Card { rank, suit })
    }
}

impl std::fmt::Display for Card {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let r = match self.rank {
            10 => 'T',
            11 => 'J',
            12 => 'Q',
            13 => 'K',
            14 => 'A',
            n => (b'0' + n) as char,
        };
        write!(f, "{}{}", r, self.suit)
    }
}

/// One action taken by one seat. The parser emits the full action list; the
/// persistence layer keeps only the hero-derived flags (see [`HeroHandAnalysis`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionKind {
    PostSmallBlind,
    PostBigBlind,
    PostAnte,
    PostDead,
    Fold,
    Check,
    Call,
    Bet,
    Raise,
    AllIn,
    /// Uncalled bet returned to the last aggressor.
    ReturnUncalled,
    /// Sit-out / disconnect / timeout notice attached to a seat.
    SitOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub street: Street,
    pub seat: u8,
    pub kind: ActionKind,
    /// Amount moved by this action, in minor units.
    pub amount: i64,
    /// Total amount this seat has committed on this street after the action.
    pub to_amount: i64,
    /// True when this action is the first voluntary aggression of the street.
    pub is_first_aggression: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedSeat {
    pub seat: u8,
    pub screen_name: String,
    pub starting_stack: Money,
    pub position: Position,
    /// Total chips this seat put into the pot across all streets (excluding
    /// returned uncalled bets). Drives weighted-contributed rake attribution.
    pub contributed: i64,
    /// Net result for this seat: amount won minus amount contributed.
    pub result: Money,
    pub shown_cards: Vec<Card>,
    pub sat_out: bool,
}

/// The parser's complete output contract for a single hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedHand {
    pub site: Site,
    pub hand_id: String,
    pub game: Game,
    pub currency: Currency,
    pub small_blind: Money,
    pub big_blind: Money,
    pub table_name: String,
    /// Maximum number of seats at the table (6 or 9 for the supported formats).
    pub table_size: u8,
    pub button_seat: u8,
    pub timestamp_utc: chrono::DateTime<chrono::Utc>,
    /// The timestamp exactly as written in the hand history.
    pub local_timestamp: String,
    /// The timezone token from the hand history (e.g. `CET`, `WEST`).
    pub tz_token: String,
    pub board: Vec<Card>,
    pub seats: Vec<ParsedSeat>,
    pub actions: Vec<Action>,
    pub pot_total: Money,
    pub rake: Money,
    /// Any jackpot/BBJ drop reported separately from rake, if present.
    pub jackpot_drop: Money,
    pub went_to_showdown: bool,
    /// Every street that was actually dealt, for `saw_flop/turn/river` flags.
    pub streets_dealt: Vec<Street>,
}

impl ParsedHand {
    pub fn seat(&self, seat: u8) -> Option<&ParsedSeat> {
        self.seats.iter().find(|s| s.seat == seat)
    }
    pub fn seat_by_name(&self, name: &str) -> Option<&ParsedSeat> {
        self.seats.iter().find(|s| s.screen_name == name)
    }
}

/// A player identity row. Screen names are stable identities; aliases map several
/// screen names to one hero profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Player {
    pub id: i64,
    pub screen_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeroProfile {
    pub id: i64,
    pub name: String,
    pub player_ids: Vec<i64>,
}

/// Derived, hero-centric per-hand counters. This is the performance-critical row
/// that every report aggregates over. Written once at import time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeroHandAnalysis {
    pub hand_id: String,
    pub profile_id: i64,
    pub player_id: i64,
    pub position: Position,
    pub table_size: u8,
    pub currency: Currency,
    pub small_blind: i64,
    pub big_blind: i64,
    pub timestamp_utc: chrono::DateTime<chrono::Utc>,

    // Hand outcome
    pub net_result: i64,
    pub adjusted_result: i64,
    pub luck_delta: i64,
    pub rake_attributed: i64,
    pub jackpot_attributed: i64,
    pub contributed: i64,
    pub won_amount: i64,
    pub sd_winnings: i64,
    pub non_sd_winnings: i64,
    pub went_to_showdown: bool,
    pub won_at_showdown: bool,
    pub saw_flop: bool,
    pub saw_turn: bool,
    pub saw_river: bool,
    pub wwsf_opportunity: bool,
    pub won_when_saw_flop: bool,

    // Preflop
    pub vpip: bool,
    pub pfr: bool,
    pub limp: bool,
    pub rfi: bool,
    pub rfi_opportunity: bool,
    pub three_bet: bool,
    pub three_bet_opportunity: bool,
    pub faced_three_bet: bool,
    pub folded_to_three_bet: bool,
    pub four_bet: bool,
    pub four_bet_opportunity: bool,
    pub faced_four_bet: bool,
    pub folded_to_four_bet: bool,
    pub cold_call: bool,
    pub cold_call_opportunity: bool,
    pub steal_attempt: bool,
    pub steal_opportunity: bool,
    pub faced_steal: bool,
    pub folded_to_steal: bool,

    // Postflop, indexed by street in [flop, turn, river] order.
    pub cbet_opportunity: [bool; 3],
    pub cbet: [bool; 3],
    pub faced_cbet: [bool; 3],
    pub folded_to_cbet: [bool; 3],
    pub check_raise: [bool; 3],
    pub probe_opportunity: [bool; 3],
    pub probe: [bool; 3],
    pub delayed_cbet_opportunity: [bool; 3],
    pub delayed_cbet: [bool; 3],

    // Aggression, indexed by street in [preflop, flop, turn, river] order.
    pub aggressive_actions: [u32; 4],
    pub passive_actions: [u32; 4],
}

impl HeroHandAnalysis {
    pub fn bb100_factor(&self) -> f64 {
        if self.big_blind == 0 {
            0.0
        } else {
            self.big_blind as f64
        }
    }
}

/// The single filter specification shared by every report. An empty `FilterSpec`
/// matches the unfiltered lifetime total.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilterSpec {
    /// Inclusive lower bound on UTC hand time.
    pub from: Option<chrono::DateTime<chrono::Utc>>,
    /// Exclusive upper bound on UTC hand time.
    pub to: Option<chrono::DateTime<chrono::Utc>>,
    /// Inclusive `(small_blind, big_blind)` pairs in minor units.
    pub stakes: Vec<(i64, i64)>,
    /// Table sizes (6 or 9).
    pub table_sizes: Vec<u8>,
    pub positions: Vec<Position>,
    /// Session ids as produced by the sessions module.
    pub sessions: Vec<i64>,
    /// Hero profile ids.
    pub profiles: Vec<i64>,
}

impl FilterSpec {
    pub fn is_empty(&self) -> bool {
        self.from.is_none()
            && self.to.is_none()
            && self.stakes.is_empty()
            && self.table_sizes.is_empty()
            && self.positions.is_empty()
            && self.sessions.is_empty()
            && self.profiles.is_empty()
    }
}

/// A session derived at query time from ordered hero hands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: i64,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: chrono::DateTime<chrono::Utc>,
    pub hands: i64,
    pub net_result: i64,
    pub currency: Currency,
}

/// The grouping dimension for a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroupBy {
    None,
    Day,
    Week,
    Month,
    Stake,
    Position,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_round_trip() {
        for s in ["As", "Td", "2c", "9h", "Kh", "Qs"] {
            let c = Card::parse(s).unwrap();
            assert_eq!(c.to_string().to_ascii_lowercase()[..1], s[..1].to_ascii_lowercase());
        }
        assert_eq!(Card::parse("1s"), None);
        assert_eq!(Card::parse("Ax"), None);
    }

    #[test]
    fn empty_filter_is_portfolio_wide() {
        assert!(FilterSpec::default().is_empty());
    }
}
