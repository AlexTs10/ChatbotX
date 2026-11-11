//! App-defined SQL functions (spec section 3), registered on every connection.
//!
//! `lower_u`, `upper_u`, `ilike`, `like_cs`, `regexp` (the `REGEXP` operator),
//! `regexp_replace`, `tz_resolve`, `tz_wall_ms`, `tz_from_wall_ms`, `vector_distance_cos`,
//! `vector32` (P2.5 spec 1.7 item 3).
//! All are `SQLITE_UTF8 | SQLITE_DETERMINISTIC | SQLITE_INNOCUOUS` (`regexp_replace` is used
//! in an index expression and `trusted_schema = OFF`). A SQL `NULL` argument gives `NULL`,
//! except for `tz_resolve` (`NULL` -> `'UTC'`).

use std::borrow::Cow;
use std::str::FromStr;

use chrono::{DateTime, MappedLocalTime, NaiveDateTime, Offset, TimeDelta, TimeZone};
use chrono_tz::Tz;
use regex::{Captures, Regex, RegexBuilder, Replacer};
use rusqlite::functions::{Context, FunctionFlags};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, Error};

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Registers every app function on `conn` (spec 2.2 step 4).
pub(crate) fn register(conn: &Connection) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    conn.create_scalar_function("lower_u", 1, flags, |ctx| {
        Ok(text_arg(ctx, 0)?.map(|t| lower_u(&t)))
    })?;
    conn.create_scalar_function("upper_u", 1, flags, |ctx| {
        Ok(text_arg(ctx, 0)?.map(|t| upper_u(&t)))
    })?;
    conn.create_scalar_function("ilike", 2, flags, |ctx| like_fn(ctx, true))?;
    conn.create_scalar_function("like_cs", 2, flags, |ctx| like_fn(ctx, false))?;
    conn.create_scalar_function("regexp", 2, flags, |ctx| regexp_fn(ctx))?;
    conn.create_scalar_function("regexp_replace", 3, flags, |ctx| regexp_replace_fn(ctx))?;
    conn.create_scalar_function("regexp_replace", 4, flags, |ctx| regexp_replace_fn(ctx))?;
    conn.create_scalar_function("tz_resolve", 1, flags, |ctx| {
        Ok(tz_resolve(text_arg(ctx, 0)?.as_deref()))
    })?;
    conn.create_scalar_function("tz_wall_ms", 2, flags, |ctx| tz_fn(ctx, wall_ms))?;
    conn.create_scalar_function("tz_from_wall_ms", 2, flags, |ctx| tz_fn(ctx, from_wall_ms))?;
    conn.create_scalar_function("vector_distance_cos", 2, flags, |ctx| {
        vector_distance_cos_fn(ctx)
    })?;
    conn.create_scalar_function("vector32", 1, flags, |ctx| vector32_fn(ctx))?;
    Ok(())
}

