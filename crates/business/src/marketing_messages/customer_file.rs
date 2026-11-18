//! `integration-facebook-ads/src/customer-file`: header mapping, the
//! normalizers and the hashed customer-file payload of an audience upload.
//! ponytail: no Unicode NFC step (`String.normalize("NFC")`); input that is
//! already composed (the common case) hashes the same.

use std::collections::HashMap;
use std::sync::LazyLock;

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use regex::Regex;
use sha2::{Digest, Sha256};

/// `CUSTOMER_FILE_KEYS` (schema order).
const KEYS: [&str; 13] = [
    "EMAIL", "PHONE", "FN", "LN", "CT", "ST", "ZIP", "COUNTRY", "DOBY", "DOBM", "DOBD", "GEN",
    "MADID",
];
const MIN_BIRTH_YEAR: i32 = 1900;

/// `CustomerFileColumnMap`: column → header index.
pub type Columns = HashMap<&'static str, usize>;

/// `CustomerFilePayload`.
#[derive(Debug, Clone, PartialEq)]
pub struct Payload {
    pub schema: Vec<&'static str>,
    pub data: Vec<Vec<String>>,
    pub skipped: i64,
}

fn alias(header: &str) -> Option<&'static str> {
    let key: String = header
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| !(c.is_whitespace() || matches!(c, '_' | '-' | '/')))
        .collect();
    Some(match key.as_str() {
        "email" => "EMAIL",
        "phone" | "phonenumber" => "PHONE",
        "fn" | "firstname" => "FN",
        "ln" | "lastname" => "LN",
        "ct" | "city" => "CT",
        "st" | "state" | "province" | "stateprovince" => "ST",
        "zip" | "zipcode" | "postalcode" | "zippostalcode" => "ZIP",
        "country" => "COUNTRY",
        "dob" | "dateofbirth" => "DOB",
        "doby" | "yearofbirth" => "DOBY",
        "gen" | "gender" => "GEN",
        "madid" | "mobileadvertiserid" => "MADID",
        _ => return None,
    })
}

/// `mapCustomerFileHeaders`: `None` without an email or phone column.
pub fn map_headers(headers: &[String]) -> Option<Columns> {
    let mut columns = Columns::new();
    for (index, header) in headers.iter().enumerate() {
        if let Some(column) = alias(header) {
            columns.entry(column).or_insert(index);
        }
    }
    (columns.contains_key("EMAIL") || columns.contains_key("PHONE")).then_some(columns)
}

static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").expect("regex"));
static NON_LETTER_OR_SPACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\p{L}\s]").expect("regex"));
static NON_LETTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\p{L}]").expect("regex"));
static WHITESPACE_RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("regex"));
static ISO_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{4})-(\d{1,2})-(\d{1,2})$").expect("regex"));
static US_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{1,2})/(\d{1,2})/(\d{2}|\d{4})$").expect("regex"));

fn or_none(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn email(raw: &str) -> Option<String> {
    let value = raw.trim().to_lowercase();
    EMAIL.is_match(&value).then_some(value)
}

/// `normalizePhoneNumber` (country as region): E.164 digits of a valid
/// number, else the bare digits unless empty or starting with 0.
fn phone(raw: &str, country: Option<&str>) -> Option<String> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    let region = country.map(str::to_uppercase);
    if let Some(number) =
        crate::contact::phone::parse(value, region.as_deref()).filter(|n| n.is_valid())
    {
        let e164 = number.format().mode(phonenumber::Mode::E164).to_string();
        return Some(e164.trim_start_matches('+').to_owned());
    }
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    (!digits.is_empty() && !digits.starts_with('0')).then_some(digits)
}

fn name(raw: &str) -> Option<String> {
    let value = NON_LETTER_OR_SPACE
        .replace_all(&raw.to_lowercase(), "")
        .into_owned();
    or_none(WHITESPACE_RUN.replace_all(&value, " ").trim().to_owned())
}

fn city(raw: &str) -> Option<String> {
    name(raw).and_then(|n| or_none(n.chars().filter(|c| !c.is_whitespace()).collect()))
}

fn state(raw: &str) -> Option<String> {
    or_none(NON_LETTER.replace_all(&raw.to_lowercase(), "").into_owned())
}

fn zip(raw: &str, country: Option<&str>) -> Option<String> {
    let value: String = raw
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if country == Some("us") {
        let prefix: String = value.chars().take(5).collect();
        return (prefix.len() == 5 && prefix.chars().all(|c| c.is_ascii_digit())).then_some(prefix);
    }
    or_none(value)
}

fn country(raw: &str) -> Option<String> {
    let value = raw.trim().to_lowercase();
    (value.len() == 2 && value.chars().all(|c| c.is_ascii_lowercase())).then_some(value)
}

/// `(doby, dobm, dobd)`.
type Dob = (String, String, String);

fn to_dob(year: i32, month: u32, day: u32, now: DateTime<Utc>) -> Option<Dob> {
    if year < MIN_BIRTH_YEAR || year > now.year() {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month, day)?;
    Some((year.to_string(), format!("{month:02}"), format!("{day:02}")))
}

