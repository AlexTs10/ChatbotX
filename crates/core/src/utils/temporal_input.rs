//! `packages/utils/src/temporal-input`: the loose parser for spreadsheet
//! cells (`parseLooseTemporalValue`) and the storage entry point
//! (`normalizeTemporalValueForStorage`). date-fns `parse` is emulated for
//! the exact tokens the format list uses (en-US locale); a naive result is
//! wall-clock fields (the process zone is UTC).

use std::sync::OnceLock;

use regex::Regex;

use super::datetime::{
    TemporalCustomFieldType, TemporalInputParsing, has_explicit_offset,
    normalize_temporal_custom_field_value, resolve_filter_timezone,
};
use super::tz::{self, DateArg};
use crate::js;

/// A parsed wall clock (date-fns `Date` with local getters).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NaiveDateTime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millisecond: u32,
}

impl NaiveDateTime {
    /// The wall clock as ms-as-if-UTC.
    pub fn as_utc_ms(&self) -> i64 {
        js::date_utc(
            self.year,
            i64::from(self.month) - 1,
            i64::from(self.day),
            i64::from(self.hour),
            i64::from(self.minute),
            i64::from(self.second),
            i64::from(self.millisecond),
        )
    }

    fn fields(&self) -> js::UtcFields {
        js::utc_fields(self.as_utc_ms())
    }
}

/// `REFERENCE_DATE`: `new Date(2000, 0, 1)`.
const REFERENCE: NaiveDateTime = NaiveDateTime {
    year: 2000,
    month: 1,
    day: 1,
    hour: 0,
    minute: 0,
    second: 0,
    millisecond: 0,
};

/// `TIME_PATTERNS`, most specific first.
pub const TIME_PATTERNS: &[&str] = &[
    "H:mm:ss.SSS",
    "H:mm:ss",
    "H:mm",
    "h:mm:ss a",
    "h:mm a",
    "h a",
];

const DATE_SHAPES: &[(&str, &[&str])] = &[
    ("yyyy-MM-dd", &["'T'", " "]),
    ("d/M/yyyy", &[" "]),
    ("d-M-yyyy", &[" "]),
    ("d.M.yyyy", &[" "]),
    ("M/d/yyyy", &[" "]),
    ("d MMM yyyy", &[" "]),
    ("d MMMM yyyy", &[" "]),
    ("MMM d, yyyy", &[" "]),
    ("MMMM d, yyyy", &[" "]),
];

const DATE_ONLY_PATTERNS: &[&str] = &["yyyyMMdd"];

/// `CANONICAL_INPUT_FORMATS`.
pub fn canonical_input_formats() -> &'static [String] {
    static FORMATS: OnceLock<Vec<String>> = OnceLock::new();
    FORMATS.get_or_init(|| {
        let mut out = Vec::new();
        for (pattern, separators) in DATE_SHAPES {
            for separator in *separators {
                for time in TIME_PATTERNS {
                    out.push(format!("{pattern}{separator}{time}"));
                }
            }
            out.push((*pattern).to_owned());
        }
        out.extend(DATE_ONLY_PATTERNS.iter().map(|p| (*p).to_owned()));
        out
    })
}

enum Token {
    Field(char, usize),
    Literal(String),
}

fn tokenize(pattern: &str) -> Vec<Token> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            let mut j = i + 1;
            let mut text = String::new();
            while j < chars.len() && chars[j] != '\'' {
                text.push(chars[j]);
                j += 1;
            }
            tokens.push(Token::Literal(text));
            i = j + 1;
        } else if c.is_ascii_alphabetic() {
            let mut n = 1;
            while i + n < chars.len() && chars[i + n] == c {
                n += 1;
            }
            tokens.push(Token::Field(c, n));
            i += n;
        } else {
            tokens.push(Token::Literal(c.to_string()));
            i += 1;
        }
    }
    tokens
}

fn take_numeric<'a>(text: &'a str, pattern: &Regex) -> Option<(i64, &'a str)> {
    let m = pattern.find(text)?;
    Some((m.as_str().parse().ok()?, &text[m.end()..]))
}

