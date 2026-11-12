//! JavaScript runtime semantics the ported code depends on: `Number#toString`,
//! UTF-16 lengths, `String#trim`, `Date.UTC`, `new Date(string)` and
//! `Date#toISOString`. The process time zone is taken as UTC (the servers
//! run in UTC; see `conversion/specs/P3.1.md`).

/// The largest time value `Date` accepts (ms, both signs).
pub const MAX_TIME_MS: i64 = 8_640_000_000_000_000;

const MS_PER_DAY: i64 = 86_400_000;

/// `String(n)` / `Number#toString()` for a finite or special `f64`.
pub fn number_to_string(value: f64) -> String {
    if value.is_nan() {
        return "NaN".to_owned();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.to_owned();
    }
    if value == 0.0 {
        return "0".to_owned();
    }
    let negative = value < 0.0;
    // `{:e}` prints the shortest round-trip digits, e.g. `1.2345e3`.
    let scientific = format!("{:e}", value.abs());
    let (mantissa, exponent) = scientific
        .split_once('e')
        .unwrap_or((scientific.as_str(), "0"));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let exponent: i64 = exponent.parse().unwrap_or(0);
    let k = digits.len() as i64;
    let n = exponent + 1;
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if k <= n && n <= 21 {
        out.push_str(&digits);
        for _ in 0..(n - k) {
            out.push('0');
        }
    } else if 0 < n && n <= 21 {
        let (int, frac) = digits.split_at(n as usize);
        out.push_str(int);
        out.push('.');
        out.push_str(frac);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        for _ in 0..(-n) {
            out.push('0');
        }
        out.push_str(&digits);
    } else {
        let e = n - 1;
        let sign = if e < 0 { '-' } else { '+' };
        let (first, rest) = digits.split_at(1);
        out.push_str(first);
        if !rest.is_empty() {
            out.push('.');
            out.push_str(rest);
        }
        out.push('e');
        out.push(sign);
        out.push_str(&e.abs().to_string());
    }
    out
}

/// `Number(text)`: trimmed; empty is 0; `Infinity`, `0x`/`0o`/`0b`
/// integers and decimal literals; anything else is `NaN`.
pub fn to_number(text: &str) -> f64 {
    let t = trim(text);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    let lower = t.get(..2).map(str::to_ascii_lowercase);
    let radix = match lower.as_deref() {
        Some("0x") => 16,
        Some("0o") => 8,
        Some("0b") => 2,
        _ => 10,
    };
    if radix != 10 {
        let digits = &t[2..];
        if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
            return f64::NAN;
        }
        return digits.chars().fold(0f64, |acc, c| {
            acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0))
        });
    }
    let bytes = t.as_bytes();
    let mut i = 0;
    if matches!(bytes.first(), Some(b'+') | Some(b'-')) {
        i += 1;
    }
    let int_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let int_digits = i - int_start;
    let mut frac_digits = 0;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        frac_digits = i - frac_start;
    }
    if int_digits == 0 && frac_digits == 0 {
        return f64::NAN;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if matches!(bytes.get(i), Some(b'+') | Some(b'-')) {
            i += 1;
        }
        let exp_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == exp_start {
            return f64::NAN;
        }
    }
    if i != bytes.len() {
        return f64::NAN;
    }
    // Rust does not take "1." or ".5" with an exponent in every form; give
    // it a canonical literal.
    let mut literal = t.to_owned();
    if literal.contains('.') {
        literal = literal.replace(".e", ".0e").replace(".E", ".0E");
        if literal.ends_with('.') {
            literal.push('0');
        }
    }
    literal.parse::<f64>().unwrap_or(f64::NAN)
}

/// `Math.round`: nearest, ties toward +Infinity.
pub fn math_round(value: f64) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let floor = value.floor();
    if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

/// `JSON.stringify(value)` for a JSON value: numbers as `Number#toString`
/// (`1e+21`), strings escaped like JavaScript, object keys in map order.
pub fn json_stringify(value: &serde_json::Value) -> String {
    let mut out = String::new();
    write_json(value, &mut out);
    out
}

fn write_json(value: &serde_json::Value, out: &mut String) {
    use serde_json::Value;
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                out.push_str(&n.to_string());
            } else {
                let f = n.as_f64().unwrap_or(f64::NAN);
                if f.is_finite() {
                    out.push_str(&number_to_string(f));
                } else {
                    out.push_str("null");
                }
            }
        }
        Value::String(s) => out.push_str(&serde_json::to_string(s).unwrap_or_default()),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (key, item)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push(':');
                write_json(item, out);
            }
            out.push('}');
        }
    }
}

