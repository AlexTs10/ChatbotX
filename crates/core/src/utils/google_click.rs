//! `packages/utils/src/google-click.ts`: Google Ads click-to-message click
//! ids (WhatsApp invisible characters, Messenger/Zalo `gclid:<id>` refs) and
//! the Google Ads conversion helpers. Run offsets are byte offsets in the
//! Rust string.

use std::sync::OnceLock;

use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::js;

string_enum! {
    /// `googleAdsChannels`.
    pub enum GoogleAdsChannel {
        Whatsapp = "whatsapp",
        Messenger = "messenger",
        Zalo = "zalo",
    }
}

/// `GOOGLE_ADS_CHANNEL_VALUES`.
pub const GOOGLE_ADS_CHANNEL_VALUES: &[&str] = &["whatsapp", "messenger", "zalo"];

string_enum! {
    /// `GoogleClickCaptureSource`.
    pub enum GoogleClickCaptureSource {
        Payload = "payload",
        ProfileLookup = "profileLookup",
    }
}

/// `resolveGoogleClickCaptureSource`: `None` for a channel that cannot
/// carry a Google click.
pub fn resolve_google_click_capture_source(channel: &str) -> Option<GoogleClickCaptureSource> {
    match GoogleAdsChannel::parse(channel)? {
        GoogleAdsChannel::Whatsapp | GoogleAdsChannel::Messenger => {
            Some(GoogleClickCaptureSource::Payload)
        }
        GoogleAdsChannel::Zalo => Some(GoogleClickCaptureSource::ProfileLookup),
    }
}

/// `GOOGLE_CLICK_REFERRAL_KEYS`.
pub const GOOGLE_CLICK_REFERRAL_KEYS: &[&str] = &[
    "gclid",
    "gbraid",
    "googleCampaignId",
    "googleAdGroupId",
    "googleAdId",
    "googleClickReceivedAt",
];

/// `GOOGLE_INVISIBLE_CHAR_SET`.
pub const GOOGLE_INVISIBLE_CHAR_SET: &str =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_{}'\",:-";
/// `GOOGLE_INVISIBLE_BASE_CODE_POINT`.
pub const GOOGLE_INVISIBLE_BASE_CODE_POINT: u32 = 0xE0100;
/// `GOOGLE_INVISIBLE_END_CODE_POINT` (exclusive).
pub const GOOGLE_INVISIBLE_END_CODE_POINT: u32 = GOOGLE_INVISIBLE_BASE_CODE_POINT + 70;

/// `INVISIBLE_RUN_PATTERN`: a maximal run of the invisible alphabet.
pub fn invisible_run_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("[\u{E0100}-\u{E0145}]+").expect("valid regex"))
}

fn click_id_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z0-9_-]{10,512}$").expect("valid regex"))
}

string_enum! {
    /// `GoogleClickIdType`.
    pub enum GoogleClickIdType {
        Gclid = "gclid",
        Gbraid = "gbraid",
    }
}

/// `GoogleClickPayload`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleClickPayload {
    pub click_id_type: GoogleClickIdType,
    pub click_id: String,
    pub campaign_id: Option<String>,
    pub ad_group_id: Option<String>,
    pub ad_id: Option<String>,
}

/// A decoded run (byte offsets).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsumedRun {
    pub start: usize,
    pub end: usize,
}

/// `DecodedGoogleClick`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DecodedGoogleClick {
    pub payload: Option<GoogleClickPayload>,
    pub consumed: Vec<ConsumedRun>,
}

fn decode_run_to_text(run: &str) -> String {
    let set: Vec<char> = GOOGLE_INVISIBLE_CHAR_SET.chars().collect();
    run.chars()
        .filter_map(|c| set.get((c as u32).wrapping_sub(GOOGLE_INVISIBLE_BASE_CODE_POINT) as usize))
        .collect()
}