fn fn_error(message: impl Into<String>) -> Error {
    Error::UserFunctionError(message.into().into())
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

/// TEXT, INTEGER and REAL are read as text (like the built-in `lower()`); BLOB is an error.
fn value_text(value: ValueRef<'_>) -> Result<Option<Cow<'_, str>>, String> {
    match value {
        ValueRef::Null => Ok(None),
        ValueRef::Text(bytes) => std::str::from_utf8(bytes)
            .map(|s| Some(Cow::Borrowed(s)))
            .map_err(|_| "invalid UTF-8 text argument".to_owned()),
        ValueRef::Integer(i) => Ok(Some(Cow::Owned(i.to_string()))),
        ValueRef::Real(f) => Ok(Some(Cow::Owned(real_text(f)))),
        ValueRef::Blob(_) => Err("a BLOB argument where text is expected".to_owned()),
    }
}

/// Text form of a REAL, close to SQLite's (`1.0`, `2.5`).
fn real_text(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

fn text_arg<'a>(ctx: &'a Context<'_>, idx: usize) -> rusqlite::Result<Option<Cow<'a, str>>> {
    value_text(ctx.get_raw(idx)).map_err(fn_error)
}

/// INTEGER (REAL is truncated, TEXT must be an integer literal); BLOB is an error.
fn int_arg(ctx: &Context<'_>, idx: usize) -> rusqlite::Result<Option<i64>> {
    match ctx.get_raw(idx) {
        ValueRef::Null => Ok(None),
        ValueRef::Integer(i) => Ok(Some(i)),
        ValueRef::Real(f) if f.is_finite() => Ok(Some(f.trunc() as i64)),
        ValueRef::Text(bytes) => std::str::from_utf8(bytes)
            .ok()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .map(Some)
            .ok_or_else(|| fn_error("an integer argument is expected")),
        _ => Err(fn_error("an integer argument is expected")),
    }
}

// ---------------------------------------------------------------------------
// 3.1 lower_u / upper_u
// ---------------------------------------------------------------------------

/// Simple (per character) Unicode lowercase, as Postgres `lower()` with a libc UTF-8
/// locale: no final-sigma rule, `İ` -> `i`, no normalization.
pub fn lower_u(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c == '\u{130}' {
            out.push('i');
            continue;
        }
        let mut lower = c.to_lowercase();
        match (lower.next(), lower.next()) {
            (Some(l), None) => out.push(l),
            _ => out.push(c),
        }
    }
    out
}

/// Simple (per character) Unicode uppercase: a character whose uppercase has more than
/// one character (`ß`, `ŉ`, `ﬁ`) stays unchanged.
pub fn upper_u(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let mut upper = c.to_uppercase();
        match (upper.next(), upper.next()) {
            (Some(u), None) => out.push(u),
            _ => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 3.2 ilike / like_cs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LikeToken {
    Any,
    One,
    Literal(char),
}

const LIKE_TRAILING_ESCAPE: &str = "LIKE pattern must not end with escape character";

fn parse_like(pattern: &str) -> Result<Vec<LikeToken>, &'static str> {
    let mut tokens = Vec::with_capacity(pattern.len());
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        let token = match c {
            '%' => {
                if tokens.last() == Some(&LikeToken::Any) {
                    continue;
                }
                LikeToken::Any
            }
            '_' => LikeToken::One,
            '\\' => LikeToken::Literal(chars.next().ok_or(LIKE_TRAILING_ESCAPE)?),
            other => LikeToken::Literal(other),
        };
        tokens.push(token);
    }
    Ok(tokens)
}

/// Anchored LIKE match with escape `\`: `%` = any run, `_` = one character (one Unicode
/// scalar value). Iterative two-pointer match with backtracking on the last `%`.
pub fn like_match(text: &str, pattern: &str) -> Result<bool, &'static str> {
    let pattern = parse_like(pattern)?;
    let text: Vec<char> = text.chars().collect();
    let (mut ti, mut pi) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < text.len() {
        match pattern.get(pi) {
            Some(LikeToken::Any) => {
                star = Some((pi, ti));
                pi += 1;
                continue;
            }
            Some(LikeToken::One) => {
                ti += 1;
                pi += 1;
                continue;
            }
            Some(LikeToken::Literal(c)) if *c == text[ti] => {
                ti += 1;
                pi += 1;
                continue;
            }
            _ => {}
        }
        match star {
            Some((star_pi, star_ti)) => {
                pi = star_pi + 1;
                ti = star_ti + 1;
                star = Some((star_pi, star_ti + 1));
            }
            None => return Ok(false),
        }
    }
    while pattern.get(pi) == Some(&LikeToken::Any) {
        pi += 1;
    }
    Ok(pi == pattern.len())
}

/// `ilike(text, pattern)`: `like_match(lower_u(text), lower_u(pattern))`.
pub fn ilike(text: &str, pattern: &str) -> Result<bool, &'static str> {
    like_match(&lower_u(text), &lower_u(pattern))
}

fn like_fn(ctx: &Context<'_>, fold: bool) -> rusqlite::Result<Option<bool>> {
    let (Some(text), Some(pattern)) = (text_arg(ctx, 0)?, text_arg(ctx, 1)?) else {
        return Ok(None);
    };
    let matched = if fold {
        ilike(&text, &pattern)
    } else {
        like_match(&text, &pattern)
    };
    matched.map(Some).map_err(fn_error)
}

// ---------------------------------------------------------------------------
// 3.3 regexp / 3.4 regexp_replace
// ---------------------------------------------------------------------------

