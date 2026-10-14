//! PokerStars hand-history parser.
//!
//! This is the only place in the crate allowed to know PokerStars' text format.
//! See `fixtures/parser/` for the golden corpus this must satisfy.
//!
//! Format reference (one hand per block):
//!
//! ```text
//! PokerStars Hand #238456789012:  Hold'em No Limit (€0.25/€0.50 EUR) - 2024/01/02 13:45:06 CET [2024/01/02 7:45:06 ET]
//! Table 'Alpha II' 6-max Seat #6 is the button
//! Seat 1: villainAA (€50 in chips)
//! ...
//! villainAA: posts small blind €0.25
//! hero: posts big blind €0.50
//! *** HOLE CARDS ***
//! Dealt to hero [As Kd]
//! ...
//! *** FLOP *** [Ah 7c 2d]
//! ...
//! *** SUMMARY ***
//! Total pot €20.50 | Rake €0.25
//! ```
//!
//! Documented decisions:
//! - `ParsedSeat::shown_cards` holds the hero's dealt hole cards (always known)
//!   plus any cards an opponent revealed with `shows [..]`. This is what the EV
//!   engine needs.
//! - `timestamp_utc` is derived from the BRACKETED Eastern-time stamp, which is
//!   the only unambiguous UTC anchor in the header (US Eastern rules: EST = UTC-5,
//!   EDT = UTC-4). The unbracketed stamp and its timezone token are preserved
//!   verbatim in `local_timestamp` / `tz_token`.
//! - `pot_total` and `rake` come from the summary. `result` is always
//!   `collected - contributed`, and `contributed` excludes returned uncalled bets.

use crate::domain::{
    Action, ActionKind, Card, Currency, Game, ParsedHand, ParsedSeat, Position, Site, Street,
};
use crate::error::ParseError;
use crate::money::{Money, parse_decimal_minor};
use crate::parser::HandParser;
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc, Weekday};
use std::collections::BTreeMap;

pub struct PokerStarsParser;

const HEADER_PREFIX: &str = "PokerStars Hand #";
const HAND_SEPARATOR: &str = "PokerStars Hand #";
const SUMMARY_MARKER: &str = "*** SUMMARY ***";

impl HandParser for PokerStarsParser {
    fn site(&self) -> Site {
        Site::PokerStars
    }

    fn can_parse(&self, text: &str) -> bool {
        text.trim_start_matches('\u{feff}').trim_start().starts_with(HEADER_PREFIX)
    }

    fn parse_hand(&self, raw: &str) -> Result<ParsedHand, ParseError> {
        parse_hand(raw)
    }
}

/// Split a file's text into complete hand blocks, returning `(hands, consumed)`
/// where `consumed` is the byte offset of the start of the trailing incomplete
/// hand (or `text.len()` when every hand is complete).
pub fn split_complete_hands(text: &str) -> (Vec<&str>, usize) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut starts: Vec<usize> = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = text[search_from..].find(HAND_SEPARATOR) {
        let idx = search_from + rel;
        let at_line_start = idx == 0 || text.as_bytes()[idx - 1] == b'\n';
        if at_line_start && !starts.contains(&idx) {
            starts.push(idx);
        }
        search_from = idx + HAND_SEPARATOR.len();
    }
    if starts.is_empty() {
        return (Vec::new(), text.len());
    }

    let mut hands = Vec::new();
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

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn parse_hand(raw: &str) -> Result<ParsedHand, ParseError> {
    Parser::new(raw)?.run()
}

struct Parser<'a> {
    lines: Vec<&'a str>,
    hand_id: String,
    small_blind: i64,
    big_blind: i64,
    table_name: String,
    table_size: u8,
    button_seat: u8,
    timestamp_utc: DateTime<Utc>,
    local_timestamp: String,
    tz_token: String,
    board: Vec<Card>,
    streets_dealt: Vec<Street>,
    seats: BTreeMap<u8, SeatAcc>,
    actions: Vec<Action>,
    street: Street,
    /// Per-seat commitment on the current street, in minor units.
    committed_street: BTreeMap<u8, i64>,
    first_aggression_seen: bool,
    pot_total: Option<i64>,
    rake: Option<i64>,
    jackpot_drop: Option<i64>,
    went_to_showdown: bool,
    summary_board: Option<Vec<Card>>,
}

#[derive(Default, Clone)]
struct SeatAcc {
    seat: u8,
    name: String,
    stack: i64,
    sat_out: bool,
    dealt: Vec<Card>,
    shown: Vec<Card>,
    contributed: i64,
    collected: i64,
}

