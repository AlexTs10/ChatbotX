//! Port of `repositories/pipeline/item-cursor.ts`: the keyset cursor of the pipeline item
//! lists (and of the newest-first comment / activity lists).
//!
//! SQLite form (P3.3 pipeline slice notes): timestamps are integer unix ms, so a cursor
//! sort value is the ISO millisecond text (`repo::iso`); `value` is money (INTEGER x
//! 10,000) and its text is the decimal with 4 fraction digits (`12.5000`, the Postgres
//! `numeric(19,4)::text`); `priority` is the rank `1`..`3`. A decoded value is bound as an
//! integer (ms, x 10,000 or rank), never as SQL text.

use std::sync::LazyLock;

use regex::Regex;
use rusqlite::types::Value;

use super::sql::Sql;
use super::utils::{SortDir, is_bigint_id};
use super::{Error, iso};

/// A fragment whose `text` holds its one `?` (unlike [`Sql::bind`], which appends it).
pub fn bind1(text: impl Into<String>, value: impl Into<Value>) -> Sql {
    Sql::new(text, vec![value.into()])
}

/// `pipelineItemSortKeys`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineItemSortKey {
    CreatedAt,
    UpdatedAt,
    Value,
    DueDate,
    Priority,
}

impl PipelineItemSortKey {
    /// The key as written in a cursor.
    pub fn as_str(self) -> &'static str {
        match self {
            PipelineItemSortKey::CreatedAt => "createdAt",
            PipelineItemSortKey::UpdatedAt => "updatedAt",
            PipelineItemSortKey::Value => "value",
            PipelineItemSortKey::DueDate => "dueDate",
            PipelineItemSortKey::Priority => "priority",
        }
    }
}

/// `PipelineItemSort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineItemSort {
    pub key: PipelineItemSortKey,
    pub direction: SortDir,
}

/// `NEWEST_FIRST_SORT`: newest first (comments and activity).
pub const NEWEST_FIRST_SORT: PipelineItemSort = PipelineItemSort {
    key: PipelineItemSortKey::CreatedAt,
    direction: SortDir::Desc,
};

/// `PipelineItemCursor` (`v` is always 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineItemCursor {
    pub v: i64,
    pub sort: PipelineItemSortKey,
    pub dir: SortDir,
    pub null_rank: u8,
    pub sort_value: Option<String>,
    pub id: String,
}

/// `InvalidPipelineCursorError("Invalid cursor")`.
pub fn invalid_cursor() -> Error {
    Error::Coded {
        kind: "InvalidPipelineCursorError",
        code: "INVALID_CURSOR".to_owned(),
        message: "Invalid cursor".to_owned(),
    }
}

/// Whether `err` is an [`invalid_cursor`] error.
pub fn is_invalid_cursor(err: &Error) -> bool {
    matches!(
        err,
        Error::Coded {
            kind: "InvalidPipelineCursorError",
            ..
        }
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cast {
    Timestamp,
    Numeric,
    Int,
}

struct SortSpec {
    expression: &'static str,
    cast: Cast,
    nullable: bool,
}

/// Priority ordering: high > medium > low.
const PRIORITY_RANK: &str =
    "case \"PipelineItem\".\"priority\" when 'high' then 3 when 'medium' then 2 else 1 end";

fn sort_spec(key: PipelineItemSortKey) -> SortSpec {
    match key {
        PipelineItemSortKey::CreatedAt => SortSpec {
            expression: "\"PipelineItem\".\"createdAt\"",
            cast: Cast::Timestamp,
            nullable: false,
        },
        PipelineItemSortKey::UpdatedAt => SortSpec {
            expression: "\"PipelineItem\".\"updatedAt\"",
            cast: Cast::Timestamp,
            nullable: false,
        },
        PipelineItemSortKey::Value => SortSpec {
            expression: "\"PipelineItem\".\"value\"",
            cast: Cast::Numeric,
            nullable: true,
        },
        PipelineItemSortKey::DueDate => SortSpec {
            expression: "\"PipelineItem\".\"dueDate\"",
            cast: Cast::Timestamp,
            nullable: true,
        },
        PipelineItemSortKey::Priority => SortSpec {
            expression: PRIORITY_RANK,
            cast: Cast::Int,
            nullable: false,
        },
    }
}

static TIMESTAMP_TEXT_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\d{4,}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(\.\d{1,6})?([+-]\d{2}(:\d{2}){0,2}|Z)?( BC)?$",
    )
    .expect("timestamp pattern")
});
static NUMERIC_TEXT_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-?\d+(\.\d+)?$").expect("numeric pattern"));
static TIMESTAMP_PARTS_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(\d{4,})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,6}))?(?:([+-])(\d{2})(?::(\d{2}))?(?::(\d{2}))?|Z)?( BC)?$",
    )
    .expect("timestamp parts pattern")
});
static PRIORITY_RANK_TEXT_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[1-3]$").expect("rank pattern"));