/// `String(value)` for a JSON value (`[object Object]`, arrays joined).
pub fn string_of(value: &serde_json::Value) -> String {
    use serde_json::Value;
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), number_to_string),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                if item.is_null() {
                    String::new()
                } else {
                    string_of(item)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_owned(),
    }
}

/// `decodeURIComponent`: `Err` (URIError) on a malformed escape or
/// invalid UTF-8.
pub fn decode_uri_component(text: &str) -> Result<String, ()> {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3).ok_or(())?;
            out.push(u8::from_str_radix(hex, 16).map_err(|_| ())?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ())
}

/// `encodeURIComponent`.
pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `Number.parseInt(text, 10)`: leading whitespace, a sign, then decimal
/// digits; `None` is `NaN`.
pub fn parse_int(text: &str) -> Option<f64> {
    let t = trim_start(text);
    let (sign, rest) = match t.as_bytes().first() {
        Some(b'-') => (-1.0, &t[1..]),
        Some(b'+') => (1.0, &t[1..]),
        _ => (1.0, t),
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    Some(sign * digits.parse::<f64>().ok()?)
}

/// `String#length`: the number of UTF-16 code units.
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Whether `c` is removed by `String#trim` (ECMAScript WhiteSpace and
/// LineTerminator).
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'
            | '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// `String#trim`.
pub fn trim(text: &str) -> &str {
    text.trim_matches(is_js_whitespace)
}

/// `String#trimStart`.
pub fn trim_start(text: &str) -> &str {
    text.trim_start_matches(is_js_whitespace)
}

/// `String#trimEnd`.
pub fn trim_end(text: &str) -> &str {
    text.trim_end_matches(is_js_whitespace)
}

/// `String#toLowerCase` (full Unicode lower-casing).
pub fn to_lower(text: &str) -> String {
    text.to_lowercase()
}

/// `String#toUpperCase` (full Unicode upper-casing).
pub fn to_upper(text: &str) -> String {
    text.to_uppercase()
}

/// `String#slice(start, end)` on UTF-16 indexes; the result never splits a
/// character (a surrogate pair cut by either index is left out).
pub fn slice_utf16(text: &str, start: usize, end: usize) -> &str {
    let mut units = 0usize;
    let mut start_byte = text.len();
    let mut end_byte = text.len();
    let mut found_start = false;
    for (byte, c) in text.char_indices() {
        if !found_start && units >= start {
            start_byte = byte;
            found_start = true;
        }
        if units + c.len_utf16() > end {
            end_byte = byte;
            break;
        }
        units += c.len_utf16();
    }
    if !found_start {
        return "";
    }
    if end_byte < start_byte {
        return "";
    }
    &text[start_byte..end_byte]
}

// ---------------------------------------------------------------------------
// Calendar helpers (proleptic Gregorian, like `Date`)
// ---------------------------------------------------------------------------

/// Days since 1970-01-01 of a civil date; months and days may overflow, as
/// in `Date.UTC` (`month` is 0-based).
pub fn days_from_civil(year: i64, month0: i64, day: i64) -> i64 {
    let y = year + month0.div_euclid(12);
    let m = month0.rem_euclid(12) + 1;
    let (y, m) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468 + (day - 1)
}

/// The civil date `(year, month 1-12, day)` of a day number.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `Date.UTC(year, month0, day, hours, minutes, seconds, ms)`.
pub fn date_utc(
    year: i64,
    month0: i64,
    day: i64,
    hours: i64,
    minutes: i64,
    seconds: i64,
    ms: i64,
) -> i64 {
    days_from_civil(year, month0, day) * MS_PER_DAY
        + hours * 3_600_000
        + minutes * 60_000
        + seconds * 1000
        + ms
}

/// Days in `month` (1-12) of `year`.
pub fn days_in_month(year: i64, month: u32) -> u32 {
    let next = days_from_civil(year, i64::from(month), 1);
    let this = days_from_civil(year, i64::from(month) - 1, 1);
    (next - this) as u32
}

/// UTC calendar fields of a time value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UtcFields {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub millisecond: u32,
    /// 0 = Sunday.
    pub weekday: u32,
}

/// The UTC fields of `ms`.
pub fn utc_fields(ms: i64) -> UtcFields {
    let days = ms.div_euclid(MS_PER_DAY);
    let rem = ms.rem_euclid(MS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    UtcFields {
        year,
        month,
        day,
        hour: (rem / 3_600_000) as u32,
        minute: (rem / 60_000 % 60) as u32,
        second: (rem / 1000 % 60) as u32,
        millisecond: (rem % 1000) as u32,
        weekday: (days + 4).rem_euclid(7) as u32,
    }
}

/// `Date#toISOString()`; `None` for an invalid (out of range) time value.
pub fn to_iso_string(ms: i64) -> Option<String> {
    if ms.abs() > MAX_TIME_MS {
        return None;
    }
    let f = utc_fields(ms);
    let year = if (0..=9999).contains(&f.year) {
        format!("{:04}", f.year)
    } else if f.year < 0 {
        format!("-{:06}", -f.year)
    } else {
        format!("+{:06}", f.year)
    };
    Some(format!(
        "{year}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        f.month, f.day, f.hour, f.minute, f.second, f.millisecond
    ))
}

fn take_digits(bytes: &[u8], pos: &mut usize, count: usize) -> Option<i64> {
    if *pos + count > bytes.len() {
        return None;
    }
    let mut value = 0i64;
    for i in 0..count {
        let b = bytes[*pos + i];
        if !b.is_ascii_digit() {
            return None;
        }
        value = value * 10 + i64::from(b - b'0');
    }
    *pos += count;
    Some(value)
}

fn take_one_or_two_digits(bytes: &[u8], pos: &mut usize) -> Option<i64> {
    let first = take_digits(bytes, pos, 1)?;
    match bytes.get(*pos) {
        Some(b) if b.is_ascii_digit() => {
            *pos += 1;
            Some(first * 10 + i64::from(b - b'0'))
        }
        _ => Some(first),
    }
}

/// `Date.parse(text)` / `new Date(text).getTime()` for the ISO-like forms V8
/// accepts: `YYYY[-MM[-DD]]` (UTC), with `T` or a space and
/// `HH:mm[:ss[.fff…]]`, then `Z`, `±HH:mm` or `±HHmm` (no offset = local
/// time, taken as UTC). Like V8, a day up to 31 rolls over (`2026-02-30` is
/// March 2). `None` is an Invalid Date.
pub fn parse_date(text: &str) -> Option<i64> {
    // V8 does not trim: `" 2026-07-22T09:30Z "` is an Invalid Date.
    let bytes = text.as_bytes();
    let mut pos = 0usize;
    let year = match bytes.first() {
        Some(b'+') | Some(b'-') => {
            let negative = bytes[0] == b'-';
            pos = 1;
            let y = take_digits(bytes, &mut pos, 6)?;
            if negative && y == 0 {
                return None;
            }
            if negative { -y } else { y }
        }
        _ => take_digits(bytes, &mut pos, 4)?,
    };
    let mut month = 1i64;
    let mut day = 1i64;
    if bytes.get(pos) == Some(&b'-') {
        pos += 1;
        // V8's legacy parser also takes one-digit months and days
        // (`2026-7-22`).
        month = take_one_or_two_digits(bytes, &mut pos)?;
        if bytes.get(pos) == Some(&b'-') {
            pos += 1;
            day = take_one_or_two_digits(bytes, &mut pos)?;
        }
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let date_days = days_from_civil(year, month - 1, day);
    if pos == bytes.len() {
        let ms = date_days * MS_PER_DAY;
        return (ms.abs() <= MAX_TIME_MS).then_some(ms);
    }
    match bytes.get(pos) {
        Some(b'T') | Some(b't') | Some(b' ') => pos += 1,
        _ => return None,
    }
    let hour = take_digits(bytes, &mut pos, 2)?;
    if bytes.get(pos) != Some(&b':') {
        return None;
    }
    pos += 1;
    let minute = take_digits(bytes, &mut pos, 2)?;
    let mut second = 0i64;
    let mut millis = 0i64;
    if bytes.get(pos) == Some(&b':') {
        pos += 1;
        second = take_digits(bytes, &mut pos, 2)?;
        if bytes.get(pos) == Some(&b'.') {
            pos += 1;
            let start = pos;
            while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                pos += 1;
            }
            if pos == start {
                return None;
            }
            let fraction = &text[start..pos];
            let padded = format!("{:0<3}", &fraction[..fraction.len().min(3)]);
            millis = padded.parse().ok()?;
        }
    }
    if hour > 24 || minute > 59 || second > 59 {
        return None;
    }
    if hour == 24 && (minute != 0 || second != 0 || millis != 0) {
        return None;
    }
    let mut offset_ms = 0i64;
    if pos < bytes.len() {
        match bytes[pos] {
            b'Z' | b'z' => {
                pos += 1;
            }
            b'+' | b'-' => {
                let sign = if bytes[pos] == b'-' { -1 } else { 1 };
                pos += 1;
                let oh = take_digits(bytes, &mut pos, 2)?;
                if bytes.get(pos) == Some(&b':') {
                    pos += 1;
                }
                let om = take_digits(bytes, &mut pos, 2)?;
                if oh > 23 || om > 59 {
                    return None;
                }
                offset_ms = sign * (oh * 3_600_000 + om * 60_000);
            }
            _ => return None,
        }
    }
    if pos != bytes.len() {
        return None;
    }
    let ms = date_days * MS_PER_DAY + hour * 3_600_000 + minute * 60_000 + second * 1000 + millis
        - offset_ms;
    (ms.abs() <= MAX_TIME_MS).then_some(ms)
}

/// The current time (`Date.now()`).
pub fn now_ms() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_millis()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_millis()).unwrap_or(i64::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_to_string_matches_js() {
        assert_eq!(number_to_string(42.0), "42");
        assert_eq!(number_to_string(-1.5), "-1.5");
        assert_eq!(number_to_string(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(number_to_string(1e21), "1e+21");
        assert_eq!(number_to_string(1e20), "100000000000000000000");
        assert_eq!(number_to_string(1e-7), "1e-7");
        assert_eq!(number_to_string(0.000001), "0.000001");
        assert_eq!(number_to_string(123.456), "123.456");
        assert_eq!(number_to_string(1.5e300), "1.5e+300");
    }

    #[test]
    fn to_number_matches_js() {
        assert_eq!(to_number(" 5 "), 5.0);
        assert_eq!(to_number("1e2"), 100.0);
        assert_eq!(to_number("1."), 1.0);
        assert_eq!(to_number(".5"), 0.5);
        assert_eq!(to_number("0x10"), 16.0);
        assert_eq!(to_number(""), 0.0);
        assert!(to_number("1aaa1").is_nan());
        assert!(to_number("inf").is_nan());
        assert!(to_number("NaN").is_nan());
        assert_eq!(to_number("-Infinity"), f64::NEG_INFINITY);
        assert_eq!(math_round(2.5), 3.0);
        assert_eq!(math_round(-2.5), -2.0);
        assert_eq!(math_round(0.49999999999999994), 0.0);
    }

    #[test]
    fn parse_date_matches_v8_for_iso_forms() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("1970-01-01T00:00:01Z"), Some(1000));
        assert_eq!(parse_date("1970-01-01T07:00:00+07:00"), Some(0));
        assert_eq!(parse_date("1970-01-01T07:00:00+0700"), Some(0));
        assert_eq!(parse_date("1970-01-01 00:00"), Some(0));
        assert_eq!(parse_date("1970-01-01T00:00:00.5Z"), Some(500));
        assert_eq!(parse_date("2026-02-30"), parse_date("2026-03-02"));
        assert_eq!(parse_date("2026-02-32"), None);
        assert_eq!(parse_date("garbage"), None);
        assert_eq!(parse_date("2026-7-22"), parse_date("2026-07-22"));
        assert_eq!(parse_date("2026-07-22T9:30Z"), None);
        assert_eq!(parse_date(" 2026-07-22T09:30Z "), None);
        assert_eq!(parse_date("2026-07-22T15:30:00+07"), None);
        assert_eq!(parse_date("2026-07-22T24:00Z"), parse_date("2026-07-23"));
    }

    #[test]
    fn iso_string_round_trips() {
        assert_eq!(
            to_iso_string(0).as_deref(),
            Some("1970-01-01T00:00:00.000Z")
        );
        assert_eq!(
            to_iso_string(-1).as_deref(),
            Some("1969-12-31T23:59:59.999Z")
        );
        assert_eq!(to_iso_string(MAX_TIME_MS + 1), None);
    }

    #[test]
    fn slice_utf16_never_splits_a_pair() {
        assert_eq!(slice_utf16("ab😀c", 0, 3), "ab");
        assert_eq!(slice_utf16("ab😀c", 0, 4), "ab😀");
        assert_eq!(slice_utf16("abc", 1, 10), "bc");
    }
}