/// Rewrites the Perl classes to ASCII (Postgres/glibc classify only ASCII digits, Rust
/// `\d` is Unicode). Inside a bracket expression the class body is inserted (a negated
/// class becomes a nested class, which Rust `regex` accepts).
fn rewrite_classes(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len() + 8);
    let mut in_class = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            let class = match next {
                'd' => Some(("0-9", false)),
                'D' => Some(("0-9", true)),
                's' => Some((" \\t\\n\\r\\f\\v", false)),
                'S' => Some((" \\t\\n\\r\\f\\v", true)),
                'w' => Some(("0-9A-Za-z_", false)),
                'W' => Some(("0-9A-Za-z_", true)),
                _ => None,
            };
            match class {
                Some((body, negated)) if in_class && !negated => out.push_str(body),
                Some((body, negated)) => {
                    out.push('[');
                    if negated {
                        out.push('^');
                    }
                    out.push_str(body);
                    out.push(']');
                }
                None => {
                    out.push(c);
                    out.push(next);
                }
            }
            i += 2;
            continue;
        }
        if c == '[' && !in_class {
            in_class = true;
            out.push('[');
            i += 1;
            if chars.get(i) == Some(&'^') {
                out.push('^');
                i += 1;
            }
            if chars.get(i) == Some(&']') {
                out.push_str("\\]");
                i += 1;
            }
            continue;
        }
        if c == '[' && in_class {
            // POSIX `[:name:]`, `[=x=]`, `[.x.]`: copy up to the closing `:]`.
            if let Some(&delim @ (':' | '=' | '.')) = chars.get(i + 1) {
                if let Some(end) = (i + 2..chars.len().saturating_sub(1))
                    .find(|&j| chars[j] == delim && chars[j + 1] == ']')
                {
                    out.extend(&chars[i..=end + 1]);
                    i = end + 2;
                    continue;
                }
            }
            out.push_str("\\[");
            i += 1;
            continue;
        }
        if c == ']' && in_class {
            in_class = false;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// Compiles a Postgres ARE pattern (subset): ASCII classes, `.` matches a newline (the
/// Postgres default). Back-references and look-around are refused by Rust `regex`.
pub fn compile_regex(pattern: &str, case_insensitive: bool) -> Result<Regex, String> {
    RegexBuilder::new(&rewrite_classes(pattern))
        .case_insensitive(case_insensitive)
        .dot_matches_new_line(true)
        .build()
        .map_err(|e| format!("invalid regular expression: {e}"))
}

fn regexp_fn(ctx: &Context<'_>) -> rusqlite::Result<Option<bool>> {
    if matches!(ctx.get_raw(0), ValueRef::Null) || matches!(ctx.get_raw(1), ValueRef::Null) {
        return Ok(None);
    }
    let regex = ctx.get_or_create_aux(0, |value| -> Result<Regex, BoxError> {
        let pattern = value_text(value)?.unwrap_or_default();
        Ok(compile_regex(&pattern, false)?)
    })?;
    let text = text_arg(ctx, 1)?.unwrap_or_default();
    Ok(Some(regex.is_match(&text)))
}

/// A compiled `regexp_replace` pattern with the case flag it was compiled with.
struct CachedRegex {
    case_insensitive: bool,
    regex: Regex,
}

/// Parses the `regexp_replace` flags: `g` (every match), `i` (case-insensitive).
fn replace_flags(flags: &str) -> Result<(bool, bool), String> {
    let (mut global, mut case_insensitive) = (false, false);
    for c in flags.chars() {
        match c {
            'g' => global = true,
            'i' => case_insensitive = true,
            other => return Err(format!("invalid regular expression option: \"{other}\"")),
        }
    }
    Ok((global, case_insensitive))
}

fn regexp_replace_fn(ctx: &Context<'_>) -> rusqlite::Result<Option<String>> {
    let flags = if ctx.len() == 4 {
        text_arg(ctx, 3)?
    } else {
        Some(Cow::Borrowed(""))
    };
    let (Some(text), Some(_), Some(replacement), Some(flags)) = (
        text_arg(ctx, 0)?,
        text_arg(ctx, 1)?,
        text_arg(ctx, 2)?,
        flags,
    ) else {
        return Ok(None);
    };
    let (global, case_insensitive) = replace_flags(&flags).map_err(fn_error)?;
    let cached = ctx.get_or_create_aux(1, |value| -> Result<CachedRegex, BoxError> {
        let pattern = value_text(value)?.unwrap_or_default();
        Ok(CachedRegex {
            case_insensitive,
            regex: compile_regex(&pattern, case_insensitive)?,
        })
    })?;
    let fresh;
    let regex = if cached.case_insensitive == case_insensitive {
        &cached.regex
    } else {
        // The flags argument changed while the pattern stayed: compile without caching.
        let pattern = text_arg(ctx, 1)?.unwrap_or_default();
        fresh = compile_regex(&pattern, case_insensitive).map_err(fn_error)?;
        &fresh
    };
    Ok(Some(regexp_replace(regex, &text, &replacement, global)))
}

/// Postgres replacement syntax: `\1`..`\9` = group, `\&` = whole match, `\\` = one
/// backslash; every other character (also `$`) is literal.
struct PgReplacement<'a>(&'a str);

impl Replacer for PgReplacement<'_> {
    fn replace_append(&mut self, caps: &Captures<'_>, dst: &mut String) {
        let mut chars = self.0.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\\' {
                dst.push(c);
                continue;
            }
            match chars.peek().copied() {
                Some(d @ '1'..='9') => {
                    chars.next();
                    let group = d as usize - '0' as usize;
                    if let Some(m) = caps.get(group) {
                        dst.push_str(m.as_str());
                    }
                }
                Some('&') => {
                    chars.next();
                    dst.push_str(caps.get(0).map_or("", |m| m.as_str()));
                }
                Some('\\') => {
                    chars.next();
                    dst.push('\\');
                }
                _ => dst.push('\\'),
            }
        }
    }
}

/// `regexp_replace(text, regex, replacement)`; without `global` only the first match.
pub fn regexp_replace(regex: &Regex, text: &str, replacement: &str, global: bool) -> String {
    let limit = if global { 0 } else { 1 };
    regex
        .replacen(text, limit, PgReplacement(replacement))
        .into_owned()
}

// ---------------------------------------------------------------------------
// 3.5 Time zones
// ---------------------------------------------------------------------------