const MAX_NUMERIC_INTEGER_DIGITS: usize = 30;
const MAX_OFFSET_HOURS: i64 = 15;
const DAYS_IN_MONTH: [i64; 12] = [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
const PAGE_SIZE_MAX: i64 = 100;
const MONEY_SCALE: i64 = 10_000;

fn in_range(value: i64, min: i64, max: i64) -> bool {
    value >= min && value <= max
}

struct TimestampParts {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    millis: i64,
    offset_seconds: i64,
}

fn timestamp_parts(text: &str) -> Option<TimestampParts> {
    let caps = TIMESTAMP_PARTS_PATTERN.captures(text)?;
    let num = |i: usize| -> i64 {
        caps.get(i)
            .and_then(|m| m.as_str().parse::<i64>().ok())
            .unwrap_or(0)
    };
    let mut year = caps.get(1)?.as_str().parse::<i64>().ok()?;
    if caps.get(12).is_some() {
        // Astronomical numbering: 1 BC = year 0.
        year = 1 - year;
    }
    let millis = caps.get(7).map_or(0, |m| {
        let mut digits = m.as_str().to_owned();
        while digits.len() < 3 {
            digits.push('0');
        }
        digits[..3].parse::<i64>().unwrap_or(0)
    });
    let sign = match caps.get(8).map(|m| m.as_str()) {
        Some("-") => -1,
        _ => 1,
    };
    Some(TimestampParts {
        year,
        month: num(2),
        day: num(3),
        hour: num(4),
        minute: num(5),
        second: num(6),
        millis,
        offset_seconds: sign * (num(9) * 3600 + num(10) * 60 + num(11)),
    })
}

/// Calendar and clock ranges Postgres accepts for a timestamp literal.
fn is_timestamp_in_range(text: &str) -> bool {
    let Some(caps) = TIMESTAMP_PARTS_PATTERN.captures(text) else {
        return false;
    };
    let num = |i: usize| -> i64 {
        caps.get(i)
            .and_then(|m| m.as_str().parse::<i64>().ok())
            .unwrap_or(0)
    };
    let month = num(2);
    let max_day = usize::try_from(month - 1)
        .ok()
        .and_then(|i| DAYS_IN_MONTH.get(i).copied())
        .unwrap_or(0);
    in_range(month, 1, 12)
        && in_range(num(3), 1, max_day)
        && in_range(num(4), 0, 24)
        && in_range(num(5), 0, 59)
        && in_range(num(6), 0, 59)
        && in_range(num(9), 0, MAX_OFFSET_HOURS)
        && in_range(num(10), 0, 59)
        && in_range(num(11), 0, 59)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (astronomical year).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Unix ms of a cursor timestamp text (fraction truncated to ms; no offset = UTC).
pub fn timestamp_text_ms(text: &str) -> Option<i64> {
    let p = timestamp_parts(text)?;
    let days = days_from_civil(p.year, p.month, p.day);
    let seconds = days * 86_400 + p.hour * 3600 + p.minute * 60 + p.second - p.offset_seconds;
    Some(seconds * 1000 + p.millis)
}

fn is_numeric_sane(text: &str) -> bool {
    let integer_part = text.replacen('-', "", 1);
    let integer_part = integer_part.split('.').next().unwrap_or_default();
    integer_part.len() <= MAX_NUMERIC_INTEGER_DIGITS
}

/// A null sort value is only legal for the NULL rank group, else it must fit the key's type.
fn is_sort_value_valid(key: PipelineItemSortKey, null_rank: u8, sort_value: Option<&str>) -> bool {
    if null_rank == 1 {
        return sort_value.is_none();
    }
    let Some(text) = sort_value else {
        return false;
    };
    match sort_spec(key).cast {
        Cast::Timestamp => TIMESTAMP_TEXT_PATTERN.is_match(text) && is_timestamp_in_range(text),
        Cast::Numeric => NUMERIC_TEXT_PATTERN.is_match(text) && is_numeric_sane(text),
        Cast::Int => PRIORITY_RANK_TEXT_PATTERN.is_match(text),
    }
}

/// `clampPageLimit`: a requested page size kept within 1..100.
pub fn clamp_page_limit(limit: i64) -> i64 {
    limit.clamp(1, PAGE_SIZE_MAX)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64url_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let take = chunk.len() + 1;
        for i in 0..take {
            let idx = usize::try_from((n >> (18 - 6 * i)) & 63).unwrap_or(0);
            out.push(char::from(B64[idx]));
        }
    }
    out
}

/// Node `Buffer.from(raw, "base64url")`: characters outside the alphabet are skipped.
fn base64url_decode(raw: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for byte in raw.bytes() {
        let v = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' => break,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).unwrap_or(0));
        }
    }
    out
}