fn numeric_re(key: &str) -> &'static Regex {
    static RES: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    let all = RES.get_or_init(|| {
        [
            ("month", "^(1[0-2]|0?[0-9])"),
            ("date", "^(3[0-1]|[0-2]?[0-9])"),
            ("hour23h", "^(2[0-3]|[0-1]?[0-9])"),
            ("hour12h", "^(1[0-2]|0?[0-9])"),
            ("1", "^[0-9]"),
            ("2", "^[0-9]{1,2}"),
            ("3", "^[0-9]{1,3}"),
            ("4", "^[0-9]{1,4}"),
        ]
        .into_iter()
        .map(|(k, p)| (k, Regex::new(p).expect("valid regex")))
        .collect()
    });
    &all.iter()
        .find(|(k, _)| *k == key)
        .expect("known pattern")
        .1
}

fn n_digits(n: usize) -> &'static Regex {
    numeric_re(match n {
        1 => "1",
        2 => "2",
        3 => "3",
        _ => "4",
    })
}

const MONTHS_ABBREVIATED: &str = "^(?i)(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)";
const MONTHS_WIDE: &str =
    "^(?i)(january|february|march|april|may|june|july|august|september|october|november|december)";
const MONTHS_NARROW: &str = "^(?i)[jfmasond]";

fn month_name(text: &str, width: &str) -> Option<(i64, usize)> {
    static RES: OnceLock<[Regex; 3]> = OnceLock::new();
    let [abbreviated, wide, narrow] = RES.get_or_init(|| {
        [
            Regex::new(MONTHS_ABBREVIATED).expect("re"),
            Regex::new(MONTHS_WIDE).expect("re"),
            Regex::new(MONTHS_NARROW).expect("re"),
        ]
    });
    let (re, narrow_parse) = match width {
        "abbreviated" => (abbreviated, false),
        "wide" => (wide, false),
        _ => (narrow, true),
    };
    let m = re.find(text)?;
    let lower = m.as_str().to_lowercase();
    let any = [
        "ja", "f", "mar", "ap", "may", "jun", "jul", "au", "s", "o", "n", "d",
    ];
    let narrow_list = ["j", "f", "m", "a", "m", "j", "j", "a", "s", "o", "n", "d"];
    let list: &[&str] = if narrow_parse { &narrow_list } else { &any };
    let index = list.iter().position(|p| lower.starts_with(p))?;
    Some((index as i64, m.end()))
}

/// en-US `matchDayPeriodPatterns.any`, then `parseDayPeriodPatterns.any`:
/// the hour the period sets.
fn day_period(text: &str) -> Option<(u32, usize)> {
    static ANY: OnceLock<Regex> = OnceLock::new();
    static NARROW: OnceLock<Regex> = OnceLock::new();
    let any = ANY.get_or_init(|| {
        Regex::new(
            r"^(?i)([ap]\.?\s?m\.?|midnight|noon|(in the|at) (morning|afternoon|evening|night))",
        )
        .expect("re")
    });
    let narrow = NARROW.get_or_init(|| {
        Regex::new(r"^(?i)(a|p|mi|n|(in the|at) (morning|afternoon|evening|night))").expect("re")
    });
    let m = any.find(text).or_else(|| narrow.find(text))?;
    let lower = m.as_str().to_lowercase();
    let hour = if lower.starts_with('a') {
        0
    } else if lower.starts_with('p') {
        12
    } else if lower.starts_with("mi") {
        0
    } else if lower.starts_with("no") {
        12
    } else if lower.contains("morning") {
        4
    } else if lower.contains("afternoon") {
        12
    } else if lower.contains("evening") {
        17
    } else if lower.contains("night") {
        0
    } else {
        return None;
    };
    Some((hour, m.end()))
}