fn optional_id(value: Option<&Value>) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new("^[0-9]{1,20}$").expect("valid regex"));
    let id = match value? {
        Value::Number(n) => {
            let f = n.as_f64()?;
            if f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0 {
                js::number_to_string(f)
            } else {
                return None;
            }
        }
        Value::String(s) => s.clone(),
        _ => return None,
    };
    re.is_match(&id).then_some(id)
}

fn build_payload(raw: &Map<String, Value>) -> Option<GoogleClickPayload> {
    let fields: Map<String, Value> = raw
        .iter()
        .map(|(k, v)| (k.to_lowercase(), v.clone()))
        .collect();
    for (click_id_type, key) in [
        (GoogleClickIdType::Gclid, "gclid"),
        (GoogleClickIdType::Gbraid, "gbraid"),
    ] {
        if let Some(Value::String(click_id)) = fields.get(key) {
            if click_id_re().is_match(click_id) {
                return Some(GoogleClickPayload {
                    click_id_type,
                    click_id: click_id.clone(),
                    campaign_id: optional_id(fields.get("campaignid")),
                    ad_group_id: optional_id(fields.get("adgroupid")),
                    ad_id: optional_id(fields.get("adid"))
                        .or_else(|| optional_id(fields.get("creativeid"))),
                });
            }
        }
    }
    None
}

fn decode_runs(runs: &[(usize, &str)]) -> Option<DecodedGoogleClick> {
    let joined: String = runs.iter().map(|(_, run)| *run).collect();
    let fields = match serde_json::from_str::<Value>(&decode_run_to_text(&joined)).ok()? {
        Value::Object(map) => map,
        _ => return None,
    };
    let payload = build_payload(&fields)?;
    Some(DecodedGoogleClick {
        payload: Some(payload),
        consumed: runs
            .iter()
            .map(|(start, run)| ConsumedRun {
                start: *start,
                end: start + run.len(),
            })
            .collect(),
    })
}

/// `decodeInvisibleGoogleClick`: every run joined first, then each run
/// alone (longest first).
pub fn decode_invisible_google_click(text: &str) -> DecodedGoogleClick {
    let runs: Vec<(usize, &str)> = invisible_run_pattern()
        .find_iter(text)
        .map(|m| (m.start(), m.as_str()))
        .collect();
    if runs.is_empty() {
        return DecodedGoogleClick::default();
    }
    let mut candidates: Vec<Vec<(usize, &str)>> = vec![runs.clone()];
    if runs.len() > 1 {
        let mut by_length = runs.clone();
        by_length.sort_by_key(|a| std::cmp::Reverse(a.1.chars().count()));
        candidates.extend(by_length.into_iter().map(|run| vec![run]));
    }
    candidates
        .iter()
        .find_map(|candidate| decode_runs(candidate))
        .unwrap_or_default()
}

/// `stripConsumedRuns`: removes only the decoded runs.
pub fn strip_consumed_runs(text: &str, consumed: &[ConsumedRun]) -> String {
    let mut runs = consumed.to_vec();
    runs.sort_by_key(|a| std::cmp::Reverse(a.start));
    runs.into_iter()
        .fold(text.to_owned(), |kept, ConsumedRun { start, end }| {
            match (kept.get(..start), kept.get(end..)) {
                (Some(head), Some(tail)) => format!("{head}{tail}"),
                _ => kept,
            }
        })
}

/// `parseGoogleClickRef`: `gclid:<id>,campaignid:1,…` (keys
/// case-insensitive, URL-decoded, first value wins).
pub fn parse_google_click_ref(reference: &str) -> Option<GoogleClickPayload> {
    let decoded = js::decode_uri_component(reference).unwrap_or_else(|_| reference.to_owned());
    let mut fields = Map::new();
    for pair in decoded.split(',') {
        if let Some(separator) = pair.find(':').filter(|i| *i > 0) {
            let key = js::trim(&pair[..separator]).to_lowercase();
            if !fields.contains_key(&key) {
                fields.insert(
                    key,
                    Value::String(js::trim(&pair[separator + 1..]).to_owned()),
                );
            }
        }
    }
    build_payload(&fields)
}

