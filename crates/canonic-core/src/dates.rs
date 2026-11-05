//! Dates. The `parse_date` rules of language 1, and the language 2 `format_date` and
//! `parse_date` formats. `transforms.rs` dispatches here. Nothing here panics: input a function
//! cannot read gives `None`.

use crate::transforms::{as_f64, js_trim};
use crate::value::Value;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value as J;

/// The `n` ASCII digits at `at` as a number. `None` if any is missing or not a digit.
fn digits_at(s: &[u8], at: usize, n: usize) -> Option<u32> {
    let d = s.get(at..at + n)?;
    d.iter().try_fold(0u32, |v, c| {
        c.is_ascii_digit().then(|| v * 10 + u32::from(c - b'0'))
    })
}

fn date_field(s: &[u8], at: usize) -> Option<u32> {
    if s.get(at) == Some(&b'-') {
        digits_at(s, at + 1, 2)
    } else {
        None
    }
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        2 if y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400)) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Accepts YYYY, YYYY-MM and YYYY-MM-DD after a trim. Anything after a T or space at 10 is dropped.
pub(crate) fn date_from_string(v: &str) -> Option<&str> {
    let s = js_trim(v);
    let b = s.as_bytes();
    let end = if b.len() > 10 && matches!(b[10], b'T' | b' ') {
        10
    } else {
        b.len()
    };
    if end != 4 && end != 7 && end != 10 {
        return None;
    }
    let y = digits_at(b, 0, 4)?;
    let m = if end > 4 { date_field(b, 4)? } else { 1 };
    let d = if end > 7 { date_field(b, 7)? } else { 1 };
    ((1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)).then(|| &s[..end])
}

/// Epoch seconds below 1e11, else milliseconds, as the UTC day. Providers send 0 or negatives for
/// "unknown". Past year 9999 the number was microseconds or an id.
pub(crate) fn date_from_epoch(v: f64) -> Option<String> {
    let ms = epoch_ms(v)?;
    let (y, m, d) = civil_from_days((ms / 86_400_000.0).floor() as i64);
    // 1970 to 9999, so four digits. Written by hand because `format!` took most of this function's time.
    let digit = |n: i64, p: i64| char::from(b'0' + (n / p % 10) as u8);
    let mut out = String::with_capacity(10);
    out.extend([1000, 100, 10, 1].map(|p| digit(y, p)));
    for n in [m, d] {
        out.push('-');
        out.extend([10, 1].map(|p| digit(n, p)));
    }
    Some(out)
}

/// The milliseconds since 1970 that `date_from_epoch` reads in `v`, whole and below year 10000.
fn epoch_ms(v: f64) -> Option<f64> {
    const YEAR_10000_MS: f64 = 253_402_300_800_000.0;
    if v.fract() != 0.0 || v <= 0.0 {
        return None;
    }
    let ms = if v < 1e11 { v * 1000.0 } else { v };
    (ms < YEAR_10000_MS).then_some(ms)
}

/// Howard Hinnant's days-since-1970 to proleptic Gregorian (year, month, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Hinnant's inverse: proleptic Gregorian (year, month, day) to days since 1970.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// The output of `format_date`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DateOut {
    Date,
    Datetime,
    DatetimeMs,
    EpochS,
    EpochMs,
}

const DATE_OUTS: [(&str, DateOut); 5] = [
    ("date", DateOut::Date),
    ("datetime", DateOut::Datetime),
    ("datetime_ms", DateOut::DatetimeMs),
    ("epoch_s", DateOut::EpochS),
    ("epoch_ms", DateOut::EpochMs),
];

impl DateOut {
    pub(crate) fn new(s: &str) -> Option<DateOut> {
        DATE_OUTS.iter().find(|(n, _)| *n == s).map(|(_, d)| *d)
    }

    pub(crate) fn name(self) -> &'static str {
        DATE_OUTS
            .iter()
            .find(|(_, d)| *d == self)
            .map_or("", |(n, _)| n)
    }
}

const DAY_MS: i64 = 86_400_000;