/// Port of `CANONICAL_BY_LEGACY_TIMEZONE` (`packages/utils/src/timezone.ts`), same order.
const CANONICAL_BY_LEGACY_TIMEZONE: &[(&str, &str)] = &[
    ("Asia/Saigon", "Asia/Ho_Chi_Minh"),
    ("Asia/Calcutta", "Asia/Kolkata"),
    ("Asia/Katmandu", "Asia/Kathmandu"),
    ("Asia/Rangoon", "Asia/Yangon"),
    ("Asia/Chongqing", "Asia/Shanghai"),
    ("Asia/Harbin", "Asia/Shanghai"),
    ("Asia/Istanbul", "Europe/Istanbul"),
    ("Europe/Kiev", "Europe/Kyiv"),
    ("Europe/Uzhgorod", "Europe/Kyiv"),
    ("Europe/Zaporozhye", "Europe/Kyiv"),
    ("America/Buenos_Aires", "America/Argentina/Buenos_Aires"),
    ("America/Godthab", "America/Nuuk"),
    ("Pacific/Ponape", "Pacific/Pohnpei"),
    ("Pacific/Truk", "Pacific/Chuuk"),
    ("Atlantic/Faeroe", "Atlantic/Faroe"),
    ("Africa/Asmera", "Africa/Asmara"),
    ("Australia/Canberra", "Australia/Sydney"),
    ("US", "America/New_York"),
];

/// Port of `timezoneCandidates` (`packages/utils/src/timezone.ts`): the caller's
/// spelling first, then its legacy/canonical alias. This is the candidate list that
/// the SQL function `tz_resolve` uses.
pub fn timezone_candidates(name: &str) -> Vec<String> {
    if let Some((_, canonical)) = CANONICAL_BY_LEGACY_TIMEZONE
        .iter()
        .find(|(legacy, _)| *legacy == name)
    {
        return vec![name.to_owned(), (*canonical).to_owned()];
    }
    match CANONICAL_BY_LEGACY_TIMEZONE
        .iter()
        .find(|(_, canonical)| *canonical == name)
    {
        Some((legacy, _)) => vec![name.to_owned(), (*legacy).to_owned()],
        None => vec![name.to_owned()],
    }
}

/// Port of `resolvedTimezone`: the first candidate the tz database knows (the caller's
/// spelling first), else `UTC`. `None` -> `UTC`. Never an error.
pub fn tz_resolve(name: Option<&str>) -> String {
    name.and_then(|n| {
        timezone_candidates(n)
            .into_iter()
            .find(|c| Tz::from_str(c).is_ok())
    })
    .unwrap_or_else(|| "UTC".to_owned())
}

fn parse_tz(name: &str) -> Result<Tz, String> {
    Tz::from_str(name).map_err(|_| format!("time zone \"{name}\" not recognized"))
}

fn naive_from_ms(ms: i64) -> Result<NaiveDateTime, String> {
    DateTime::from_timestamp_millis(ms)
        .map(|d| d.naive_utc())
        .ok_or_else(|| "timestamp out of range".to_owned())
}

fn utc_offset_ms(tz: Tz, utc: &NaiveDateTime) -> i64 {
    i64::from(tz.offset_from_utc_datetime(utc).fix().local_minus_utc()) * 1000
}

/// Local wall-clock time of the instant `ts_ms` in `tz`, written as if it were UTC.
pub fn wall_ms(ts_ms: i64, tz: Tz) -> Result<i64, String> {
    Ok(ts_ms + utc_offset_ms(tz, &naive_from_ms(ts_ms)?))
}

/// The instant whose local time in `tz` is `wall_ms`. DST gap: the offset before the
/// transition; DST overlap: the offset after it (both are the smaller of the two offsets,
/// the Postgres rule).
pub fn from_wall_ms(wall_ms: i64, tz: Tz) -> Result<i64, String> {
    let local = naive_from_ms(wall_ms)?;
    let offset_ms = match tz.offset_from_local_datetime(&local) {
        MappedLocalTime::Single(o) => i64::from(o.fix().local_minus_utc()) * 1000,
        MappedLocalTime::Ambiguous(a, b) => {
            i64::from(a.fix().local_minus_utc().min(b.fix().local_minus_utc())) * 1000
        }
        MappedLocalTime::None => {
            let day = TimeDelta::days(1);
            let before = local
                .checked_sub_signed(day)
                .ok_or("timestamp out of range")?;
            let after = local
                .checked_add_signed(day)
                .ok_or("timestamp out of range")?;
            utc_offset_ms(tz, &before).min(utc_offset_ms(tz, &after))
        }
    };
    Ok(wall_ms - offset_ms)
}

fn tz_fn(
    ctx: &Context<'_>,
    f: fn(i64, Tz) -> Result<i64, String>,
) -> rusqlite::Result<Option<i64>> {
    let (Some(ms), Some(name)) = (int_arg(ctx, 0)?, text_arg(ctx, 1)?) else {
        return Ok(None);
    };
    let tz = parse_tz(&name).map_err(fn_error)?;
    f(ms, tz).map(Some).map_err(fn_error)
}

// ---------------------------------------------------------------------------
// 3.6 vector_distance_cos
// ---------------------------------------------------------------------------

/// pgvector `a <=> b` on little-endian `f32` blobs (libSQL `vector32` layout). `None`
/// for a zero vector (pgvector returns NaN).
pub fn vector_distance_cos(a: &[u8], b: &[u8]) -> Result<Option<f64>, String> {
    if a.len() % 4 != 0 || b.len() % 4 != 0 {
        return Err("invalid vector blob".to_owned());
    }
    if a.len() != b.len() {
        return Err(format!(
            "different vector dimensions {} and {}",
            a.len() / 4,
            b.len() / 4
        ));
    }
    let (mut dot, mut norm_a, mut norm_b) = (0f32, 0f32, 0f32);
    for (ca, cb) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let x = f32::from_le_bytes([ca[0], ca[1], ca[2], ca[3]]);
        let y = f32::from_le_bytes([cb[0], cb[1], cb[2], cb[3]]);
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let similarity = f64::from(dot) / (f64::from(norm_a) * f64::from(norm_b)).sqrt();
    if similarity.is_nan() {
        return Ok(None);
    }
    Ok(Some(1.0 - similarity.clamp(-1.0, 1.0)))
}