fn dir_str(dir: SortDir) -> &'static str {
    match dir {
        SortDir::Asc => "asc",
        SortDir::Desc => "desc",
    }
}

/// `encodePipelineItemCursor`: base64url of the cursor JSON.
pub fn encode_pipeline_item_cursor(cursor: &PipelineItemCursor) -> String {
    let json = serde_json::json!({
        "v": cursor.v,
        "sort": cursor.sort.as_str(),
        "dir": dir_str(cursor.dir),
        "nullRank": cursor.null_rank,
        "sortValue": cursor.sort_value,
        "id": cursor.id,
    });
    base64url_encode(json.to_string().as_bytes())
}

/// `cursorFor`: keyset cursor resuming right after the row (`sort_value` read as text).
pub fn cursor_for(sort: PipelineItemSort, sort_value: Option<&str>, id: i64) -> String {
    encode_pipeline_item_cursor(&PipelineItemCursor {
        v: 1,
        sort: sort.key,
        dir: sort.direction,
        null_rank: u8::from(sort_value.is_none()),
        sort_value: sort_value.map(str::to_owned),
        id: id.to_string(),
    })
}

/// `decodePipelineItemCursor`: fails with [`invalid_cursor`] on any malformed cursor or
/// one issued for another sort.
pub fn decode_pipeline_item_cursor(
    raw: &str,
    sort: PipelineItemSort,
) -> Result<PipelineItemCursor, Error> {
    let bytes = base64url_decode(raw);
    let text = String::from_utf8_lossy(&bytes);
    let parsed: serde_json::Value = serde_json::from_str(&text).map_err(|_| invalid_cursor())?;
    let obj = parsed.as_object().ok_or_else(invalid_cursor)?;
    let v_ok = obj.get("v").and_then(serde_json::Value::as_f64) == Some(1.0);
    let sort_ok = obj.get("sort").and_then(serde_json::Value::as_str) == Some(sort.key.as_str());
    let dir_ok =
        obj.get("dir").and_then(serde_json::Value::as_str) == Some(dir_str(sort.direction));
    let rank = obj.get("nullRank").and_then(serde_json::Value::as_f64);
    let null_rank = if rank == Some(0.0) {
        Some(0u8)
    } else if rank == Some(1.0) {
        Some(1u8)
    } else {
        None
    };
    let id = obj.get("id").and_then(serde_json::Value::as_str);
    let sort_value = match obj.get("sortValue") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(()),
    };
    let (Some(null_rank), Some(id), Ok(sort_value)) = (null_rank, id, sort_value) else {
        return Err(invalid_cursor());
    };
    let valid = v_ok
        && sort_ok
        && dir_ok
        && is_bigint_id(id)
        && is_sort_value_valid(sort.key, null_rank, sort_value.as_deref());
    if !valid {
        return Err(invalid_cursor());
    }
    Ok(PipelineItemCursor {
        v: 1,
        sort: sort.key,
        dir: sort.direction,
        null_rank,
        sort_value,
        id: id.to_owned(),
    })
}