/// `v` as `out`. `v` is what `parse_date` reads, or an ISO datetime, which is converted to UTC.
/// A partial date is day 1, and a date alone is midnight UTC. Outside years 0 to 9999 is `None`.
pub(crate) fn format_date(v: &Value, out: DateOut) -> Option<Value<'static>> {
    let ms = match v {
        Value::Str(s) => instant(s)?,
        v => epoch_ms(as_f64(v)?)? as i64,
    };
    let (y, m, d) = civil_from_days(ms.div_euclid(DAY_MS));
    if !(0..=9999).contains(&y) {
        return None;
    }
    let t = ms.rem_euclid(DAY_MS);
    let (h, mi, s, frac) = (t / 3_600_000, t / 60_000 % 60, t / 1000 % 60, t % 1000);
    let text = match out {
        DateOut::EpochS => return Some(Value::Int(ms.div_euclid(1000))),
        DateOut::EpochMs => return Some(Value::Int(ms)),
        DateOut::Date => format!("{y:04}-{m:02}-{d:02}"),
        DateOut::Datetime => format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"),
        DateOut::DatetimeMs => format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{frac:03}Z"),
    };
    Some(Value::Str(text.into()))
}

/// Milliseconds since 1970 for a string `date_from_string` accepts. Text after the T or space must
/// read as a time, else `None`, never a guessed midnight.
fn instant(s: &str) -> Option<i64> {
    let b = date_from_string(s)?.as_bytes();
    let field = |at: usize| digits_at(b, at, 2).map_or(1, i64::from);
    let y = i64::from(digits_at(b, 0, 4)?);
    let days = days_from_civil(y, field(5), field(8));
    let time = match js_trim(s).as_bytes().get(11..) {
        Some(t) => time_of_day(t)?,
        None => 0,
    };
    Some(days * DAY_MS + time)
}

/// `HH:MM`, optional `:SS` and `.fraction`, then `Z`, `±HH:MM`, `±HHMM` or nothing (UTC), as
/// milliseconds after midnight UTC. Can be negative or past a day, which moves the date.
fn time_of_day(b: &[u8]) -> Option<i64> {
    let two = |at: usize| digits_at(b, at, 2).map(i64::from);
    let (h, mi) = (two(0)?, (b.get(2) == Some(&b':')).then(|| two(3))??);
    let (mut s, mut frac, mut i) = (0, 0, 5);
    if b.get(i) == Some(&b':') {
        s = two(i + 1)?;
        i += 3;
        if b.get(i) == Some(&b'.') {
            let n = b[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
            if n == 0 {
                return None;
            }
            // Milliseconds: the first three digits, padded.
            frac = (0..3).fold(0, |f, k| {
                f * 10
                    + b.get(i + 1 + k)
                        .filter(|_| k < n)
                        .map_or(0, |c| i64::from(c - b'0'))
            });
            i += 1 + n;
        }
    }
    let offset = match &b[i..] {
        [] | [b'Z' | b'z'] => 0,
        [sign @ (b'+' | b'-'), r @ ..] => {
            let (oh, om) = match r {
                [_, _, b':', _, _] => (digits_at(r, 0, 2)?, digits_at(r, 3, 2)?),
                [_, _, _, _] => (digits_at(r, 0, 2)?, digits_at(r, 2, 2)?),
                _ => return None,
            };
            if oh > 23 || om > 59 {
                return None;
            }
            let minutes = i64::from(oh * 60 + om);
            if *sign == b'-' { -minutes } else { minutes }
        }
        _ => return None,
    };
    (h <= 23 && mi <= 59 && s <= 59)
        .then(|| ((h * 60 + mi) * 60 + s) * 1000 + frac - offset * 60_000)
}

/// One piece of a `parse_date` format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    Year,
    Month,
    MonthAbbr,
    MonthName,
    Day,
    Text(char),
}

/// A `parse_date` format such as `%B %Y`, checked when the plan is built or loaded. A plan holds
/// its text.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    text: String,
    tokens: Vec<Token>,
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