/// `GoogleClickReferral`: the six `ContactInbox.referral` keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleClickReferral {
    pub gclid: Option<String>,
    pub gbraid: Option<String>,
    pub google_campaign_id: Option<String>,
    pub google_ad_group_id: Option<String>,
    pub google_ad_id: Option<String>,
    pub google_click_received_at: String,
}

/// `toGoogleClickReferral`: `received_at_ms` `None` (an Invalid Date) is
/// stamped `now_ms`.
pub fn to_google_click_referral(
    payload: &GoogleClickPayload,
    received_at_ms: Option<i64>,
    now_ms: i64,
) -> GoogleClickReferral {
    let stamp = received_at_ms
        .and_then(js::to_iso_string)
        .or_else(|| js::to_iso_string(now_ms))
        .unwrap_or_default();
    GoogleClickReferral {
        gclid: (payload.click_id_type == GoogleClickIdType::Gclid)
            .then(|| payload.click_id.clone()),
        gbraid: (payload.click_id_type == GoogleClickIdType::Gbraid)
            .then(|| payload.click_id.clone()),
        google_campaign_id: payload.campaign_id.clone(),
        google_ad_group_id: payload.ad_group_id.clone(),
        google_ad_id: payload.ad_id.clone(),
        google_click_received_at: stamp,
    }
}

/// `GoogleClickCapture`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoogleClickCapture {
    /// The ref to keep routing on: `None` only when a Google click consumed it.
    pub reference: Option<String>,
    pub google_referral: Option<GoogleClickReferral>,
}

/// `consumeGoogleClickRef`.
pub fn consume_google_click_ref(
    reference: Option<&str>,
    received_at_ms: Option<i64>,
    now_ms: i64,
) -> GoogleClickCapture {
    let payload = reference
        .filter(|r| !r.is_empty())
        .and_then(parse_google_click_ref);
    match payload {
        Some(payload) => GoogleClickCapture {
            reference: None,
            google_referral: Some(to_google_click_referral(&payload, received_at_ms, now_ms)),
        },
        None => GoogleClickCapture {
            reference: reference.map(str::to_owned),
            google_referral: None,
        },
    }
}

/// `extractInvisibleGoogleClick`'s result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedGoogleClick {
    pub text: String,
    pub google_referral: Option<GoogleClickReferral>,
}

/// `extractInvisibleGoogleClick`.
pub fn extract_invisible_google_click(
    text: &str,
    received_at_ms: Option<i64>,
    now_ms: i64,
) -> ExtractedGoogleClick {
    let decoded = decode_invisible_google_click(text);
    match decoded.payload {
        Some(payload) => ExtractedGoogleClick {
            text: strip_consumed_runs(text, &decoded.consumed),
            google_referral: Some(to_google_click_referral(&payload, received_at_ms, now_ms)),
        },
        None => ExtractedGoogleClick {
            text: text.to_owned(),
            google_referral: None,
        },
    }
}

/// The click ids a referral carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GoogleClickIds<'a> {
    pub gclid: Option<&'a str>,
    pub gbraid: Option<&'a str>,
}

/// Something with `gclid` / `gbraid` (a referral struct or its jsonb).
pub trait HasGoogleClickIds {
    fn click_ids(&self) -> GoogleClickIds<'_>;
}

impl HasGoogleClickIds for GoogleClickReferral {
    fn click_ids(&self) -> GoogleClickIds<'_> {
        GoogleClickIds {
            gclid: self.gclid.as_deref(),
            gbraid: self.gbraid.as_deref(),
        }
    }
}

impl HasGoogleClickIds for GoogleClickIds<'_> {
    fn click_ids(&self) -> GoogleClickIds<'_> {
        *self
    }
}