/// `decodeTimestampCursor`: a newest-first cursor over a NOT NULL timestamp; a null-ranked
/// one is rejected. Returns `(unix ms, id)`.
pub fn decode_timestamp_cursor(raw: &str, sort: PipelineItemSort) -> Result<(i64, i64), Error> {
    let cursor = decode_pipeline_item_cursor(raw, sort)?;
    let (0, Some(text)) = (cursor.null_rank, cursor.sort_value.as_deref()) else {
        return Err(invalid_cursor());
    };
    let ms = timestamp_text_ms(text).ok_or_else(invalid_cursor)?;
    let id = cursor.id.parse::<i64>().map_err(|_| invalid_cursor())?;
    Ok((ms, id))
}

/// The money text of a stored `value` (x 10,000): `12.5000`.
pub fn money_text(scaled: i64) -> String {
    let sign = if scaled < 0 { "-" } else { "" };
    let abs = scaled.unsigned_abs();
    let scale = MONEY_SCALE.unsigned_abs();
    format!("{sign}{}.{:04}", abs / scale, abs % scale)
}

/// The bind value of a numeric cursor text (x 10,000, truncated; a REAL beyond i64).
fn money_bind(text: &str) -> Value {
    let negative = text.starts_with('-');
    let body = text.trim_start_matches('-');
    let (int_part, frac_part) = body.split_once('.').unwrap_or((body, ""));
    let mut frac: String = frac_part.chars().take(4).collect();
    while frac.len() < 4 {
        frac.push('0');
    }
    let scaled = int_part
        .parse::<i64>()
        .ok()
        .and_then(|i| i.checked_mul(MONEY_SCALE))
        .and_then(|i| i.checked_add(frac.parse::<i64>().unwrap_or(0)));
    match scaled {
        Some(v) => Value::Integer(if negative { -v } else { v }),
        None => Value::Real(text.parse::<f64>().unwrap_or(0.0) * 10_000.0),
    }
}

/// The bind value of a valid non-null cursor sort value.
fn sort_value_bind(key: PipelineItemSortKey, text: &str) -> Value {
    match sort_spec(key).cast {
        Cast::Timestamp => timestamp_text_ms(text).map_or(Value::Null, Value::Integer),
        Cast::Numeric => money_bind(text),
        Cast::Int => Value::Integer(text.parse::<i64>().unwrap_or(1)),
    }
}

/// `pipelineItemOrderBy`: the `(nullRank, sortValue, id)` sort terms; NULLS LAST in both
/// directions. The null-rank term is only emitted for keys that can be NULL.
pub fn pipeline_item_order_by(sort: PipelineItemSort) -> Vec<String> {
    let spec = sort_spec(sort.key);
    let dir = dir_str(sort.direction);
    let mut parts = Vec::with_capacity(3);
    if spec.nullable {
        parts.push(format!("({} is null) asc", spec.expression));
    }
    parts.push(format!("{} {dir}", spec.expression));
    parts.push(format!("\"PipelineItem\".\"id\" {dir}"));
    parts
}