/// date-fns `parse(value, pattern, REFERENCE_DATE)`; `None` is an Invalid
/// Date.
pub fn date_fns_parse(value: &str, pattern: &str) -> Option<NaiveDateTime> {
    let mut rest = value;
    let mut year = None;
    let mut month = None;
    let mut day = None;
    let mut day_period_hour = None;
    let mut hour24 = None;
    let mut hour12 = None;
    let mut minute = None;
    let mut second = None;
    let mut fraction = None;
    for token in tokenize(pattern) {
        match token {
            Token::Literal(text) => rest = rest.strip_prefix(text.as_str())?,
            Token::Field(c, n) => match c {
                'y' => {
                    let (v, r) = take_numeric(rest, n_digits(if n == 1 { 4 } else { n }))?;
                    year = Some(v);
                    rest = r;
                }
                'M' if n >= 3 => {
                    let found = if n == 3 {
                        month_name(rest, "abbreviated").or_else(|| month_name(rest, "narrow"))
                    } else {
                        month_name(rest, "wide")
                            .or_else(|| month_name(rest, "abbreviated"))
                            .or_else(|| month_name(rest, "narrow"))
                    };
                    let (v, end) = found?;
                    month = Some(v);
                    rest = &rest[end..];
                }
                'M' => {
                    let re = if n == 1 {
                        numeric_re("month")
                    } else {
                        n_digits(2)
                    };
                    let (v, r) = take_numeric(rest, re)?;
                    month = Some(v - 1);
                    rest = r;
                }
                'd' => {
                    let re = if n == 1 {
                        numeric_re("date")
                    } else {
                        n_digits(n)
                    };
                    let (v, r) = take_numeric(rest, re)?;
                    day = Some(v);
                    rest = r;
                }
                'H' => {
                    let re = if n == 1 {
                        numeric_re("hour23h")
                    } else {
                        n_digits(n)
                    };
                    let (v, r) = take_numeric(rest, re)?;
                    hour24 = Some(v);
                    rest = r;
                }
                'h' => {
                    let re = if n == 1 {
                        numeric_re("hour12h")
                    } else {
                        n_digits(n)
                    };
                    let (v, r) = take_numeric(rest, re)?;
                    hour12 = Some(v);
                    rest = r;
                }
                'm' => {
                    let (v, r) = take_numeric(rest, n_digits(n.max(2)))?;
                    minute = Some(v);
                    rest = r;
                }
                's' => {
                    let (v, r) = take_numeric(rest, n_digits(n.max(2)))?;
                    second = Some(v);
                    rest = r;
                }
                'S' => {
                    let (v, r) = take_numeric(rest, n_digits(n))?;
                    fraction = Some((v as f64 * 10f64.powi(3 - n as i32)).trunc() as i64);
                    rest = r;
                }
                'a' => {
                    let (h, end) = day_period(rest)?;
                    day_period_hour = Some(h);
                    rest = &rest[end..];
                }
                _ => return None,
            },
        }
    }
    if !rest.trim().is_empty() && rest.chars().any(|c| !c.is_whitespace()) {
        return None;
    }
    // Setters by priority: year 130, month 110, date 90, am/pm 80, hour
    // 70, minute 60, second 50, fraction 30; each validates first.
    let mut out = REFERENCE;
    if let Some(y) = year {
        if y <= 0 {
            return None;
        }
        out = NaiveDateTime {
            year: y,
            month: 1,
            day: 1,
            ..REFERENCE
        };
    }
    if let Some(m) = month {
        if !(0..=11).contains(&m) {
            return None;
        }
        out.month = m as u32 + 1;
        out.day = 1;
        out.hour = 0;
    }
    if let Some(d) = day {
        if d < 1 || d > i64::from(js::days_in_month(out.year, out.month)) {
            return None;
        }
        out.day = d as u32;
        out.hour = 0;
    }
    if let Some(h) = day_period_hour {
        out.hour = h;
        out.minute = 0;
        out.second = 0;
        out.millisecond = 0;
    }
    if let Some(h) = hour24 {
        if !(0..=23).contains(&h) {
            return None;
        }
        out.hour = h as u32;
        out.minute = 0;
        out.second = 0;
        out.millisecond = 0;
    } else if let Some(h) = hour12 {
        if !(1..=12).contains(&h) {
            return None;
        }
        let is_pm = out.hour >= 12;
        out.hour = if is_pm && h < 12 {
            h as u32 + 12
        } else if !is_pm && h == 12 {
            0
        } else {
            h as u32
        };
        out.minute = 0;
        out.second = 0;
        out.millisecond = 0;
    }
    if let Some(m) = minute {
        if !(0..=59).contains(&m) {
            return None;
        }
        out.minute = m as u32;
        out.second = 0;
        out.millisecond = 0;
    }
    if let Some(s) = second {
        if !(0..=59).contains(&s) {
            return None;
        }
        out.second = s as u32;
        out.millisecond = 0;
    }
    if let Some(f) = fraction {
        out.millisecond = f.clamp(0, 999) as u32;
    }
    Some(out)
}