impl HasGoogleClickIds for Value {
    fn click_ids(&self) -> GoogleClickIds<'_> {
        GoogleClickIds {
            gclid: self.get("gclid").and_then(Value::as_str),
            gbraid: self.get("gbraid").and_then(Value::as_str),
        }
    }
}

/// `hasGoogleClick`.
pub fn has_google_click<C: HasGoogleClickIds + ?Sized>(referral: Option<&C>) -> bool {
    read_google_click(referral).is_some()
}

/// `readGoogleClick`: gclid first, then gbraid (empty ids do not count).
pub fn read_google_click<C: HasGoogleClickIds + ?Sized>(
    referral: Option<&C>,
) -> Option<(GoogleClickIdType, String)> {
    let ids = referral?.click_ids();
    if let Some(gclid) = ids.gclid.filter(|g| !g.is_empty()) {
        return Some((GoogleClickIdType::Gclid, gclid.to_owned()));
    }
    ids.gbraid
        .filter(|g| !g.is_empty())
        .map(|gbraid| (GoogleClickIdType::Gbraid, gbraid.to_owned()))
}

/// `withoutRepeatedGoogleClick`: `next`, unless it has no click or is the
/// click already stored on `existing`.
pub fn without_repeated_google_click<'a, E, T>(
    existing: Option<&E>,
    next: Option<&'a T>,
) -> Option<&'a T>
where
    E: HasGoogleClickIds + ?Sized,
    T: HasGoogleClickIds + ?Sized,
{
    let incoming = read_google_click(next)?;
    let stored = read_google_click(existing);
    if stored.as_ref() == Some(&incoming) {
        None
    } else {
        next
    }
}

const LEAD_LIKE_CATEGORIES: &[&str] = &[
    "QUALIFIED_LEAD",
    "CONVERTED_LEAD",
    "SUBMIT_LEAD_FORM",
    "CONTACT",
    "SIGNUP",
    "BOOK_APPOINTMENT",
    "REQUEST_QUOTE",
    "IMPORTED_LEAD",
];

/// `isLeadLikeCategory`.
pub fn is_lead_like_category(category: &str) -> bool {
    LEAD_LIKE_CATEGORIES.contains(&category)
}

fn ten_digits(value: &str) -> bool {
    value.len() == 10 && value.bytes().all(|b| b.is_ascii_digit())
}

/// `formatCustomerId`: `1234567890` → `123-456-7890`.
pub fn format_customer_id(customer_id: &str) -> String {
    if ten_digits(customer_id) {
        format!(
            "{}-{}-{}",
            &customer_id[..3],
            &customer_id[3..6],
            &customer_id[6..]
        )
    } else {
        customer_id.to_owned()
    }
}

/// `customerIdSchema.parse`: dashes and whitespace removed, then 10 digits.
pub fn parse_customer_id(value: &str) -> Option<String> {
    let cleaned: String = value
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .collect();
    ten_digits(&cleaned).then_some(cleaned)
}

string_enum! {
    /// `googleAdsCustomerTypes`.
    pub enum GoogleAdsCustomerType {
        New = "NEW",
        Returning = "RETURNING",
    }
}

string_enum! {
    /// `googleAdsCustomerValueBuckets`.
    pub enum GoogleAdsCustomerValueBucket {
        Low = "LOW",
        Medium = "MEDIUM",
        High = "HIGH",
    }
}

/// `isGoogleAdsMatchTemplate`: exactly one `{{variable}}`, ≤ 200 chars.
pub fn is_google_ads_match_template(value: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\{\{[^{}]{1,196}\}\}$").expect("valid regex"))
        .is_match(value)
}

fn rfc3339_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^([0-9]{4})-([0-9]{2})-([0-9]{2})[Tt]([0-9]{2}):([0-9]{2}):([0-9]{2})(?:\.[0-9]+)?(?:[Zz]|([+-])([0-9]{2}):([0-9]{2}))$")
            .expect("valid regex")
    })
}