/// `pipelineItemAfterCursor`: the strictly-after predicate of the keyset page following
/// `cursor`.
pub fn pipeline_item_after_cursor(sort: PipelineItemSort, cursor: &PipelineItemCursor) -> Sql {
    let spec = sort_spec(sort.key);
    let op = match sort.direction {
        SortDir::Asc => ">",
        SortDir::Desc => "<",
    };
    let e = spec.expression;
    let id = Value::Integer(cursor.id.parse::<i64>().unwrap_or(0));
    let text = match (cursor.null_rank, cursor.sort_value.as_deref()) {
        (0, Some(text)) => text,
        _ => {
            return Sql::new(
                format!("(({e} is null) and \"PipelineItem\".\"id\" {op} ?)"),
                vec![id],
            );
        }
    };
    let value = sort_value_bind(sort.key, text);
    if !spec.nullable {
        return Sql::new(
            format!("(({e}, \"PipelineItem\".\"id\") {op} (?, ?))"),
            vec![value, id],
        );
    }
    Sql::new(
        format!("({e} is null or {e} {op} ? or ({e} = ? and \"PipelineItem\".\"id\" {op} ?))"),
        vec![value.clone(), value, id],
    )
}

/// `pipelineItemSortValueText`: the SQL of the sort value as cursor text (`NULL` stays).
pub fn pipeline_item_sort_value_sql(sort: PipelineItemSort) -> String {
    sort_spec(sort.key).expression.to_owned()
}

/// The cursor text of a sort value read with [`pipeline_item_sort_value_sql`].
pub fn sort_value_text(key: PipelineItemSortKey, value: Option<i64>) -> Option<String> {
    let v = value?;
    Some(match sort_spec(key).cast {
        Cast::Timestamp => iso(v),
        Cast::Numeric => money_text(v),
        Cast::Int => v.to_string(),
    })
}

/// `newestFirstKeyset`: one newest-first keyset page over a NOT NULL timestamp column.
#[derive(Debug, Clone, PartialEq)]
pub struct NewestFirstKeyset {
    pub limit: i64,
    /// `(unix ms, id)` of the decoded cursor.
    pub cursor: Option<(i64, i64)>,
}

impl NewestFirstKeyset {
    /// Clamps the limit and decodes the cursor ([`invalid_cursor`] when malformed).
    pub fn new(limit: i64, cursor: Option<&str>) -> Result<NewestFirstKeyset, Error> {
        let cursor = match cursor {
            Some(raw) => Some(decode_timestamp_cursor(raw, NEWEST_FIRST_SORT)?),
            None => None,
        };
        Ok(NewestFirstKeyset {
            limit: clamp_page_limit(limit),
            cursor,
        })
    }

    /// `limit + 1` (the extra row proves a next page).
    pub fn fetch_limit(&self) -> i64 {
        self.limit + 1
    }

    /// `(timestamp, id) < (cursor)` over the given columns.
    pub fn after(&self, timestamp_col: &str, id_col: &str) -> Option<Sql> {
        self.cursor.map(|(ms, id)| {
            Sql::new(
                format!("({timestamp_col}, {id_col}) < (?, ?)"),
                vec![Value::Integer(ms), Value::Integer(id)],
            )
        })
    }

    /// `ORDER BY` terms.
    pub fn order_by(timestamp_col: &str, id_col: &str) -> String {
        format!("{timestamp_col} desc, {id_col} desc")
    }

    /// Cuts the fetched rows to the page and builds the next cursor.
    pub fn page<R>(
        &self,
        mut rows: Vec<R>,
        key_of: impl Fn(&R) -> (i64, i64),
    ) -> (Vec<R>, Option<String>) {
        let limit = usize::try_from(self.limit).unwrap_or(0);
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = match rows.last() {
            Some(last) if more => {
                let (ms, id) = key_of(last);
                Some(cursor_for(NEWEST_FIRST_SORT, Some(&iso(ms)), id))
            }
            _ => None,
        };
        (rows, next)
    }
}