/// `parseWithFormats`: the first format that yields a valid date.
pub fn parse_with_formats<S: AsRef<str>>(value: &str, patterns: &[S]) -> Option<NaiveDateTime> {
    patterns
        .iter()
        .find_map(|pattern| date_fns_parse(value, pattern.as_ref()))
}

/// What a matcher recovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemporalParseResult {
    /// A wall clock to anchor.
    Naive(NaiveDateTime),
    /// An instant (explicit offset or unix timestamp), epoch ms.
    Absolute(i64),
}

fn escape_regexp(value: &str) -> String {
    regex::escape(value)
}

const MERIDIEM_AM: &[&str] = &["a.m", "am", "sa", "sáng", "sang"];
const MERIDIEM_PM: &[&str] = &["p.m", "pm", "ch", "chiều", "chieu", "tối", "toi"];

fn meridiem_patterns() -> &'static [(&'static str, Regex)] {
    static RES: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    RES.get_or_init(|| {
        [("am", MERIDIEM_AM), ("pm", MERIDIEM_PM)]
            .into_iter()
            .map(|(marker, aliases)| {
                let mut sorted: Vec<&str> = aliases.to_vec();
                // Stable sort by UTF-16 length, longest first.
                sorted.sort_by_key(|a| std::cmp::Reverse(js::utf16_len(a)));
                let alternatives: Vec<String> = sorted.iter().map(|a| escape_regexp(a)).collect();
                let pattern = format!(r"(?i)([0-9])\s*(?:{})\.?$", alternatives.join("|"));
                (marker, Regex::new(&pattern).expect("valid regex"))
            })
            .collect()
    })
}

/// `normalizeMeridiemSuffix`: any known meridiem marker after a digit
/// becomes ` am` / ` pm`.
pub fn normalize_meridiem_suffix(value: &str) -> String {
    meridiem_patterns()
        .iter()
        .fold(value.to_owned(), |current, (marker, pattern)| {
            pattern
                .replace(&current, format!("${{1}} {marker}").as_str())
                .into_owned()
        })
}

fn localized_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let clock = r"[0-9]{1,2}:[0-9]{2}(?::[0-9]{2})?(?:\s*[ap]m)?";
        let day = r"(?:ngày)?\s*(?P<day>[0-9]{1,4})";
        let month = r"(?:thg|tháng)\s*(?P<month>[0-9]{1,4})";
        let year = r"(?:năm)?\s*(?P<year>[0-9]{1,4})";
        let time = format!(r"(?:\s*(?:lúc)?\s*(?P<time>{clock}))?");
        Regex::new(&format!(r"(?i)^{day}\s*,?\s*{month}\s*,?\s*{year}{time}$"))
            .expect("valid regex")
    })
}

fn match_offset_input(raw: &str) -> Option<TemporalParseResult> {
    if !has_explicit_offset(raw) {
        return None;
    }
    js::parse_date(raw).map(TemporalParseResult::Absolute)
}

fn match_unix_timestamp_input(raw: &str) -> Option<TemporalParseResult> {
    static RE: OnceLock<Regex> = OnceLock::new();
    if !RE
        .get_or_init(|| Regex::new("^[+-]?[0-9]+$").expect("re"))
        .is_match(raw)
    {
        return None;
    }
    let digits = raw.trim_start_matches(['+', '-']).len();
    if digits != 10 && digits != 13 {
        return None;
    }
    let numeric: f64 = raw.parse().ok()?;
    let ms = if digits == 13 {
        numeric
    } else {
        numeric * 1000.0
    };
    if ms.abs() > js::MAX_TIME_MS as f64 {
        return None;
    }
    Some(TemporalParseResult::Absolute(ms as i64))
}

fn match_canonical_format_input(raw: &str) -> Option<TemporalParseResult> {
    parse_with_formats(raw, canonical_input_formats()).map(TemporalParseResult::Naive)
}

