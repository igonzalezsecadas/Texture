//! Money is always an integer count of minor units plus an explicit ISO currency
//! code. Floating point money is forbidden anywhere in this crate.

use std::fmt;

/// Currencies this application can represent. v1 assumes a single EUR account,
/// but every money-bearing row carries its currency so the schema stays honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Eur,
}

impl Currency {
    /// ISO 4217 alphabetic code.
    pub const fn code(self) -> &'static str {
        match self {
            Currency::Eur => "EUR",
        }
    }

    /// Number of decimal places of the minor unit.
    pub const fn exponent(self) -> u32 {
        match self {
            Currency::Eur => 2,
        }
    }

    /// The symbol PokerStars writes in hand histories.
    pub const fn symbol(self) -> &'static str {
        match self {
            Currency::Eur => "\u{20ac}",
        }
    }

    pub fn from_code(code: &str) -> Option<Currency> {
        match code.to_ascii_uppercase().as_str() {
            "EUR" => Some(Currency::Eur),
            _ => None,
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// An exact monetary amount, stored as integer minor units (e.g. cents).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct Money {
    pub minor: i64,
    pub currency: Currency,
}

impl Money {
    pub const fn new(minor: i64, currency: Currency) -> Self {
        Money { minor, currency }
    }

    pub const fn zero(currency: Currency) -> Self {
        Money { minor: 0, currency }
    }

    pub fn checked_add(self, other: Money) -> Option<Money> {
        debug_assert_eq!(self.currency, other.currency);
        if self.currency != other.currency {
            return None;
        }
        self.minor.checked_add(other.minor).map(|m| Money::new(m, self.currency))
    }

    pub fn checked_sub(self, other: Money) -> Option<Money> {
        debug_assert_eq!(self.currency, other.currency);
        if self.currency != other.currency {
            return None;
        }
        self.minor.checked_sub(other.minor).map(|m| Money::new(m, self.currency))
    }

    /// Lossy conversion for render-only purposes (chart axes, equity math).
    /// Never use the result to build a stored total.
    pub fn to_major_f64(self) -> f64 {
        self.minor as f64 / 10f64.powi(self.currency.exponent() as i32)
    }

    /// Human-readable amount including the currency symbol, e.g. `-€12.34`.
    pub fn format(self) -> String {
        format_amount(self.minor, self.currency.symbol(), self.currency.exponent())
    }

    /// Human-readable amount without a currency symbol, e.g. `-12.34`.
    pub fn format_plain(self) -> String {
        format_amount(self.minor, "", self.currency.exponent())
    }

    /// Machine-readable decimal for CSV export: no currency symbol and no digit
    /// grouping, so a spreadsheet or script can parse it as a number. Exact,
    /// because the minor units are only divided by the currency exponent.
    pub fn format_machine(self) -> String {
        let negative = self.minor < 0;
        let magnitude = self.minor.unsigned_abs();
        let exponent = self.currency.exponent();
        let sign = if negative { "-" } else { "" };
        if exponent == 0 {
            return format!("{sign}{magnitude}");
        }
        let scale = 10u64.pow(exponent);
        format!(
            "{sign}{}.{:0width$}",
            magnitude / scale,
            magnitude % scale,
            width = exponent as usize
        )
    }

    /// Parse a decimal amount as written in a hand history, e.g. `"12.34"`,
    /// `"1,234.5"`, `"$0.25"`. Returns minor units.
    pub fn parse_minor(text: &str, currency: Currency) -> Option<i64> {
        parse_decimal_minor(text, currency.exponent())
    }
}

fn format_amount(minor: i64, symbol: &str, exponent: u32) -> String {
    let negative = minor < 0;
    let magnitude = minor.unsigned_abs();
    let scale = 10u64.pow(exponent);
    let whole = magnitude / scale;
    let frac = magnitude % scale;
    let sign = if negative { "-" } else { "" };
    let mut out = String::with_capacity(symbol.len() + 24);
    out.push_str(sign);
    out.push_str(symbol);
    out.push_str(&group_thousands(whole));
    if exponent > 0 {
        out.push('.');
        out.push_str(&format!("{:0width$}", frac, width = exponent as usize));
    }
    out
}

fn group_thousands(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut parts = Vec::new();
    while n > 0 {
        parts.push(format!("{:03}", n % 1000));
        n /= 1000;
    }
    let mut out = parts.pop().unwrap_or_default();
    let trimmed = out.trim_start_matches('0');
    out = if trimmed.is_empty() { "0".to_string() } else { trimmed.to_string() };
    while let Some(p) = parts.pop() {
        out.push(',');
        out.push_str(&p);
    }
    out
}

/// Parse a decimal string into minor units, truncating any digits beyond the
/// currency exponent after checking that they are not significant.
pub fn parse_decimal_minor(text: &str, exponent: u32) -> Option<i64> {
    let mut t = text.trim();
    // Strip a leading currency symbol or code, and a leading sign.
    let negative = t.starts_with('-');
    t = t.trim_start_matches(['-', '+']);
    t = t.trim_start_matches(|c: char| !c.is_ascii_digit() && c != '.');
    t = t.trim_end_matches(|c: char| !c.is_ascii_digit());
    if t.is_empty() {
        return None;
    }
    // Remove thousands separators.
    let cleaned: String = t.chars().filter(|c| *c != ',' && *c != '_' && !c.is_whitespace()).collect();
    let (whole_s, frac_s) = match cleaned.split_once('.') {
        Some((w, f)) => (w, f),
        None => (cleaned.as_str(), ""),
    };
    let whole: i64 = if whole_s.is_empty() { 0 } else { whole_s.parse().ok()? };
    let mut frac: i64 = 0;
    let mut seen = 0u32;
    for ch in frac_s.chars() {
        if seen >= exponent {
            break;
        }
        frac = frac * 10 + ch.to_digit(10)? as i64;
        seen += 1;
    }
    while seen < exponent {
        frac *= 10;
        seen += 1;
    }
    let magnitude = whole.checked_mul(10i64.pow(exponent))?.checked_add(frac)?;
    Some(if negative { -magnitude } else { magnitude })
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.format())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_round_trip() {
        assert_eq!(Money::parse_minor("0.25", Currency::Eur), Some(25));
        assert_eq!(Money::parse_minor("\u{20ac}1.00", Currency::Eur), Some(100));
        assert_eq!(Money::parse_minor("\u{20ac}12.34", Currency::Eur), Some(1234));
        assert_eq!(Money::parse_minor("1,234.5", Currency::Eur), Some(123450));
        assert_eq!(Money::parse_minor("-2.50", Currency::Eur), Some(-250));
        assert_eq!(Money::parse_minor("0", Currency::Eur), Some(0));
        assert_eq!(Money::new(1234, Currency::Eur).format(), "\u{20ac}12.34");
        assert_eq!(Money::new(-250, Currency::Eur).format(), "-\u{20ac}2.50");
        assert_eq!(Money::new(123456789, Currency::Eur).format(), "\u{20ac}1,234,567.89");
    }
}