/// libSQL `vector32(text)`: the JSON array text `'[0.1,0.2,…]'` as a little-endian `f32`
/// blob (the layout of `vector_distance_cos` and of the `embedding` columns). Non-finite
/// values are rejected, as pgvector rejects NaN / Infinity on input.
pub fn vector32(text: &str) -> Result<Vec<u8>, String> {
    let invalid = || format!("invalid vector: {text}");
    let inner = text
        .trim()
        .strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .ok_or_else(invalid)?
        .trim();
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(inner.len() / 2);
    for item in inner.split(',') {
        let item = item.trim();
        let starts_ok = item
            .bytes()
            .next()
            .is_some_and(|b| b == b'-' || b.is_ascii_digit());
        let value: f32 = item.parse().map_err(|_| invalid())?;
        if !starts_ok || !value.is_finite() {
            return Err(invalid());
        }
        out.extend_from_slice(&value.to_le_bytes());
    }
    Ok(out)
}

fn vector32_fn(ctx: &Context<'_>) -> rusqlite::Result<Option<Vec<u8>>> {
    match ctx.get_raw(0) {
        ValueRef::Null => Ok(None),
        ValueRef::Text(bytes) => {
            let text = std::str::from_utf8(bytes).map_err(|_| fn_error("invalid vector"))?;
            vector32(text).map(Some).map_err(fn_error)
        }
        ValueRef::Blob(b) if b.len() % 4 == 0 => Ok(Some(b.to_vec())),
        _ => Err(fn_error("invalid vector")),
    }
}