fn parse_rfc3339(value: &str) -> Option<i64> {
    let caps = rfc3339_re().captures(value)?;
    let n = |i: usize| {
        caps.get(i)
            .map_or(0, |m| m.as_str().parse::<i64>().unwrap_or(0))
    };
    let (year, month, day, hour, minute, second) = (n(1), n(2), n(3), n(4), n(5), n(6));
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    let (offset_hour, offset_minute) = (n(8), n(9));
    if offset_hour > 23 || offset_minute > 59 {
        return None;
    }
    if !(100..).contains(&year) {
        return None;
    }
    let wall = js::date_utc(year, month - 1, day, hour, minute, second, 0);
    let calendar = js::utc_fields(wall);
    if calendar.year != year || i64::from(calendar.month) != month || i64::from(calendar.day) != day
    {
        return None;
    }
    let offset = offset_hour * 60 + offset_minute;
    let signed = if caps.get(7).map(|m| m.as_str()) == Some("-") {
        -offset
    } else {
        offset
    };
    Some(wall - signed * 60_000)
}

/// `hasRfc3339Shape`.
pub fn has_rfc3339_shape(value: &str) -> bool {
    rfc3339_re().is_match(value)
}

/// `isRfc3339WithZone`: a real calendar date-time with `Z` or `±hh:mm`.
pub fn is_rfc3339_with_zone(value: &str) -> bool {
    parse_rfc3339(value).is_some()
}

/// `parseConversionTime`: the instant (whole seconds), or `None`.
pub fn parse_conversion_time(value: &str) -> Option<i64> {
    parse_rfc3339(value)
}

/// `googleAdsConversionErrorCodes`, in declaration order.
pub const GOOGLE_ADS_CONVERSION_ERROR_CODES: &[(&str, &str)] = &[
    ("unsupportedChannel", "google_ads_unsupported_channel"),
    ("noClick", "google_ads_no_click"),
    ("noAccount", "google_ads_no_account"),
    (
        "unknownConversionAction",
        "google_ads_unknown_conversion_action",
    ),
    ("actionDisabled", "google_ads_action_disabled"),
    ("incompatibleAction", "google_ads_incompatible_action"),
    ("unsupportedAction", "google_ads_unsupported_action"),
    ("invalidValue", "google_ads_invalid_value"),
    ("invalidInput", "google_ads_invalid_input"),
    ("recordFailed", "google_ads_record_failed"),
    ("invalidConsentConfig", "google_ads_invalid_consent_config"),
    ("invalidConsentValue", "google_ads_invalid_consent_value"),
    (
        "invalidConversionTime",
        "google_ads_invalid_conversion_time",
    ),
    ("missingDedupId", "google_ads_missing_dedup_id"),
    ("invalidDedupId", "google_ads_invalid_dedup_id"),
    ("missingOccurrenceKey", "google_ads_missing_occurrence_key"),
    (
        "invalidCustomerProperty",
        "google_ads_invalid_customer_property",
    ),
];

/// `googleAdsConversionErrorCodes[name]`.
pub fn google_ads_conversion_error_code(name: &str) -> Option<&'static str> {
    GOOGLE_ADS_CONVERSION_ERROR_CODES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, code)| *code)
}

/// `isEnabledConversionAction`.
pub fn is_enabled_conversion_action(status: &str) -> bool {
    status == "ENABLED"
}

/// `isExternalAttributionAction`.
pub fn is_external_attribution_action(attribution_model: Option<&str>) -> bool {
    attribution_model == Some("EXTERNAL")
}

/// `isOnePerClickAction`.
pub fn is_one_per_click_action(counting_type: &str) -> bool {
    counting_type == "ONE_PER_CLICK"
}

/// `isSendableConversionAction`.
pub fn is_sendable_conversion_action(status: &str, attribution_model: Option<&str>) -> bool {
    is_enabled_conversion_action(status) && !is_external_attribution_action(attribution_model)
}