impl Layout {
    /// One `%Y`, at most one month (`%m`, `%b` or `%B`), and a `%d` only with a month.
    pub(crate) fn new(text: &str) -> Option<Layout> {
        let mut tokens = Vec::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            tokens.push(match c {
                '%' => match chars.next()? {
                    'Y' => Token::Year,
                    'm' => Token::Month,
                    'b' => Token::MonthAbbr,
                    'B' => Token::MonthName,
                    'd' => Token::Day,
                    _ => return None,
                },
                c => Token::Text(c),
            });
        }
        let count = |f: fn(&Token) -> bool| tokens.iter().filter(|t| f(t)).count();
        let years = count(|t| *t == Token::Year);
        let months = count(|t| matches!(t, Token::Month | Token::MonthAbbr | Token::MonthName));
        let days = count(|t| *t == Token::Day);
        let text = text.to_string();
        (years == 1 && months <= 1 && days <= months).then_some(Layout { text, tokens })
    }

    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The date `s` holds in this format, as `YYYY`, `YYYY-MM` or `YYYY-MM-DD`. Month names are
    /// English, in any case. `%m` and `%d` take one or two digits.
    fn read(&self, mut s: &str) -> Option<String> {
        let (mut y, mut m, mut d) = (0, None, None);
        for t in &self.tokens {
            let n;
            (n, s) = match t {
                Token::Year => digits(s, 4, 4)?,
                Token::Month => digits(s, 1, 2)?,
                Token::Day => digits(s, 1, 2)?,
                Token::MonthAbbr => month(s, |name| &name[..3])?,
                Token::MonthName => month(s, |name| name)?,
                Token::Text(c) => (0, s.strip_prefix(*c)?),
            };
            match t {
                Token::Year => y = n,
                Token::Day => d = Some(n),
                Token::Text(_) => {}
                _ => m = Some(n),
            }
        }
        let ok = s.is_empty()
            && m.is_none_or(|m| (1..=12).contains(&m))
            && d.is_none_or(|d| d >= 1 && d <= days_in_month(y, m.unwrap_or(1)));
        if !ok {
            return None;
        }
        Some(match (m, d) {
            (Some(m), Some(d)) => format!("{y:04}-{m:02}-{d:02}"),
            (Some(m), None) => format!("{y:04}-{m:02}"),
            _ => format!("{y:04}"),
        })
    }
}

/// `min` to `max` ASCII digits at the start of `s`, as a number, and the rest of `s`.
fn digits(s: &str, min: usize, max: usize) -> Option<(u32, &str)> {
    let n = s.bytes().take(max).take_while(u8::is_ascii_digit).count();
    (n >= min).then(|| (s[..n].parse().unwrap_or(0), &s[n..]))
}

/// The month whose `form` starts `s`, in any case, as 1 to 12, and the rest of `s`.
fn month(s: &str, form: fn(&'static str) -> &'static str) -> Option<(u32, &str)> {
    MONTHS.iter().zip(1..).find_map(|(name, i)| {
        let name = form(name);
        let head = s.get(..name.len())?;
        head.eq_ignore_ascii_case(name)
            .then(|| (i, &s[name.len()..]))
    })
}

/// The date the first format that fits reads in trimmed `v`. `None` when none fits.
pub(crate) fn read_formats(layouts: &[Layout], v: &Value) -> Option<String> {
    let Value::Str(s) = v else {
        return None;
    };
    let s = js_trim(s);
    layouts.iter().find_map(|l| l.read(s))
}

/// A plan's `parse_date` formats, held to the rule `formats_arg` checks: not empty.
pub(crate) fn non_empty<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Layout>, D::Error> {
    let list = Vec::<Layout>::deserialize(d)?;
    if list.is_empty() {
        return Err(serde::de::Error::custom("parse_date formats are empty"));
    }
    Ok(list)
}

/// `{"formats": [...]}`, a non-empty list of valid formats.
pub(crate) fn formats_arg(arg: Option<&J>) -> Option<Vec<Layout>> {
    let o = crate::strings::object(arg, &["formats"], &[])?;
    let list = o["formats"].as_array().filter(|l| !l.is_empty())?;
    list.iter().map(|f| Layout::new(f.as_str()?)).collect()
}

impl Serialize for Layout {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for Layout {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Layout, D::Error> {
        let s = String::deserialize(d)?;
        Layout::new(&s)
            .ok_or_else(|| serde::de::Error::custom(format!("invalid date format {s:?}")))
    }
}