fn vector_distance_cos_fn(ctx: &Context<'_>) -> rusqlite::Result<Option<f64>> {
    match (ctx.get_raw(0), ctx.get_raw(1)) {
        (ValueRef::Null, _) | (_, ValueRef::Null) => Ok(None),
        (ValueRef::Blob(a), ValueRef::Blob(b)) => vector_distance_cos(a, b).map_err(fn_error),
        _ => Err(fn_error("invalid vector blob")),
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::types::Value;

    use super::*;

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        register(&conn).expect("register");
        conn
    }

    fn q(sql: &str) -> Value {
        conn()
            .query_row(sql, [], |r| r.get::<_, Value>(0))
            .expect(sql)
    }

    fn text(sql: &str) -> Option<String> {
        match q(sql) {
            Value::Text(t) => Some(t),
            Value::Null => None,
            other => panic!("{sql}: not text: {other:?}"),
        }
    }

    fn int(sql: &str) -> Option<i64> {
        match q(sql) {
            Value::Integer(i) => Some(i),
            Value::Null => None,
            other => panic!("{sql}: not an integer: {other:?}"),
        }
    }

    fn err(sql: &str) -> String {
        conn()
            .query_row(sql, [], |r| r.get::<_, Value>(0))
            .expect_err(sql)
            .to_string()
    }

    // 3.1
    #[test]
    fn lower_u_vietnamese() {
        assert_eq!(
            text("SELECT lower_u('ĐẶNG Văn Ánh')").as_deref(),
            Some("đặng văn ánh")
        );
    }

    #[test]
    fn lower_u_german_keeps_sharp_s() {
        assert_eq!(text("SELECT lower_u('ÄÖÜ ß')").as_deref(), Some("äöü ß"));
    }

    #[test]
    fn lower_u_no_final_sigma() {
        assert_eq!(text("SELECT lower_u('ΟΔΟΣ')").as_deref(), Some("οδοσ"));
    }

    #[test]
    fn lower_u_dotted_capital_i() {
        assert_eq!(
            text("SELECT lower_u('İstanbul')").as_deref(),
            Some("istanbul")
        );
    }

    #[test]
    fn lower_u_ascii_and_integer() {
        assert_eq!(text("SELECT lower_u('ABC123')").as_deref(), Some("abc123"));
        assert_eq!(text("SELECT lower_u(123)").as_deref(), Some("123"));
    }

    #[test]
    fn lower_u_null() {
        assert_eq!(text("SELECT lower_u(NULL)"), None);
    }

    #[test]
    fn lower_u_no_normalization() {
        assert_eq!(
            text("SELECT lower_u('E' || char(0x301))"),
            Some("e\u{301}".to_owned())
        );
    }

    #[test]
    fn lower_u_blob_is_an_error() {
        assert!(err("SELECT lower_u(x'00')").contains("BLOB"));
    }

    #[test]
    fn upper_u_simple_mapping() {
        assert_eq!(text("SELECT upper_u('straße')").as_deref(), Some("STRAßE"));
        assert_eq!(text("SELECT upper_u('đặng')").as_deref(), Some("ĐẶNG"));
        assert_eq!(text("SELECT upper_u(NULL)"), None);
    }

    // 3.2
    #[test]
    fn ilike_unicode_folding() {
        assert_eq!(int("SELECT ilike('Nguyễn Văn A', '%VĂN%')"), Some(1));
        assert_eq!(int("SELECT ilike('ĐẶNG', 'đặng')"), Some(1));
    }

    #[test]
    fn ilike_underscore_is_one_character() {
        assert_eq!(int("SELECT ilike('abc', 'a_c')"), Some(1));
        assert_eq!(int("SELECT ilike('aéc', 'a_c')"), Some(1));
    }

    #[test]
    fn ilike_escaped_percent_and_underscore() {
        assert_eq!(int(r"SELECT ilike('100%', '100\%')"), Some(1));
        assert_eq!(int(r"SELECT ilike('1000', '100\%')"), Some(0));
        assert_eq!(int(r"SELECT ilike('a_b', 'a\_b')"), Some(1));
        assert_eq!(int(r"SELECT ilike('axb', 'a\_b')"), Some(0));
        assert_eq!(int(r"SELECT ilike('a\b', 'a\\b')"), Some(1));
    }

    #[test]
    fn ilike_is_anchored() {
        assert_eq!(int("SELECT ilike('abc', 'ab')"), Some(0));
        assert_eq!(int("SELECT ilike('', '%')"), Some(1));
        assert_eq!(int("SELECT ilike('abcabd', '%ab_')"), Some(1));
        assert_eq!(int("SELECT ilike('mississippi', '%iss%ppi')"), Some(1));
    }

    #[test]
    fn ilike_null() {
        assert_eq!(int("SELECT ilike(NULL, '%')"), None);
        assert_eq!(int("SELECT ilike('a', NULL)"), None);
        assert_eq!(int("SELECT NOT ilike(NULL, '%')"), None);
    }

    #[test]
    fn ilike_trailing_escape_is_an_error() {
        assert!(err(r"SELECT ilike('a', 'a\')").contains(LIKE_TRAILING_ESCAPE));
    }

    #[test]
    fn ilike_no_normalization() {
        assert_eq!(int("SELECT ilike('e' || char(0x301), 'é')"), Some(0));
    }

    #[test]
    fn like_cs_is_case_sensitive() {
        assert_eq!(int("SELECT like_cs('Abc', 'a%')"), Some(0));
        assert_eq!(int("SELECT like_cs('Abc', 'A%')"), Some(1));
        assert_eq!(int("SELECT like_cs(NULL, 'A%')"), None);
    }

    // 3.3
    #[test]
    fn regexp_matches_anywhere() {
        assert_eq!(int("SELECT '12345' REGEXP '^[0-9]+$'"), Some(1));
        assert_eq!(int("SELECT '12a45' REGEXP '^[0-9]+$'"), Some(0));
        assert_eq!(int("SELECT 'abc' REGEXP 'b'"), Some(1));
    }

    #[test]
    fn regexp_digit_class_is_ascii() {
        assert_eq!(int(r"SELECT '١٢٣' REGEXP '^\d+$'"), Some(0));
        assert_eq!(int(r"SELECT '123' REGEXP '^\d+$'"), Some(1));
        assert_eq!(
            int(r"SELECT '2026-09-08T17:30' REGEXP '^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}'"),
            Some(1)
        );
        assert_eq!(
            int(r"SELECT '-1.5' REGEXP '^-?[0-9]+(\.[0-9]+)?$'"),
            Some(1)
        );
        assert_eq!(int(r"SELECT 'a1' REGEXP '^[\da-z]+$'"), Some(1));
        assert_eq!(int(r"SELECT 'x' REGEXP '^[[:digit:]x]$'"), Some(1));
    }

    #[test]
    fn regexp_null() {
        assert_eq!(int("SELECT NULL REGEXP 'a'"), None);
        assert_eq!(int("SELECT 'a' REGEXP NULL"), None);
    }

    #[test]
    fn regexp_invalid_pattern_is_an_error() {
        assert!(err("SELECT 'a' REGEXP '('").contains("invalid regular expression"));
        assert!(err(r"SELECT 'aa' REGEXP '(a)\1'").contains("invalid regular expression"));
        assert!(err("SELECT 'a' REGEXP '(?=a)'").contains("invalid regular expression"));
    }

    #[test]
    fn regexp_cached_pattern_over_rows() {
        let n = int(
            "WITH RECURSIVE s(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM s WHERE x < 50) \
             SELECT count(*) FROM s WHERE 'a' || x REGEXP '^a[0-9]$'",
        );
        assert_eq!(n, Some(9));
    }

    // 3.4
    #[test]
    fn regexp_replace_non_digits() {
        assert_eq!(
            text(r"SELECT regexp_replace('+84 (90) 123-4567', '\D', '', 'g')").as_deref(),
            Some("84901234567")
        );
    }

    #[test]
    fn regexp_replace_whitespace_runs() {
        assert_eq!(
            text(r"SELECT regexp_replace('John  ' || char(9) || 'Smith ', '[ \t\n\v\f\r]+', ' ', 'g')").as_deref(),
            Some("John Smith ")
        );
    }

    #[test]
    fn regexp_replace_first_only_without_g() {
        assert_eq!(
            text("SELECT regexp_replace('aaa', 'a', 'b')").as_deref(),
            Some("baa")
        );
    }

    #[test]
    fn regexp_replace_groups() {
        assert_eq!(
            text(r"SELECT regexp_replace('ab', '(a)(b)', '\2\1')").as_deref(),
            Some("ba")
        );
        assert_eq!(
            text(r"SELECT regexp_replace('ab', 'b', '[\&]')").as_deref(),
            Some("a[b]")
        );
        assert_eq!(
            text(r"SELECT regexp_replace('ab', 'b', '\\')").as_deref(),
            Some(r"a\")
        );
        assert_eq!(
            text(r"SELECT regexp_replace('ab', 'b', '\x')").as_deref(),
            Some(r"a\x")
        );
    }

    #[test]
    fn regexp_replace_dollar_is_literal() {
        assert_eq!(
            text("SELECT regexp_replace('price', 'price', '$5')").as_deref(),
            Some("$5")
        );
    }

    #[test]
    fn regexp_replace_case_insensitive() {
        assert_eq!(
            text("SELECT regexp_replace('ABC', 'b', 'x', 'gi')").as_deref(),
            Some("AxC")
        );
        assert_eq!(
            text("SELECT regexp_replace('ABC', 'b', 'x', 'g')").as_deref(),
            Some("ABC")
        );
    }

    #[test]
    fn regexp_replace_null() {
        assert_eq!(text("SELECT regexp_replace(NULL, 'a', 'b')"), None);
        assert_eq!(text("SELECT regexp_replace('a', NULL, 'b')"), None);
        assert_eq!(text("SELECT regexp_replace('a', 'a', NULL)"), None);
        assert_eq!(text("SELECT regexp_replace('a', 'a', 'b', NULL)"), None);
    }

    #[test]
    fn regexp_replace_bad_flag_is_an_error() {
        assert!(
            err("SELECT regexp_replace('a', 'a', 'b', 'x')")
                .contains("invalid regular expression option")
        );
    }

    // 3.5
    const AFTER_MIDNIGHT_IN_VIETNAM: i64 = 1_788_888_600_000;

    #[test]
    fn tz_wall_ms_saigon() {
        assert_eq!(
            int(&format!(
                "SELECT tz_wall_ms({AFTER_MIDNIGHT_IN_VIETNAM}, 'Asia/Saigon')"
            )),
            Some(1_788_913_800_000)
        );
    }

    #[test]
    fn tz_day_keys() {
        let day = |tz: &str, ms: i64| {
            text(&format!(
                "SELECT strftime('%Y-%m-%d', tz_wall_ms({ms}, tz_resolve('{tz}')) / 1000, 'unixepoch')"
            ))
        };
        assert_eq!(
            day("Asia/Saigon", AFTER_MIDNIGHT_IN_VIETNAM).as_deref(),
            Some("2026-09-09")
        );
        assert_eq!(
            day("Asia/Ho_Chi_Minh", AFTER_MIDNIGHT_IN_VIETNAM).as_deref(),
            Some("2026-09-09")
        );
        assert_eq!(
            day("Not/AZone", AFTER_MIDNIGHT_IN_VIETNAM).as_deref(),
            Some("2026-09-08")
        );
        assert_eq!(
            day("America/New_York", 1_788_838_200_000).as_deref(),
            Some("2026-09-07")
        );
    }

    #[test]
    fn tz_wall_ms_half_hour_offset() {
        assert_eq!(
            int("SELECT tz_wall_ms(1768478400000, 'Asia/Kolkata')"),
            Some(1_768_498_200_000)
        );
        assert_eq!(
            int("SELECT tz_wall_ms(1768478400000, 'UTC')"),
            Some(1_768_478_400_000)
        );
    }

    #[test]
    fn tz_from_wall_ms_gap_uses_offset_before() {
        assert_eq!(
            int("SELECT tz_from_wall_ms(1520735400000, 'America/New_York')"),
            Some(1_520_753_400_000)
        );
    }

    #[test]
    fn tz_from_wall_ms_overlap_uses_offset_after() {
        assert_eq!(
            int("SELECT tz_from_wall_ms(1541295000000, 'America/New_York')"),
            Some(1_541_313_000_000)
        );
    }

    #[test]
    fn tz_from_wall_ms_inverts_tz_wall_ms() {
        assert_eq!(
            int(
                "SELECT tz_from_wall_ms(tz_wall_ms(1768478400000, 'Asia/Kolkata'), 'Asia/Kolkata')"
            ),
            Some(1_768_478_400_000)
        );
    }

    #[test]
    fn tz_resolve_prefers_callers_spelling() {
        assert_eq!(
            text("SELECT tz_resolve('Asia/Saigon')").as_deref(),
            Some("Asia/Saigon")
        );
        assert_eq!(
            text("SELECT tz_resolve('Asia/Ho_Chi_Minh')").as_deref(),
            Some("Asia/Ho_Chi_Minh")
        );
        assert_eq!(
            text("SELECT tz_resolve('Not/AZone')").as_deref(),
            Some("UTC")
        );
        assert_eq!(text("SELECT tz_resolve(NULL)").as_deref(), Some("UTC"));
        assert_eq!(
            text("SELECT tz_resolve('US')").as_deref(),
            Some("America/New_York")
        );
    }

    #[test]
    fn tz_unknown_zone_is_an_error() {
        assert!(
            err("SELECT tz_wall_ms(0, 'Not/AZone')")
                .contains("time zone \"Not/AZone\" not recognized")
        );
        assert!(err("SELECT tz_from_wall_ms(0, 'Not/AZone')").contains("not recognized"));
    }

    #[test]
    fn tz_null() {
        assert_eq!(int("SELECT tz_wall_ms(NULL, 'UTC')"), None);
        assert_eq!(int("SELECT tz_from_wall_ms(NULL, 'UTC')"), None);
        assert_eq!(int("SELECT tz_wall_ms(0, NULL)"), None);
    }

    #[test]
    fn timezone_candidates_both_directions() {
        assert_eq!(
            timezone_candidates("Asia/Calcutta"),
            ["Asia/Calcutta", "Asia/Kolkata"]
        );
        assert_eq!(
            timezone_candidates("Europe/Kyiv"),
            ["Europe/Kyiv", "Europe/Kiev"]
        );
        assert_eq!(timezone_candidates("Europe/Berlin"), ["Europe/Berlin"]);
    }

    // 3.6
    fn blob(v: &[f32]) -> String {
        let hex: String = v
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .map(|b| format!("{b:02x}"))
            .collect();
        format!("x'{hex}'")
    }

    fn distance(a: &[f32], b: &[f32]) -> Option<f64> {
        match q(&format!(
            "SELECT vector_distance_cos({}, {})",
            blob(a),
            blob(b)
        )) {
            Value::Real(f) => Some(f),
            Value::Null => None,
            other => panic!("not real: {other:?}"),
        }
    }

    #[test]
    fn vector_distance_cos_identical() {
        assert!(
            distance(&[0.3, 0.4, 0.5], &[0.3, 0.4, 0.5])
                .expect("value")
                .abs()
                < 1e-7
        );
    }

    #[test]
    fn vector_distance_cos_orthogonal_and_opposite() {
        assert!((distance(&[1.0, 0.0], &[0.0, 1.0]).expect("value") - 1.0).abs() < 1e-6);
        assert!((distance(&[1.0, 2.0], &[-1.0, -2.0]).expect("value") - 2.0).abs() < 1e-6);
    }

    #[test]
    fn vector_distance_cos_45_degrees() {
        let d = distance(&[1.0, 0.0], &[1.0, 1.0]).expect("value");
        assert!((d - (1.0 - 1.0 / 2f64.sqrt())).abs() < 1e-6, "{d}");
    }

    #[test]
    fn vector_distance_cos_zero_vector_is_null() {
        assert_eq!(distance(&[0.0, 0.0], &[1.0, 1.0]), None);
    }

    #[test]
    fn vector_distance_cos_errors() {
        assert!(
            err(&format!(
                "SELECT vector_distance_cos({}, {})",
                blob(&[1.0]),
                blob(&[1.0, 2.0])
            ))
            .contains("different vector dimensions 1 and 2")
        );
        assert!(
            err("SELECT vector_distance_cos(x'000000', x'000000')").contains("invalid vector blob")
        );
        assert!(err("SELECT vector_distance_cos('abcd', 'abcd')").contains("invalid vector blob"));
    }

    #[test]
    fn vector_distance_cos_null() {
        assert_eq!(
            q("SELECT vector_distance_cos(NULL, x'00000000')"),
            Value::Null
        );
    }

    #[test]
    fn vector32_text_gives_the_little_endian_f32_blob() {
        let expected: Vec<u8> = [0.1f32, -2.5, 3e-3]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        assert_eq!(
            q("SELECT vector32('[0.1, -2.5,3e-3]')"),
            Value::Blob(expected)
        );
        assert_eq!(q("SELECT vector32('[]')"), Value::Blob(Vec::new()));
        assert_eq!(q("SELECT vector32(NULL)"), Value::Null);
        // A valid blob passes through (libSQL accepts both forms).
        assert_eq!(
            q("SELECT vector32(x'0000803f')"),
            Value::Blob(1f32.to_le_bytes().to_vec())
        );
    }

    #[test]
    fn vector32_matches_vector_distance_cos_on_blobs() {
        let d = match q(&format!(
            "SELECT vector_distance_cos(vector32('[1,0]'), {})",
            blob(&[1.0, 1.0])
        )) {
            Value::Real(f) => f,
            other => panic!("not real: {other:?}"),
        };
        assert!((d - (1.0 - 1.0 / 2f64.sqrt())).abs() < 1e-6, "{d}");
    }

    #[test]
    fn vector32_rejects_bad_input() {
        for bad in [
            "'0.1,0.2'",
            "'[0.1,]'",
            "'[a]'",
            "'[NaN]'",
            "'[inf]'",
            "'[1e400]'",
            "'[+1]'",
            "x'000000'",
            "1",
        ] {
            assert!(
                err(&format!("SELECT vector32({bad})")).contains("invalid vector"),
                "{bad}"
            );
        }
    }
}