impl<'a> Parser<'a> {
    fn new(raw: &'a str) -> Result<Parser<'a>, ParseError> {
        let raw = raw.trim_start_matches('\u{feff}');
        let mut lines: Vec<&str> = Vec::new();
        for line in raw.lines() {
            let line = line.trim_end_matches('\r');
            let line = line.trim();
            // Stop at a second hand: the contract is one hand per call.
            if !lines.is_empty() && line.starts_with(HAND_SEPARATOR) {
                break;
            }
            if line.is_empty() && lines.is_empty() {
                continue;
            }
            lines.push(line);
        }
        let header = lines
            .iter()
            .find(|l| l.starts_with(HEADER_PREFIX))
            .ok_or(ParseError::NotPokerStars)?;
        let (hand_id, small_blind, big_blind, timestamp_utc, local_timestamp, tz_token) =
            parse_header(header)?;

        Ok(Parser {
            lines,
            hand_id,
            small_blind,
            big_blind,
            table_name: String::new(),
            table_size: 0,
            button_seat: 0,
            timestamp_utc,
            local_timestamp,
            tz_token,
            board: Vec::new(),
            streets_dealt: Vec::new(),
            seats: BTreeMap::new(),
            actions: Vec::new(),
            street: Street::Preflop,
            committed_street: BTreeMap::new(),
            first_aggression_seen: false,
            pot_total: None,
            rake: None,
            jackpot_drop: None,
            went_to_showdown: false,
            summary_board: None,
        })
    }

    fn run(mut self) -> Result<ParsedHand, ParseError> {
        let mut in_summary = false;
        for line in self.lines.clone() {
            if line.is_empty() {
                continue;
            }
            if line.starts_with("Tournament #") {
                return Err(ParseError::UnsupportedGame("tournament".into()));
            }
            if line == SUMMARY_MARKER {
                in_summary = true;
                continue;
            }
            if in_summary {
                self.parse_summary_line(line)?;
                continue;
            }
            if line.starts_with("*** ") {
                self.parse_street_marker(line)?;
                continue;
            }
            if line.starts_with("Table ") {
                self.parse_table_line(line)?;
                continue;
            }
            if let Some(rest) = line.strip_prefix("Seat ") {
                self.parse_seat_line(rest)?;
                continue;
            }
            if let Some(rest) = line.strip_prefix("Dealt to ") {
                self.parse_dealt(rest)?;
                continue;
            }
            if line.starts_with("Uncalled bet") {
                self.parse_uncalled(line)?;
                continue;
            }
            // Collection lines carry no colon: "hero collected €20.25 from pot".
            if let Some((name, rest)) = line.split_once(" collected ") {
                if let Some(seat) = self.seats.values().find(|s| s.name == name).map(|s| s.seat) {
                    self.parse_collected(seat, &format!("collected {rest}"))?;
                }
                continue;
            }
            // Anything else is either an action or a table notice we ignore.
            if let Some((name, rest)) = split_name_action(line) {
                self.parse_action(name, rest, line)?;
            }
        }

        self.finish()
    }

    fn parse_table_line(&mut self, line: &str) -> Result<(), ParseError> {
        // Table 'Alpha II' 6-max Seat #3 is the button
        let after = line.strip_prefix("Table '").ok_or_else(|| ParseError::malformed("table", line))?;
        let (name, rest) = after
            .split_once("' ")
            .ok_or_else(|| ParseError::malformed("table", line))?;
        self.table_name = name.to_string();
        let max_token = rest
            .split_whitespace()
            .find(|t| t.ends_with("-max"))
            .ok_or_else(|| ParseError::malformed("table size", line))?;
        let size: u8 = max_token
            .trim_end_matches("-max")
            .parse()
            .map_err(|_| ParseError::malformed("table size", max_token))?;
        if size == 0 {
            return Err(ParseError::malformed("table size", line));
        }
        self.table_size = size;
        if let Some(idx) = rest.find("Seat #") {
            let digits: String = rest[idx + "Seat #".len()..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            self.button_seat = digits
                .parse()
                .map_err(|_| ParseError::malformed("button seat", line))?;
        }
        Ok(())
    }

    fn parse_seat_line(&mut self, rest: &str) -> Result<(), ParseError> {
        // "1: villainAA (€50 in chips) is sitting out"
        let (num, after) = rest
            .split_once(": ")
            .ok_or_else(|| ParseError::malformed("seat", rest))?;
        let seat: u8 = num.parse().map_err(|_| ParseError::malformed("seat number", rest))?;
        let (name, amounts) = after
            .split_once(" (")
            .ok_or_else(|| ParseError::malformed("seat stack", rest))?;
        let stack_text = amounts
            .split(" in chips)")
            .next()
            .ok_or_else(|| ParseError::malformed("seat stack", rest))?;
        let stack = parse_money_eur(stack_text)
            .ok_or_else(|| ParseError::malformed("seat stack", stack_text))?;
        let flags = after.to_ascii_lowercase();
        let sat_out = flags.contains("sitting out") || flags.contains("is disconnected");
        let acc = self.seats.entry(seat).or_default();
        acc.seat = seat;
        acc.name = name.to_string();
        acc.stack = stack;
        acc.sat_out = sat_out;
        Ok(())
    }

    fn parse_dealt(&mut self, rest: &str) -> Result<(), ParseError> {
        // "hero [As Kd]"
        let (name, bracket) = rest
            .split_once(" [")
            .ok_or_else(|| ParseError::malformed("hole cards", rest))?;
        let cards = parse_cards(bracket.trim_end_matches(']'));
        if let Some(acc) = self.seats.values_mut().find(|s| s.name == name) {
            acc.dealt = cards.clone();
            if acc.shown.is_empty() {
                acc.shown = cards;
            }
        }
        Ok(())
    }

    fn parse_street_marker(&mut self, line: &str) -> Result<(), ParseError> {
        let upper = line.to_ascii_uppercase();
        if upper.contains("SUMMARY") {
            return Ok(());
        }
        if upper.contains("SHOW DOWN") {
            self.went_to_showdown = true;
            return Ok(());
        }
        let street = if upper.contains("HOLE CARDS") {
            Some(Street::Preflop)
        } else if upper.contains("FLOP") {
            Some(Street::Flop)
        } else if upper.contains("TURN") {
            Some(Street::Turn)
        } else if upper.contains("RIVER") {
            Some(Street::River)
        } else {
            None
        };
        if let Some(street) = street {
            self.street = street;
            // Preflop commitments come from the blinds that were posted before
            // `*** HOLE CARDS ***`, so only the later streets start from zero.
            if street != Street::Preflop {
                self.committed_street.clear();
            }
            self.first_aggression_seen = false;
            if street != Street::Preflop && !self.streets_dealt.contains(&street) {
                self.streets_dealt.push(street);
            }
            // The new cards are in the last bracket on the line.
            if let Some(bracket) = last_bracket(line) {
                let cards = parse_cards(bracket);
                if street == Street::Flop {
                    self.board = cards;
                } else if let Some(card) = cards.last() {
                    self.board.push(*card);
                }
            }
        }
        Ok(())
    }

    fn parse_uncalled(&mut self, line: &str) -> Result<(), ParseError> {
        // Uncalled bet (€12) returned to hero
        let amount = extract_paren_money(line)
            .ok_or_else(|| ParseError::malformed("uncalled bet", line))?;
        let name = line
            .rsplit(" returned to ")
            .next()
            .ok_or_else(|| ParseError::malformed("uncalled bet", line))?;
        let seat = self
            .seats
            .values()
            .find(|s| s.name == name)
            .map(|s| s.seat)
            .ok_or_else(|| ParseError::Inconsistent(format!("uncalled bet to unknown seat {name}")))?;
        if let Some(acc) = self.seats.get_mut(&seat) {
            acc.contributed -= amount;
        }
        let entry = self.committed_street.entry(seat).or_insert(0);
        *entry -= amount;
        self.actions.push(Action {
            street: self.street,
            seat,
            kind: ActionKind::ReturnUncalled,
            amount,
            to_amount: *entry,
            is_first_aggression: false,
        });
        Ok(())
    }

    fn parse_action(&mut self, name: &str, rest: &str, line: &str) -> Result<(), ParseError> {
        let seat = match self.seats.values().find(|s| s.name == name).map(|s| s.seat) {
            Some(seat) => seat,
            // Chat, joins, leaves and notices about players not in the hand.
            None => return Ok(()),
        };
        let rest = rest.trim();
        let lower = rest.to_ascii_lowercase();

        // Actions we record as part of the hand's ledger.
        if lower.starts_with("posts ") {
            let kind = self.parse_post(rest)?;
            let amount = action_amount(kind, rest, line)?;
            self.record_action(seat, kind, amount, None);
            return Ok(());
        }
        if lower == "folds" || lower.starts_with("folds ") {
            self.record_action(seat, ActionKind::Fold, 0, None);
            return Ok(());
        }
        if lower == "checks" {
            self.record_action(seat, ActionKind::Check, 0, None);
            return Ok(());
        }
        if lower.starts_with("calls ") {
            let amount = action_amount(ActionKind::Call, rest, line)?;
            self.record_action(seat, ActionKind::Call, amount, None);
            return Ok(());
        }
        if lower.starts_with("bets ") {
            let amount = action_amount(ActionKind::Bet, rest, line)?;
            self.record_action(seat, ActionKind::Bet, amount, None);
            return Ok(());
        }
        if lower.starts_with("raises ") {
            let amount = action_amount(ActionKind::Raise, rest, line)?;
            let to = raise_total(rest);
            self.record_action(seat, ActionKind::Raise, amount, to);
            return Ok(());
        }
        if lower.starts_with("shows ") {
            self.parse_show(seat, rest)?;
            return Ok(());
        }
        if lower.starts_with("mucks") || lower.starts_with("doesn't show") {
            // Only ever reached at showdown.
            self.went_to_showdown = true;
            return Ok(());
        }
        if lower.starts_with("collected ") || lower.contains(" collected ") {
            self.parse_collected(seat, rest)?;
            return Ok(());
        }
        if lower.contains("sits out") || lower.contains("is sitting out") {
            if let Some(acc) = self.seats.get_mut(&seat) {
                acc.sat_out = true;
            }
            self.record_action(seat, ActionKind::SitOut, 0, None);
            return Ok(());
        }
        if lower.contains("has timed out")
            || lower.contains("is disconnected")
            || lower.contains("has returned")
            || lower.contains("joins the table")
            || lower.contains("leaves the table")
            || lower.contains("said,")
            || lower.contains("sits out")
        {
            return Ok(());
        }
        // Text we cannot identify that carries an amount is a parse error, so it
        // is quarantined rather than silently dropped.
        if rest.contains('\u{20ac}') {
            return Err(ParseError::malformed("action", line));
        }
        Ok(())
    }

    /// Apply an action to the seat's street and total commitments and record it.
    ///
    /// `amount` is the amount as written. For a raise, PokerStars writes the
    /// raise increment over the previous bet level, which is *not* the chips this
    /// seat actually adds when it already has a blind posted; the recorded
    /// [`Action::amount`] is always the chips actually moved, and `to_amount` is
    /// the seat's total commitment on the street after the action.
    fn record_action(&mut self, seat: u8, kind: ActionKind, amount: i64, raise_to: Option<i64>) {
        let entry = self.committed_street.entry(seat).or_insert(0);
        let previous = *entry;
        let (delta, to_amount) = match kind {
            ActionKind::Fold | ActionKind::Check | ActionKind::SitOut => (0, previous),
            ActionKind::Raise => {
                let to = raise_to.unwrap_or(previous + amount);
                (to - previous, to)
            }
            _ => (amount, previous + amount),
        };
        *entry = to_amount;

        if let Some(acc) = self.seats.get_mut(&seat) {
            acc.contributed += delta;
        }
        let is_first_aggression =
            matches!(kind, ActionKind::Bet | ActionKind::Raise) && !self.first_aggression_seen;
        if is_first_aggression {
            self.first_aggression_seen = true;
        }
        self.actions.push(Action {
            street: self.street,
            seat,
            kind,
            amount: delta,
            to_amount,
            is_first_aggression,
        });
    }

    fn parse_post(&mut self, rest: &str) -> Result<ActionKind, ParseError> {
        let lower = rest.to_ascii_lowercase();
        let kind = if lower.contains("small blind") {
            ActionKind::PostSmallBlind
        } else if lower.contains("big blind") {
            ActionKind::PostBigBlind
        } else if lower.contains("ante") {
            ActionKind::PostAnte
        } else if lower.contains("dead") {
            ActionKind::PostDead
        } else {
            return Err(ParseError::malformed("post", rest));
        };
        Ok(kind)
    }

    fn parse_show(&mut self, seat: u8, rest: &str) -> Result<(), ParseError> {
        // "shows [As Kd] (a pair of Aces)"
        if let Some(lb) = rest.find('[') {
            if let Some(rb) = rest[lb..].find(']') {
                let cards = parse_cards(&rest[lb + 1..lb + rb]);
                if let Some(acc) = self.seats.get_mut(&seat) {
                    acc.shown = cards;
                }
                self.went_to_showdown = true;
            }
        }
        Ok(())
    }

    fn parse_collected(&mut self, seat: u8, rest: &str) -> Result<(), ParseError> {
        // "collected €20.25 from pot"
        let after = rest
            .strip_prefix("collected ")
            .or_else(|| rest.split_once(" collected ").map(|(_, a)| a))
            .unwrap_or(rest);
        let amount_text = after.split(" from ").next().unwrap_or(after);
        let amount = parse_money_eur(amount_text)
            .ok_or_else(|| ParseError::malformed("collected amount", after))?;
        if let Some(acc) = self.seats.get_mut(&seat) {
            acc.collected += amount;
        }
        Ok(())
    }

    fn parse_summary_line(&mut self, line: &str) -> Result<(), ParseError> {
        if let Some(v) = amount_after(line, "Total pot") {
            self.pot_total = Some(v);
        }
        if let Some(v) = amount_after(line, "Rake") {
            self.rake = Some(v);
        }
        if let Some(v) = amount_after(line, "Jackpot drop").or_else(|| amount_after(line, "Jackpot")) {
            self.jackpot_drop = Some(v);
        }
        if let Some(bracket) = line.strip_prefix("Board [") {
            self.summary_board = Some(parse_cards(bracket.trim_end_matches(']')));
        }
        Ok(())
    }

    fn finish(mut self) -> Result<ParsedHand, ParseError> {
        if self.table_size == 0 {
            return Err(ParseError::malformed("table", "no table line found"));
        }
        if self.button_seat == 0 {
            return Err(ParseError::malformed("table", "no button seat found"));
        }
        if self.seats.is_empty() {
            return Err(ParseError::Incomplete("no seats found".into()));
        }
        if let Some(board) = self.summary_board.take() {
            if !board.is_empty() {
                self.board = board;
            }
        }
        if self.streets_dealt.is_empty() {
            // Every hand has at least a preflop.
            self.streets_dealt.push(Street::Preflop);
        }

        let positions = assign_positions(self.button_seat, &self.seats);

        let mut contributed_total = 0i64;
        let mut collected_total = 0i64;
        let mut seats = Vec::with_capacity(self.seats.len());
        for (seat, acc) in self.seats.iter() {
            contributed_total += acc.contributed;
            collected_total += acc.collected;
            seats.push(ParsedSeat {
                seat: *seat,
                screen_name: acc.name.clone(),
                starting_stack: Money::new(acc.stack, Currency::Eur),
                position: positions.get(seat).copied().unwrap_or(Position::Mp),
                contributed: acc.contributed,
                result: Money::new(acc.collected - acc.contributed, Currency::Eur),
                shown_cards: acc.shown.clone(),
                sat_out: acc.sat_out,
            });
        }

        let pot_total = self.pot_total.unwrap_or(collected_total);
        let rake = self.rake.unwrap_or(0);
        let jackpot_drop = self.jackpot_drop.unwrap_or(0);

        if rake < 0 || pot_total < 0 {
            return Err(ParseError::Inconsistent("negative pot or rake".into()));
        }
        if rake > pot_total {
            return Err(ParseError::Inconsistent(format!(
                "rake {rake} exceeds pot {pot_total}"
            )));
        }
        // Everything the players put in, minus bets returned uncalled, is the pot.
        // A separately reported jackpot drop may or may not be inside `Total pot`,
        // so both accountings are accepted; anything else is a parse defect.
        if contributed_total != pot_total && contributed_total != pot_total + jackpot_drop {
            return Err(ParseError::Inconsistent(format!(
                "contributions {contributed_total} do not match pot {pot_total} (jackpot {jackpot_drop})"
            )));
        }
        if collected_total > pot_total {
            return Err(ParseError::Inconsistent(format!(
                "collected {collected_total} exceeds pot {pot_total}"
            )));
        }

        Ok(ParsedHand {
            site: Site::PokerStars,
            hand_id: self.hand_id,
            game: Game::NoLimitHoldem,
            currency: Currency::Eur,
            small_blind: Money::new(self.small_blind, Currency::Eur),
            big_blind: Money::new(self.big_blind, Currency::Eur),
            table_name: self.table_name,
            table_size: self.table_size,
            button_seat: self.button_seat,
            timestamp_utc: self.timestamp_utc,
            local_timestamp: self.local_timestamp,
            tz_token: self.tz_token,
            board: self.board,
            seats,
            actions: self.actions,
            pot_total: Money::new(pot_total, Currency::Eur),
            rake: Money::new(rake, Currency::Eur),
            jackpot_drop: Money::new(jackpot_drop, Currency::Eur),
            went_to_showdown: self.went_to_showdown,
            streets_dealt: self.streets_dealt,
        })
    }
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

fn parse_header(
    line: &str,
) -> Result<(String, i64, i64, DateTime<Utc>, String, String), ParseError> {
    let after = line.strip_prefix(HEADER_PREFIX).ok_or(ParseError::NotPokerStars)?;
    let (hand_id, rest) = after
        .split_once(':')
        .ok_or_else(|| ParseError::malformed("header", line))?;
    let hand_id = hand_id.trim().to_string();
    if hand_id.is_empty() || !hand_id.chars().all(|c| c.is_ascii_digit()) {
        return Err(ParseError::malformed("hand id", hand_id.as_str()));
    }

    // "<game> (<stakes> <CODE>) - <local ts> <tz> [<et ts> ET]"
    let open = rest
        .find('(')
        .ok_or_else(|| ParseError::malformed("header stakes", line))?;
    let close = rest
        .find(')')
        .ok_or_else(|| ParseError::malformed("header stakes", line))?;
    if close < open {
        return Err(ParseError::malformed("header stakes", line));
    }
    let game = rest[..open].trim();
    let stakes_group = &rest[open + 1..close];

    if game != "Hold'em No Limit" {
        return Err(ParseError::UnsupportedGame(game.to_string()));
    }

    if stakes_group.contains('$') {
        return Err(ParseError::UnsupportedCurrency("USD".into()));
    }
    if !stakes_group.contains('\u{20ac}') {
        return Err(ParseError::UnsupportedCurrency(stakes_group.to_string()));
    }

    let stakes = stakes_group
        .split_whitespace()
        .next()
        .ok_or_else(|| ParseError::malformed("header stakes", stakes_group))?;
    let (sb_text, bb_text) = stakes
        .split_once('/')
        .ok_or_else(|| ParseError::malformed("header stakes", stakes))?;
    let small_blind = parse_money_eur(sb_text)
        .ok_or_else(|| ParseError::malformed("small blind", sb_text))?;
    let big_blind =
        parse_money_eur(bb_text).ok_or_else(|| ParseError::malformed("big blind", bb_text))?;
    if small_blind <= 0 || big_blind <= 0 {
        return Err(ParseError::malformed("header stakes", stakes));
    }

    // Everything after the stakes group.
    let tail = rest[close + 1..].trim_start_matches([' ', '-']).trim();

    // The bracketed Eastern stamp is the authoritative UTC anchor.
    let bracket_start = tail
        .find('[')
        .ok_or_else(|| ParseError::Incomplete("no bracketed timestamp".into()))?;
    let bracket_end = tail
        .find(']')
        .ok_or_else(|| ParseError::Incomplete("unterminated bracketed timestamp".into()))?;
    let et_text = tail[bracket_start + 1..bracket_end].trim();
    let et_naive = parse_naive_datetime(et_text)
        .ok_or_else(|| ParseError::malformed("timestamp", et_text))?;
    let timestamp_utc = et_to_utc(et_naive);

    // The unbracketed stamp and its timezone token are preserved verbatim.
    let local_part = tail[..bracket_start].trim();
    let mut parts = local_part.split_whitespace();
    let date = parts.next().unwrap_or("").to_string();
    let time = parts.next().unwrap_or("").to_string();
    let tz = parts.next().unwrap_or("").to_string();
    let local_timestamp = if time.is_empty() { date } else { format!("{date} {time}") };
    let tz_token = tz;

    Ok((hand_id, small_blind, big_blind, timestamp_utc, local_timestamp, tz_token))
}

fn parse_naive_datetime(text: &str) -> Option<NaiveDateTime> {
    // "2024/01/02 7:45:06" or "2024/01/02 13:45:06"
    let mut parts = text.split_whitespace();
    let date_s = parts.next()?;
    let time_s = parts.next()?;
    let mut d = date_s.split('/');
    let year: i32 = d.next()?.parse().ok()?;
    let month: u32 = d.next()?.parse().ok()?;
    let day: u32 = d.next()?.parse().ok()?;
    let mut t = time_s.split(':');
    let hour: u32 = t.next()?.trim().parse().ok()?;
    let minute: u32 = t.next()?.parse().ok()?;
    let second: u32 = t.next().unwrap_or("0").parse().ok()?;
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    date.and_hms_opt(hour, minute, second)
}

/// The US Eastern offset in seconds for a local wall-clock time.
fn et_offset_secs(naive: NaiveDateTime) -> i64 {
    let year = naive.year();
    let dst_start = nth_weekday_of_month(year, 3, Weekday::Sun, 2)
        .and_hms_opt(2, 0, 0)
        .expect("valid DST start");
    let dst_end = nth_weekday_of_month(year, 11, Weekday::Sun, 1)
        .and_hms_opt(2, 0, 0)
        .expect("valid DST end");
    if naive >= dst_start && naive < dst_end {
        -4 * 3600 // EDT
    } else {
        -5 * 3600 // EST
    }
}

fn nth_weekday_of_month(year: i32, month: u32, weekday: Weekday, nth: u32) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
    let offset = (7 + weekday.num_days_from_monday() - first.weekday().num_days_from_monday()) % 7;
    first + Duration::days((offset + 7 * (nth - 1)) as i64)
}

fn et_to_utc(naive: NaiveDateTime) -> DateTime<Utc> {
    let offset = et_offset_secs(naive);
    Utc.from_utc_datetime(&(naive - Duration::seconds(offset)))
}

// ---------------------------------------------------------------------------
// Position assignment
// ---------------------------------------------------------------------------

/// Assign a [`Position`] to each occupied seat given the button seat.
///
/// Seats act clockwise in seat-number order. Heads-up is special: the button is
/// the small blind, so there is no separate SB label.
fn assign_positions(button_seat: u8, seats: &BTreeMap<u8, SeatAcc>) -> BTreeMap<u8, Position> {
    let mut out = BTreeMap::new();
    let ordered: Vec<u8> = seats.keys().copied().collect();
    let n = ordered.len();
    if n == 0 {
        return out;
    }
    let Some(bi) = ordered.iter().position(|s| *s == button_seat) else {
        // Button seat not occupied: fall back to MP rather than failing the hand.
        for s in ordered {
            out.insert(s, Position::Mp);
        }
        return out;
    };
    if n == 1 {
        out.insert(ordered[bi], Position::Btn);
        return out;
    }
    if n == 2 {
        let other = ordered[(bi + 1) % n];
        out.insert(ordered[bi], Position::Btn);
        out.insert(other, Position::Bb);
        return out;
    }

    let sb = ordered[(bi + 1) % n];
    let bb = ordered[(bi + 2) % n];
    out.insert(ordered[bi], Position::Btn);
    out.insert(sb, Position::Sb);
    out.insert(bb, Position::Bb);

    // The remaining seats, in acting order, run from the seat after the big
    // blind around to the seat before the button.
    let middle = Position::middle_sequence(n);
    let mut idx = (bi + 3) % n;
    let mut mi = 0usize;
    while idx != bi {
        let position = middle.get(mi).copied().unwrap_or(Position::Mp);
        out.insert(ordered[idx], position);
        mi += 1;
        idx = (idx + 1) % n;
    }
    out
}

// ---------------------------------------------------------------------------
// Small text helpers
// ---------------------------------------------------------------------------

fn parse_money_eur(text: &str) -> Option<i64> {
    parse_decimal_minor(text, Currency::Eur.exponent())
}

/// Find `key` in a line and parse the money amount that follows it.
fn amount_after(line: &str, key: &str) -> Option<i64> {
    let idx = line.find(key)?;
    let rest = &line[idx + key.len()..];
    // Take the next token that looks like an amount.
    for token in rest.split_whitespace() {
        let cleaned: String = token
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',' || *c == '\u{20ac}')
            .collect();
        if cleaned.chars().any(|c| c.is_ascii_digit()) {
            return parse_money_eur(&cleaned);
        }
        if token == "|" {
            return None;
        }
    }
    None
}

fn extract_paren_money(line: &str) -> Option<i64> {
    let start = line.find('(')?;
    let end = line[start..].find(')')?;
    parse_money_eur(&line[start + 1..start + end])
}

fn last_bracket(line: &str) -> Option<&str> {
    let start = line.rfind('[')?;
    let end = line[start..].find(']')?;
    Some(&line[start + 1..start + end])
}

fn parse_cards(text: &str) -> Vec<Card> {
    text.split_whitespace().filter_map(Card::parse).collect()
}

/// Split "name: action" respecting that screen names may contain spaces.
fn split_name_action(line: &str) -> Option<(&str, &str)> {
    let idx = line.find(": ")?;
    Some((&line[..idx], &line[idx + 2..]))
}

/// The amount as written for an action, in minor units.
fn action_amount(kind: ActionKind, rest: &str, line: &str) -> Result<i64, ParseError> {
    let amount = match kind {
        ActionKind::Fold | ActionKind::Check | ActionKind::SitOut => 0,
        ActionKind::PostSmallBlind
        | ActionKind::PostBigBlind
        | ActionKind::PostAnte
        | ActionKind::PostDead => {
            // "posts small blind €0.25" / "posts big blind €0.50 + dead blind"
            rest.split_whitespace()
                .find_map(parse_money_eur)
                .ok_or_else(|| ParseError::malformed("post amount", line))?
        }
        ActionKind::Call => {
            // "calls €0.50"
            rest.split_whitespace()
                .find_map(parse_money_eur)
                .ok_or_else(|| ParseError::malformed("call amount", line))?
        }
        ActionKind::Bet => {
            // "bets €1.50"
            rest.split_whitespace()
                .find_map(parse_money_eur)
                .ok_or_else(|| ParseError::malformed("bet amount", line))?
        }
        ActionKind::Raise => {
            // "raises €1 to €1.50": the written amount is the raise increment.
            let after = rest
                .strip_prefix("raises ")
                .ok_or_else(|| ParseError::malformed("raise", line))?;
            let head = after.split(" to ").next().unwrap_or(after);
            head.split_whitespace()
                .find_map(parse_money_eur)
                .ok_or_else(|| ParseError::malformed("raise amount", line))?
        }
        ActionKind::AllIn | ActionKind::ReturnUncalled => 0,
    };
    Ok(amount)
}

/// Total commitment on the street after a raise: the `to €Y` amount.
fn raise_total(rest: &str) -> Option<i64> {
    let after = rest.split(" to ").nth(1)?;
    after.split_whitespace().find_map(parse_money_eur)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HAND: &str = r#"PokerStars Hand #238456789012:  Hold'em No Limit (€0.25/€0.50 EUR) - 2024/01/02 13:45:06 CET [2024/01/02 7:45:06 ET]
Table 'Alpha II' 6-max Seat #6 is the button
Seat 1: villainAA (€50 in chips)
Seat 2: hero (€52.75 in chips)
Seat 3: villainBB (€48.30 in chips)
Seat 4: villainCC (€51 in chips)
Seat 5: villainDD (€50 in chips)
Seat 6: villainEE (€49.50 in chips)
villainAA: posts small blind €0.25
hero: posts big blind €0.50
*** HOLE CARDS ***
Dealt to hero [As Kd]
villainBB: folds
villainCC: raises €1 to €1.50
villainDD: folds
villainEE: folds
villainAA: folds
hero: raises €3.50 to €5
villainCC: calls €3.50
*** FLOP *** [Ah 7c 2d]
hero: bets €5
villainCC: calls €5
*** TURN *** [Ah 7c 2d] [Ts]
hero: bets €12
villainCC: folds
Uncalled bet (€12) returned to hero
hero collected €20 from pot
*** SUMMARY ***
Total pot €20.25 | Rake €0.25
Board [Ah 7c 2d Ts]
Seat 1: villainAA (small blind) folded before Flop
Seat 2: hero (big blind) collected (€20)
Seat 3: villainBB (UTG) folded before Flop
Seat 6: villainEE (button) folded before Flop
"#;

    #[test]
    fn parses_a_basic_hand() {
        let hand = parse_hand(HAND).expect("parses");
        assert_eq!(hand.hand_id, "238456789012");
        assert_eq!(hand.small_blind.minor, 25);
        assert_eq!(hand.big_blind.minor, 50);
        assert_eq!(hand.table_name, "Alpha II");
        assert_eq!(hand.table_size, 6);
        assert_eq!(hand.button_seat, 6);
        assert_eq!(hand.seats.len(), 6);
        assert_eq!(hand.board.len(), 4);
        assert_eq!(hand.pot_total.minor, 2025);
        assert_eq!(hand.rake.minor, 25);
        assert_eq!(hand.local_timestamp, "2024/01/02 13:45:06");
        assert_eq!(hand.tz_token, "CET");
        // 2024-01-02 07:45:06 EST == 12:45:06 UTC
        assert_eq!(hand.timestamp_utc.to_string(), "2024-01-02 12:45:06 UTC");
        assert_eq!(hand.streets_dealt, vec![Street::Flop, Street::Turn]);
    }

    #[test]
    fn hero_cards_and_positions() {
        let hand = parse_hand(HAND).expect("parses");
        let hero = hand.seat_by_name("hero").expect("hero seat");
        assert_eq!(hero.shown_cards.len(), 2);
        assert_eq!(hero.position, Position::Bb);
        // €0.50 blind + €4.50 to complete the raise to €5 + €5 flop bet. The €12
        // turn bet was returned uncalled, so it contributes nothing.
        assert_eq!(hero.contributed, 1000);
        assert_eq!(hero.result.minor, 1000);

        let positions: Vec<(&str, Position)> = hand
            .seats
            .iter()
            .map(|s| (s.screen_name.as_str(), s.position))
            .collect();
        assert!(positions.contains(&("villainEE", Position::Btn)));
        assert!(positions.contains(&("villainAA", Position::Sb)));
        assert!(positions.contains(&("hero", Position::Bb)));
        assert!(positions.contains(&("villainBB", Position::Utg)));
        assert!(positions.contains(&("villainCC", Position::Mp)));
        assert!(positions.contains(&("villainDD", Position::Co)));
    }

    #[test]
    fn contributions_match_the_pot() {
        let hand = parse_hand(HAND).expect("parses");
        let contributed: i64 = hand.seats.iter().map(|s| s.contributed).sum();
        // The uncalled turn bet was returned, so contributions equal the pot.
        assert_eq!(contributed, hand.pot_total.minor);
    }

    #[test]
    fn split_finds_complete_hands_only() {
        let complete = format!("{HAND}\n{HAND}");
        let (hands, consumed) = split_complete_hands(&complete);
        assert_eq!(hands.len(), 2);
        assert_eq!(consumed, complete.len());

        let mut truncated = HAND.to_string();
        truncated.push_str("\nPokerStars Hand #999:  Hold'em No Limit (€0.25/€0.50 EUR) - 2024/01/02 13:45:06 CET [2024/01/02 7:45:06 ET]\nTable 'Beta' 6-max Seat #1 is the button\n");
        let (hands, consumed) = split_complete_hands(&truncated);
        assert_eq!(hands.len(), 1);
        assert!(consumed < truncated.len());
        assert!(truncated[consumed..].starts_with("PokerStars Hand #999"));
    }

    #[test]
    fn rejects_non_eur_and_non_holdem() {
        let usd = HAND.replace("€0.25/€0.50 EUR", "$0.25/$0.50 USD");
        assert!(matches!(parse_hand(&usd), Err(ParseError::UnsupportedCurrency(_))));
        let omaha = HAND.replace("Hold'em No Limit", "Omaha Pot Limit");
        assert!(matches!(parse_hand(&omaha), Err(ParseError::UnsupportedGame(_))));
    }

    #[test]
    fn positions_are_complete_for_every_table_size() {
        for n in 2..=9usize {
            let seq = Position::middle_sequence(n);
            let expected_middle = n.saturating_sub(3);
            assert_eq!(seq.len(), expected_middle, "table size {n}");
            // No label may repeat within a table.
            let mut seen = std::collections::BTreeSet::new();
            for p in seq {
                assert!(seen.insert(*p), "duplicate label at table size {n}");
            }
        }
    }
}
