//! `contact/extract-contact.ts`: phone number and email found in free text
//! (the PHP `parsePhoneNumber` / `hasPhoneNumber` port).

use std::sync::LazyLock;

use phonenumber::{Mode, PhoneNumber, Type};
use regex::Regex;

use super::phone::parse;

static EMAIL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap());
/// `DIGIT_RUN_RE`: at least 8 digits with separators between them.
static DIGIT_RUN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)(?:[-_.+()\s]*\d){8,}").unwrap());
static STRIP_CHARS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[-.,_\s()\t\n\r]").unwrap());
static MULTI_PLUS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\++").unwrap());
static VN_TEN_DIGIT_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)(\d{10})\D").unwrap());
static COMMON_PHONE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)[+]?[(]?[0-9]{3}[)]?[-\s.]?[0-9]{3}[-\s.]?[0-9]{4,6}\b").unwrap()
});

/// `FALLBACK_REGIONS`.
const FALLBACK_REGIONS: [&str; 2] = ["VN", "US"];
/// `MIN_DIGITS`.
const MIN_DIGITS: usize = 8;

/// `ExtractedContactInfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedContactInfo {
    /// E.164.
    pub phone_number: Option<String>,
    /// Trimmed, lower-cased.
    pub email: Option<String>,
}

/// `ExtractContactOptions`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtractOptions {
    pub skip_phone: bool,
    pub skip_email: bool,
}

fn clean_phone(raw: &str) -> String {
    MULTI_PLUS_RE
        .replace_all(&STRIP_CHARS_RE.replace_all(raw, ""), "+")
        .into_owned()
}

fn try_regions(text: &str, regions: &[String]) -> Option<(PhoneNumber, String)> {
    if text.chars().count() < MIN_DIGITS {
        return None;
    }
    for raw in DIGIT_RUN_RE.find_iter(text) {
        let raw = raw.as_str();
        let cleaned = clean_phone(raw);
        for region in regions {
            let phone = if region == "VN" && cleaned.chars().count() >= 10 {
                VN_TEN_DIGIT_RE
                    .captures(raw)
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().to_owned())
                    .unwrap_or_else(|| cleaned.clone())
            } else {
                cleaned.clone()
            };
            let Some(parsed) = parse(&phone, Some(region)) else {
                continue;
            };
            if parsed.number_type(&phonenumber::metadata::DATABASE) == Type::FixedLine
                && region != "VN"
            {
                continue;
            }
            // ponytail: `isPossible` is implied by `isValid` (the crate has no
            // separate length check).
            if !parsed.is_valid() {
                continue;
            }
            return Some((parsed, phone));
        }
    }
    None
}

/// `parsePhoneNumber`: the strict digit-run scan, then the loose
/// common-format fallback (its last match).
fn parse_phone_number(text: &str, region: Option<&str>) -> Option<(PhoneNumber, String)> {
    if text.is_empty() {
        return None;
    }
    let mut regions: Vec<String> = region.map(str::to_owned).into_iter().collect();
    regions.extend(FALLBACK_REGIONS.iter().map(|r| (*r).to_owned()));
    try_regions(text, &regions).or_else(|| {
        COMMON_PHONE_RE
            .find_iter(text)
            .last()
            .and_then(|common| try_regions(common.as_str(), &regions))
    })
}

/// `extractContactInfo` (phone forced to E.164).
pub fn extract_contact_info(
    text: Option<&str>,
    default_country: Option<&str>,
    options: ExtractOptions,
) -> ExtractedContactInfo {
    let Some(text) = text.filter(|t| t.chars().count() >= 5) else {
        return ExtractedContactInfo::default();
    };
    let mut result = ExtractedContactInfo::default();
    if !options.skip_email {
        result.email = EMAIL_RE
            .find(text)
            .map(|m| m.as_str().trim().to_lowercase());
    }
    if !options.skip_phone {
        result.phone_number = parse_phone_number(text, default_country)
            .map(|(number, _)| number.format().mode(Mode::E164).to_string());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_vietnamese_mobile_and_an_email() {
        let info = extract_contact_info(
            Some("sđt của mình 0912 345 678, mail An.Nguyen@example.com"),
            None,
            ExtractOptions::default(),
        );
        assert_eq!(info.phone_number.as_deref(), Some("+84912345678"));
        assert_eq!(info.email.as_deref(), Some("an.nguyen@example.com"));
    }

    #[test]
    fn a_short_number_is_not_a_phone() {
        let info =
            extract_contact_info(Some("mình muốn mua 2 cái"), None, ExtractOptions::default());
        assert_eq!(info, ExtractedContactInfo::default());
    }
}