/// `parseDateOfBirth`: `YYYY-M-D` or `M/D/YY(YY)`.
fn date_of_birth(raw: &str, now: DateTime<Utc>) -> Option<Dob> {
    let value = raw.trim();
    if let Some(c) = ISO_DATE.captures(value) {
        return to_dob(
            c[1].parse().ok()?,
            c[2].parse().ok()?,
            c[3].parse().ok()?,
            now,
        );
    }
    let c = US_DATE.captures(value)?;
    let mut year: i32 = c[3].parse().ok()?;
    if c[3].len() == 2 {
        year += if year > now.year() % 100 { 1900 } else { 2000 };
    }
    to_dob(year, c[1].parse().ok()?, c[2].parse().ok()?, now)
}

fn birth_year(raw: &str, now: DateTime<Utc>) -> Option<String> {
    let value = raw.trim();
    if value.len() != 4 || !value.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let year: i32 = value.parse().ok()?;
    (MIN_BIRTH_YEAR..=now.year())
        .contains(&year)
        .then(|| value.to_owned())
}

fn gender(raw: &str) -> Option<String> {
    match raw.trim().to_lowercase().as_str() {
        "m" | "male" => Some("m".into()),
        "f" | "female" => Some("f".into()),
        _ => None,
    }
}

fn key_present(key: &str, columns: &Columns) -> bool {
    match key {
        "DOBY" => columns.contains_key("DOB") || columns.contains_key("DOBY"),
        "DOBM" | "DOBD" => columns.contains_key("DOB"),
        _ => columns.contains_key(key),
    }
}

fn sha256_hex(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `normalizeRow`.
fn normalize_row(
    row: &[String],
    columns: &Columns,
    now: DateTime<Utc>,
) -> HashMap<&'static str, Option<String>> {
    let cell = |column: &str| {
        columns
            .get(column)
            .and_then(|&i| row.get(i))
            .map_or("", |c| c.trim())
    };
    let country = country(cell("COUNTRY"));
    let dob = date_of_birth(cell("DOB"), now);
    HashMap::from([
        ("EMAIL", email(cell("EMAIL"))),
        ("PHONE", phone(cell("PHONE"), country.as_deref())),
        ("FN", name(cell("FN"))),
        ("LN", name(cell("LN"))),
        ("CT", city(cell("CT"))),
        ("ST", state(cell("ST"))),
        ("ZIP", zip(cell("ZIP"), country.as_deref())),
        (
            "DOBY",
            birth_year(cell("DOBY"), now).or_else(|| dob.as_ref().map(|d| d.0.clone())),
        ),
        ("DOBM", dob.as_ref().map(|d| d.1.clone())),
        ("DOBD", dob.as_ref().map(|d| d.2.clone())),
        ("GEN", gender(cell("GEN"))),
        ("MADID", or_none(cell("MADID").to_lowercase())),
        ("COUNTRY", country),
    ])
}

/// `buildCustomerFilePayload`: SHA-256 of every normalized value but MADID;
/// a row without email and phone is skipped.
pub fn build_payload(rows: &[Vec<String>], columns: &Columns, now: DateTime<Utc>) -> Payload {
    let schema: Vec<&'static str> = KEYS
        .into_iter()
        .filter(|k| key_present(k, columns))
        .collect();
    let mut data = Vec::new();
    let mut skipped = 0;
    for row in rows {
        let values = normalize_row(row, columns, now);
        if values["EMAIL"].is_none() && values["PHONE"].is_none() {
            skipped += 1;
            continue;
        }
        data.push(
            schema
                .iter()
                .map(|key| match &values[key] {
                    None => String::new(),
                    Some(v) if *key == "MADID" => v.clone(),
                    Some(v) => sha256_hex(v),
                })
                .collect(),
        );
    }
    Payload {
        schema,
        data,
        skipped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn hashes_the_normalized_rows_and_skips_rows_without_an_identifier() {
        let columns = map_headers(&strings(&["Email", "First Name", "Ignored column"])).unwrap();
        let rows = vec![
            strings(&["  Alice@Example.COM ", "Alice", "x"]),
            strings(&["not-an-email", "Bob", "y"]),
            strings(&["carol@example.com", "Carol-Ann 2", "z"]),
        ];
        let payload = build_payload(&rows, &columns, Utc::now());
        assert_eq!(payload.schema, vec!["EMAIL", "FN"]);
        assert_eq!(payload.skipped, 1);
        assert_eq!(
            payload.data[0],
            vec![sha256_hex("alice@example.com"), sha256_hex("alice")]
        );
        assert_eq!(payload.data[1][1], sha256_hex("carolann"));
        assert!(map_headers(&strings(&["Name", "City"])).is_none());
    }

    #[test]
    fn normalizes_dates_and_zips() {
        let now = Utc::now();
        assert_eq!(
            date_of_birth("1990-2-3", now),
            Some(("1990".into(), "02".into(), "03".into()))
        );
        assert_eq!(date_of_birth("2/30/1990", now), None);
        assert_eq!(zip("12345-6789", Some("us")), Some("12345".into()));
        assert_eq!(phone("0123", None), None);
        assert_eq!(phone("+1 650 253 0000", None), Some("16502530000".into()));
    }
}