fn match_localized_date_time_input(raw: &str) -> Option<TemporalParseResult> {
    let caps = localized_pattern().captures(raw)?;
    let date = format!(
        "{}-{:0>2}-{:0>2}",
        &caps["year"], &caps["month"], &caps["day"]
    );
    let canonical = match caps.name("time") {
        Some(time) => format!("{date} {}", time.as_str()),
        None => date,
    };
    let mut formats: Vec<String> = TIME_PATTERNS
        .iter()
        .map(|t| format!("yyyy-MM-dd {t}"))
        .collect();
    formats.push("yyyy-MM-dd".to_owned());
    parse_with_formats(&canonical, &formats).map(TemporalParseResult::Naive)
}

fn match_time_only_input(raw: &str, timezone: &str, now_ms: i64) -> Option<TemporalParseResult> {
    let time = parse_with_formats(raw, TIME_PATTERNS)?;
    let today = tz::format_in_time_zone(DateArg::Ms(now_ms), timezone, "yyyy-MM-dd").ok()?;
    let clock = tz::format_fields(&time.fields(), 0, "HH:mm:ss.SSS");
    date_fns_parse(&format!("{today} {clock}"), "yyyy-MM-dd HH:mm:ss.SSS")
        .map(TemporalParseResult::Naive)
}

fn project(
    kind: TemporalCustomFieldType,
    result: TemporalParseResult,
    anchor: &str,
) -> Option<String> {
    match result {
        TemporalParseResult::Absolute(instant) => match kind {
            TemporalCustomFieldType::Date => {
                tz::format_in_time_zone(DateArg::Ms(instant), anchor, "yyyy-MM-dd").ok()
            }
            TemporalCustomFieldType::Datetime => js::to_iso_string(instant),
        },
        TemporalParseResult::Naive(date) => {
            let pattern = match kind {
                TemporalCustomFieldType::Date => "yyyy-MM-dd",
                TemporalCustomFieldType::Datetime => "yyyy-MM-dd'T'HH:mm:ss",
            };
            Some(tz::format_fields(&date.fields(), 0, pattern))
        }
    }
}

/// `parseLooseTemporalValue`: a raw cell as the canonical string
/// `normalizeTemporalCustomFieldValue` accepts, or `None`. `now_ms` only
/// matters for a bare clock time.
pub fn parse_loose_temporal_value(
    kind: TemporalCustomFieldType,
    raw: &str,
    anchor_timezone: Option<&str>,
    now_ms: i64,
) -> Option<String> {
    let trimmed = js::trim(raw);
    if trimmed.is_empty() {
        return None;
    }
    let normalized = normalize_meridiem_suffix(trimmed);
    let safe = resolve_filter_timezone(anchor_timezone);
    let result = match_offset_input(&normalized)
        .or_else(|| match_unix_timestamp_input(&normalized))
        .or_else(|| match_canonical_format_input(&normalized))
        .or_else(|| match_localized_date_time_input(&normalized))
        .or_else(|| match_time_only_input(&normalized, &safe, now_ms))?;
    project(kind, result, &safe)
}

/// `TemporalStorageNormalizationInput`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporalStorageNormalizationInput<'a> {
    pub kind: TemporalCustomFieldType,
    /// Raw source text as authored.
    pub value: &'a str,
    /// Anchors a naive value; anything unusable resolves to UTC.
    pub timezone: Option<&'a str>,
    /// Defaults to strict.
    pub parsing: Option<TemporalInputParsing>,
}

/// `normalizeTemporalValueForStorage`: the persisted form, or `None`.
/// Lenient parsing tries the strict normalizer first, then the loose parser.
pub fn normalize_temporal_value_for_storage(
    input: &TemporalStorageNormalizationInput<'_>,
    now_ms: i64,
) -> Option<String> {
    let strict = normalize_temporal_custom_field_value(
        input.kind.as_str(),
        Some(input.value),
        input.timezone,
    );
    match input.parsing.unwrap_or(TemporalInputParsing::Strict) {
        TemporalInputParsing::Strict => strict,
        TemporalInputParsing::Lenient => strict.or_else(|| {
            let canonical =
                parse_loose_temporal_value(input.kind, input.value, input.timezone, now_ms)?;
            normalize_temporal_custom_field_value(
                input.kind.as_str(),
                Some(&canonical),
                input.timezone,
            )
        }),
    }
}
