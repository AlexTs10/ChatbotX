//! Port of `packages/utils/__tests__/google-click.test.ts`. `Date` values
//! are epoch ms (`new Date(NaN)` is `None`); run offsets are byte offsets.

mod support;

use chatbotx_core::utils::google_click::{
    GOOGLE_ADS_CHANNEL_VALUES, GOOGLE_ADS_CONVERSION_ERROR_CODES, GOOGLE_CLICK_REFERRAL_KEYS,
    GOOGLE_INVISIBLE_BASE_CODE_POINT, GOOGLE_INVISIBLE_CHAR_SET, GOOGLE_INVISIBLE_END_CODE_POINT,
    GoogleAdsChannel, GoogleClickCapture, GoogleClickCaptureSource, GoogleClickIdType,
    GoogleClickIds, GoogleClickPayload, GoogleClickReferral, consume_google_click_ref,
    decode_invisible_google_click, extract_invisible_google_click, format_customer_id,
    google_ads_conversion_error_code, has_google_click, has_rfc3339_shape, invisible_run_pattern,
    is_google_ads_match_template, is_lead_like_category, is_rfc3339_with_zone,
    parse_conversion_time, parse_customer_id, parse_google_click_ref,
    resolve_google_click_capture_source, strip_consumed_runs, to_google_click_referral,
    without_repeated_google_click,
};
use serde_json::json;

const GOOGLE_DOC_TEST_URL_TEXT: &str = "%F3%A0%84%BF%F3%A0%85%82%F3%A0%84%A0%F3%A0%84%9C%F3%A0%84%A5%F3%A0%84%A2%F3%A0%84%9D%F3%A0%85%82%F3%A0%85%84%F3%A0%85%82%F3%A0%84%80%F3%A0%84%81%F3%A0%84%82%F3%A0%84%83%F3%A0%84%84%F3%A0%84%85%F3%A0%84%86%F3%A0%84%87%F3%A0%84%88%F3%A0%84%89%F3%A0%84%8A%F3%A0%84%8B%F3%A0%84%8C%F3%A0%84%8D%F3%A0%84%8E%F3%A0%84%8F%F3%A0%84%90%F3%A0%84%91%F3%A0%84%92%F3%A0%84%93%F3%A0%84%94%F3%A0%84%95%F3%A0%84%96%F3%A0%84%97%F3%A0%84%98%F3%A0%84%99%F3%A0%84%9A%F3%A0%84%9B%F3%A0%84%9C%F3%A0%84%9D%F3%A0%84%9E%F3%A0%84%9F%F3%A0%84%A0%F3%A0%84%A1%F3%A0%84%A2%F3%A0%84%A3%F3%A0%84%A4%F3%A0%84%A5%F3%A0%84%A6%F3%A0%84%A7%F3%A0%84%A8%F3%A0%84%A9%F3%A0%84%AA%F3%A0%84%AB%F3%A0%84%AC%F3%A0%84%AD%F3%A0%84%AE%F3%A0%84%AF%F3%A0%84%B0%F3%A0%84%B1%F3%A0%84%B2%F3%A0%84%B3%F3%A0%84%B4%F3%A0%84%B5%F3%A0%84%B6%F3%A0%84%B7%F3%A0%84%B8%F3%A0%84%B9%F3%A0%84%BA%F3%A0%84%BB%F3%A0%84%BC%F3%A0%84%BD%F3%A0%84%BE%F3%A0%85%82%F3%A0%85%80";
const GCLID: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_";

fn encode(text: &str) -> String {
    text.chars()
        .map(|c| {
            let index = GOOGLE_INVISIBLE_CHAR_SET
                .chars()
                .position(|x| x == c)
                .map_or(-1i64, |i| i as i64);
            char::from_u32((i64::from(GOOGLE_INVISIBLE_BASE_CODE_POINT) + index) as u32).unwrap()
        })
        .collect()
}

fn hide(value: serde_json::Value) -> String {
    encode(&chatbotx_core::js::json_stringify(&value))
}

fn received_at() -> i64 {
    support::iso_ms("2026-10-01T00:00:00.000Z")
}

fn payload(kind: GoogleClickIdType, id: &str) -> GoogleClickPayload {
    GoogleClickPayload {
        click_id_type: kind,
        click_id: id.to_owned(),
        campaign_id: None,
        ad_group_id: None,
        ad_id: None,
    }
}

fn ids<'a>(gclid: Option<&'a str>, gbraid: Option<&'a str>) -> GoogleClickIds<'a> {
    GoogleClickIds { gclid, gbraid }
}

fn s(value: &str) -> Option<String> {
    Some(value.to_owned())
}

/// Port of: packages/utils/__tests__/google-click.test.ts:47 "invisible alphabet > has 70 unique characters and the documented code point range"
#[test]
fn invisible_alphabet_has_70_unique_characters_and_the_documented_code_point_range() {
    assert_eq!(GOOGLE_INVISIBLE_CHAR_SET.chars().count(), 70);
    assert_eq!(
        GOOGLE_INVISIBLE_CHAR_SET
            .chars()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        70
    );
    assert_eq!(GOOGLE_INVISIBLE_BASE_CODE_POINT, 0xE0100);
    assert_eq!(GOOGLE_INVISIBLE_END_CODE_POINT, 0xE0146);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:56 "decodeInvisibleGoogleClick > joins a payload split across runs inside the text, like Google's reference decoder"
#[test]
fn decodeinvisiblegoogleclick_joins_a_payload_split_across_runs_inside_the_text_like_google_s_reference_decoder()
 {
    let json = chatbotx_core::js::json_stringify(&json!({ "gclid": GCLID, "campaignid": 123 }));
    let half = json.len() / 2;
    let text = format!(
        "{}Hello {}world",
        encode(&json[..half]),
        encode(&json[half..])
    );
    let decoded = decode_invisible_google_click(&text);
    let p = decoded.payload.clone().unwrap();
    assert_eq!(p.click_id_type, GoogleClickIdType::Gclid);
    assert_eq!(p.click_id, GCLID);
    assert_eq!(p.campaign_id, s("123"));
    assert_eq!(decoded.consumed.len(), 2);
    assert_eq!(strip_consumed_runs(&text, &decoded.consumed), "Hello world");
}

/// Port of: packages/utils/__tests__/google-click.test.ts:70 "decodeInvisibleGoogleClick > falls back to a single run when joining all runs is not a payload"
#[test]
fn decodeinvisiblegoogleclick_falls_back_to_a_single_run_when_joining_all_runs_is_not_a_payload() {
    let unrelated = encode("zz");
    let text = format!("{unrelated}Hi {}", hide(json!({ "gclid": GCLID })));
    let decoded = decode_invisible_google_click(&text);
    assert_eq!(decoded.payload.unwrap().click_id, GCLID);
    assert_eq!(
        strip_consumed_runs(&text, &decoded.consumed),
        format!("{unrelated}Hi ")
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:78 "decodeInvisibleGoogleClick > decodes Google's documented gclid vector"
#[test]
fn decodeinvisiblegoogleclick_decodes_google_s_documented_gclid_vector() {
    let text = format!(
        "{}Hello, how can I help you",
        hide(json!({ "gclid": GCLID }))
    );
    let decoded = decode_invisible_google_click(&text);
    assert_eq!(
        decoded.payload,
        Some(payload(GoogleClickIdType::Gclid, GCLID))
    );
    assert_eq!(
        strip_consumed_runs(&text, &decoded.consumed),
        "Hello, how can I help you"
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:91 "decodeInvisibleGoogleClick > decodes the literal test URL from Google's documentation"
#[test]
fn decodeinvisiblegoogleclick_decodes_the_literal_test_url_from_google_s_documentation() {
    let text = format!(
        "{}Hello, how can I help you",
        chatbotx_core::js::decode_uri_component(GOOGLE_DOC_TEST_URL_TEXT).unwrap()
    );
    assert_eq!(
        decode_invisible_google_click(&text)
            .payload
            .map(|p| p.click_id),
        s(GCLID)
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:96 "decodeInvisibleGoogleClick > reads the gclid test URL of the Nov 2025 document, whose ad ids are camelCase"
#[test]
fn decodeinvisiblegoogleclick_reads_the_gclid_test_url_of_the_nov_2025_document_whose_ad_ids_are_camelcase()
 {
    let id = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_";
    let text = format!(
        "{}Hello, how can I help you",
        encode(&format!(
            r#"{{"gclid":"{id}","campaignId":123,"adGroupId":456,"creativeId":789}}"#
        ))
    );
    let decoded = decode_invisible_google_click(&text);
    assert_eq!(
        decoded.payload,
        Some(GoogleClickPayload {
            click_id_type: GoogleClickIdType::Gclid,
            click_id: id.to_owned(),
            campaign_id: s("123"),
            ad_group_id: s("456"),
            ad_id: s("789"),
        })
    );
    assert_eq!(
        strip_consumed_runs(&text, &decoded.consumed),
        "Hello, how can I help you"
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:96 "decodeInvisibleGoogleClick > reads the gbraid test URL of the Nov 2025 document, whose ad ids are camelCase"
#[test]
fn decodeinvisiblegoogleclick_reads_the_gbraid_test_url_of_the_nov_2025_document_whose_ad_ids_are_camelcase()
 {
    let id = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_";
    let text = format!(
        "{}Hello, how can I help you",
        encode(&format!(
            r#"{{"gbraid":"{id}","campaignId":123,"adGroupId":456,"creativeId":789}}"#
        ))
    );
    let decoded = decode_invisible_google_click(&text);
    assert_eq!(
        decoded.payload,
        Some(GoogleClickPayload {
            click_id_type: GoogleClickIdType::Gbraid,
            click_id: id.to_owned(),
            campaign_id: s("123"),
            ad_group_id: s("456"),
            ad_id: s("789"),
        })
    );
    assert_eq!(
        strip_consumed_runs(&text, &decoded.consumed),
        "Hello, how can I help you"
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:113 "decodeInvisibleGoogleClick > falls back to creativeId when adid is present but unusable"
#[test]
fn decodeinvisiblegoogleclick_falls_back_to_creativeid_when_adid_is_present_but_unusable() {
    let text = encode(&format!(
        r#"{{"gclid":"{GCLID}","adid":"abc","creativeId":2}}"#
    ));
    assert_eq!(
        decode_invisible_google_click(&text).payload.unwrap().ad_id,
        s("2")
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:118 "decodeInvisibleGoogleClick > keeps the click, stamped now, when the provider timestamp is out of range"
#[test]
fn decodeinvisiblegoogleclick_keeps_the_click_stamped_now_when_the_provider_timestamp_is_out_of_range()
 {
    let text = encode(&format!(r#"{{"gclid":"{GCLID}"}}"#));
    let extracted = extract_invisible_google_click(&text, None, support::now_ms());
    let referral = extracted.google_referral.unwrap();
    assert_eq!(referral.gclid.as_deref(), Some(GCLID));
    assert!(chatbotx_core::js::parse_date(&referral.google_click_received_at).is_some());
}

/// Port of: packages/utils/__tests__/google-click.test.ts:130 "decodeInvisibleGoogleClick > prefers adid over creativeId when both are present"
#[test]
fn decodeinvisiblegoogleclick_prefers_adid_over_creativeid_when_both_are_present() {
    let text = encode(&format!(r#"{{"gclid":"{GCLID}","adid":1,"creativeId":2}}"#));
    assert_eq!(
        decode_invisible_google_click(&text).payload.unwrap().ad_id,
        s("1")
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:135 "decodeInvisibleGoogleClick > decodes the extended vector with ad identifiers"
#[test]
fn decodeinvisiblegoogleclick_decodes_the_extended_vector_with_ad_identifiers() {
    let id = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_";
    let text = encode(&format!(
        r#"{{"gclid":"{id}","campaignid":123,"adgroupid":456,"adid":789}}"#
    ));
    assert_eq!(
        decode_invisible_google_click(&text).payload,
        Some(GoogleClickPayload {
            click_id_type: GoogleClickIdType::Gclid,
            click_id: id.to_owned(),
            campaign_id: s("123"),
            ad_group_id: s("456"),
            ad_id: s("789"),
        })
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:149 "decodeInvisibleGoogleClick > decodes gbraid"
#[test]
fn decodeinvisiblegoogleclick_decodes_gbraid() {
    let text = encode(&format!(r#"{{"gbraid":"{GCLID}"}}"#));
    assert_eq!(
        decode_invisible_google_click(&text)
            .payload
            .unwrap()
            .click_id_type,
        GoogleClickIdType::Gbraid
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:154 "decodeInvisibleGoogleClick > decodes a run in the middle of visible text and strips only that run"
#[test]
fn decodeinvisiblegoogleclick_decodes_a_run_in_the_middle_of_visible_text_and_strips_only_that_run()
{
    let text = format!("Hi {}there", encode(&format!(r#"{{"gclid":"{GCLID}"}}"#)));
    let decoded = decode_invisible_google_click(&text);
    assert_eq!(decoded.payload.unwrap().click_id, GCLID);
    assert_eq!(strip_consumed_runs(&text, &decoded.consumed), "Hi there");
}

/// Port of: packages/utils/__tests__/google-click.test.ts:161 "decodeInvisibleGoogleClick > returns nothing and leaves text untouched for plain text"
#[test]
fn decodeinvisiblegoogleclick_returns_nothing_and_leaves_text_untouched_for_plain_text() {
    let decoded = decode_invisible_google_click("hello");
    assert_eq!(decoded.payload, None);
    assert_eq!(strip_consumed_runs("hello", &decoded.consumed), "hello");
}

/// Port of: packages/utils/__tests__/google-click.test.ts:167 "decodeInvisibleGoogleClick > does not strip an invisible run that is not valid JSON"
#[test]
fn decodeinvisiblegoogleclick_does_not_strip_an_invisible_run_that_is_not_valid_json() {
    let text = format!("{}hello", encode("not json"));
    let decoded = decode_invisible_google_click(&text);
    assert_eq!(decoded.payload, None);
    assert_eq!(strip_consumed_runs(&text, &decoded.consumed), text);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:174 "decodeInvisibleGoogleClick > rejects a payload whose click id is too short"
#[test]
fn decodeinvisiblegoogleclick_rejects_a_payload_whose_click_id_is_too_short() {
    assert_eq!(
        decode_invisible_google_click(&encode(r#"{"gclid":"short"}"#)).payload,
        None
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:180 "decodeInvisibleGoogleClick > leaves ideographic variation selectors in Japanese text untouched"
#[test]
fn decodeinvisiblegoogleclick_leaves_ideographic_variation_selectors_in_japanese_text_untouched() {
    let text = "葛\u{E0100}城";
    let decoded = decode_invisible_google_click(text);
    assert_eq!(decoded.payload, None);
    assert_eq!(strip_consumed_runs(text, &decoded.consumed), text);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:187 "decodeInvisibleGoogleClick > picks the run that parses when several runs exist"
#[test]
fn decodeinvisiblegoogleclick_picks_the_run_that_parses_when_several_runs_exist() {
    let text = format!(
        "{}x{}",
        encode("zz"),
        encode(&format!(r#"{{"gclid":"{GCLID}"}}"#))
    );
    let decoded = decode_invisible_google_click(&text);
    assert_eq!(decoded.payload.unwrap().click_id, GCLID);
    assert_eq!(
        strip_consumed_runs(&text, &decoded.consumed),
        format!("{}x", encode("zz"))
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:194 "decodeInvisibleGoogleClick > prefers the longest valid run and consumes only that range"
#[test]
fn decodeinvisiblegoogleclick_prefers_the_longest_valid_run_and_consumes_only_that_range() {
    let short = encode(&format!(r#"{{"gclid":"{}"}}"#, &GCLID[..12]));
    let long = encode(&format!(r#"{{"gclid":"{GCLID}"}}"#));
    let text = format!("{short}-{long}");
    let decoded = decode_invisible_google_click(&text);
    assert_eq!(decoded.payload.unwrap().click_id, GCLID);
    assert_eq!(
        strip_consumed_runs(&text, &decoded.consumed),
        format!("{short}-")
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:203 "decodeInvisibleGoogleClick > returns nothing for valid JSON that is not a usable object"
#[test]
fn decodeinvisiblegoogleclick_returns_nothing_for_valid_json_that_is_not_a_usable_object() {
    for json in ["123", r#"["gclid"]"#, "null"] {
        let text = format!("{}hi", encode(json));
        let decoded = decode_invisible_google_click(&text);
        assert_eq!(decoded.payload, None);
        assert_eq!(strip_consumed_runs(&text, &decoded.consumed), text);
    }
}

/// Port of: packages/utils/__tests__/google-click.test.ts:212 "decodeInvisibleGoogleClick > drops non-numeric ad identifiers"
#[test]
fn decodeinvisiblegoogleclick_drops_non_numeric_ad_identifiers() {
    let text = encode(&format!(
        r#"{{"gclid":"{GCLID}","campaignid":-1,"adgroupid":"abc","adid":""}}"#
    ));
    let p = decode_invisible_google_click(&text).payload.unwrap();
    assert_eq!((p.campaign_id, p.ad_group_id, p.ad_id), (None, None, None));
    assert_eq!(
        parse_google_click_ref(&format!("gclid:{GCLID},adid:"))
            .unwrap()
            .ad_id,
        None
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:224 "decodeInvisibleGoogleClick > matches exactly the alphabet code point range"
#[test]
fn decodeinvisiblegoogleclick_matches_exactly_the_alphabet_code_point_range() {
    let last = char::from_u32(GOOGLE_INVISIBLE_END_CODE_POINT - 1).unwrap();
    assert_eq!(last as u32, 0xE0145);
    assert!(invisible_run_pattern().is_match(&last.to_string()));
    let outside = char::from_u32(GOOGLE_INVISIBLE_END_CODE_POINT).unwrap();
    assert!(!invisible_run_pattern().is_match(&outside.to_string()));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:235 "decodeInvisibleGoogleClick > ignores code points just outside the alphabet range"
#[test]
fn decodeinvisiblegoogleclick_ignores_code_points_just_outside_the_alphabet_range() {
    let outside = char::from_u32(GOOGLE_INVISIBLE_END_CODE_POINT)
        .unwrap()
        .to_string();
    assert_eq!(decode_invisible_google_click(&outside).payload, None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:242 "parseGoogleClickRef > parses gclid with extra keys, case-insensitively"
#[test]
fn parsegoogleclickref_parses_gclid_with_extra_keys_case_insensitively() {
    assert_eq!(
        parse_google_click_ref(&format!("gclid:{GCLID},CampaignId:1,adgroupid:2,adid:3")),
        Some(GoogleClickPayload {
            click_id_type: GoogleClickIdType::Gclid,
            click_id: GCLID.to_owned(),
            campaign_id: s("1"),
            ad_group_id: s("2"),
            ad_id: s("3"),
        })
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:254 "parseGoogleClickRef > parses gbraid and URL-encoded refs"
#[test]
fn parsegoogleclickref_parses_gbraid_and_url_encoded_refs() {
    let parsed = parse_google_click_ref(&chatbotx_core::js::encode_uri_component(&format!(
        "gbraid:{GCLID},adid:9"
    )))
    .unwrap();
    assert_eq!(parsed.click_id_type, GoogleClickIdType::Gbraid);
    assert_eq!(parsed.ad_id, s("9"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:260 "parseGoogleClickRef > returns null for an empty id, a legacy raw id or a flow ref"
#[test]
fn parsegoogleclickref_returns_null_for_an_empty_id_a_legacy_raw_id_or_a_flow_ref() {
    assert_eq!(parse_google_click_ref("gclid:"), None);
    assert_eq!(parse_google_click_ref(GCLID), None);
    assert_eq!(parse_google_click_ref("f_abc123"), None);
    assert_eq!(parse_google_click_ref(""), None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:267 "parseGoogleClickRef > survives malformed percent-encoding"
#[test]
fn parsegoogleclickref_survives_malformed_percent_encoding() {
    assert_eq!(parse_google_click_ref("%E0%A4%A"), None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:275 "toGoogleClickReferral / hasGoogleClick > writes all six keys and nulls the other click id"
#[test]
fn togoogleclickreferral_hasgoogleclick_writes_all_six_keys_and_nulls_the_other_click_id() {
    let at = support::iso_ms("2026-10-05T01:02:03.000Z");
    let referral = to_google_click_referral(
        &GoogleClickPayload {
            ad_id: s("7"),
            ..payload(GoogleClickIdType::Gbraid, GCLID)
        },
        Some(at),
        0,
    );
    let value = serde_json::to_value(&referral).unwrap();
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut expected = GOOGLE_CLICK_REFERRAL_KEYS.to_vec();
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(referral.gclid, None);
    assert_eq!(referral.gbraid, s(GCLID));
    assert_eq!(referral.google_ad_id, s("7"));
    assert_eq!(referral.google_campaign_id, None);
    assert_eq!(
        referral.google_click_received_at,
        "2026-10-05T01:02:03.000Z"
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:292 "toGoogleClickReferral / hasGoogleClick > detects a click on either id"
#[test]
fn togoogleclickreferral_hasgoogleclick_detects_a_click_on_either_id() {
    assert!(has_google_click(Some(&ids(Some(GCLID), None))));
    assert!(has_google_click(Some(&ids(None, Some(GCLID)))));
    assert!(!has_google_click(Some(&ids(None, None))));
    assert!(!has_google_click::<GoogleClickIds>(None));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:301 "isLeadLikeCategory > treats lead categories as lead-like and purchases as not"
#[test]
fn isleadlikecategory_treats_lead_categories_as_lead_like_and_purchases_as_not() {
    assert!(is_lead_like_category("QUALIFIED_LEAD"));
    assert!(!is_lead_like_category("PURCHASE"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:308 "customer id helpers > formats a 10-digit id"
#[test]
fn customer_id_helpers_formats_a_10_digit_id() {
    assert_eq!(format_customer_id("1234567890"), "123-456-7890");
    assert_eq!(format_customer_id("abc"), "abc");
}

/// Port of: packages/utils/__tests__/google-click.test.ts:313 "customer id helpers > normalises dashes and spaces and rejects bad ids"
#[test]
fn customer_id_helpers_normalises_dashes_and_spaces_and_rejects_bad_ids() {
    assert_eq!(
        parse_customer_id("123-456-7890").as_deref(),
        Some("1234567890")
    );
    assert_eq!(parse_customer_id("12345"), None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:322 "googleAdsChannels > is derived from the single channel list"
#[test]
fn googleadschannels_is_derived_from_the_single_channel_list() {
    let options: Vec<&str> = GoogleAdsChannel::ALL.iter().map(|c| c.as_str()).collect();
    assert_eq!(options, GOOGLE_ADS_CHANNEL_VALUES.to_vec());
    assert_eq!(GoogleAdsChannel::parse("email"), None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:329 "Google click capture source > resolves whatsapp to payload"
#[test]
fn google_click_capture_source_resolves_whatsapp_to_payload() {
    assert_eq!(
        resolve_google_click_capture_source("whatsapp"),
        Some(GoogleClickCaptureSource::Payload)
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:329 "Google click capture source > resolves messenger to payload"
#[test]
fn google_click_capture_source_resolves_messenger_to_payload() {
    assert_eq!(
        resolve_google_click_capture_source("messenger"),
        Some(GoogleClickCaptureSource::Payload)
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:329 "Google click capture source > resolves zalo to profileLookup"
#[test]
fn google_click_capture_source_resolves_zalo_to_profilelookup() {
    assert_eq!(
        resolve_google_click_capture_source("zalo"),
        Some(GoogleClickCaptureSource::ProfileLookup)
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:337 "Google click capture source > resolves a channel that cannot carry a click to null"
#[test]
fn google_click_capture_source_resolves_a_channel_that_cannot_carry_a_click_to_null() {
    assert_eq!(resolve_google_click_capture_source("telegram"), None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:348 "withoutRepeatedGoogleClick > returns null when the same gclid is already stored"
#[test]
fn withoutrepeatedgoogleclick_returns_null_when_the_same_gclid_is_already_stored() {
    let next = to_google_click_referral(
        &GoogleClickPayload {
            campaign_id: s("1"),
            ..payload(GoogleClickIdType::Gclid, "ABCDEFGHIJ1234567890")
        },
        Some(received_at()),
        0,
    );
    assert_eq!(
        without_repeated_google_click(Some(&ids(Some("ABCDEFGHIJ1234567890"), None)), Some(&next)),
        None
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:354 "withoutRepeatedGoogleClick > returns null when the same gbraid is already stored"
#[test]
fn withoutrepeatedgoogleclick_returns_null_when_the_same_gbraid_is_already_stored() {
    let gbraid = to_google_click_referral(
        &payload(GoogleClickIdType::Gbraid, "ZYXWVUTSRQ0987654321"),
        Some(received_at()),
        0,
    );
    assert_eq!(
        without_repeated_google_click(
            Some(&ids(None, Some("ZYXWVUTSRQ0987654321"))),
            Some(&gbraid)
        ),
        None
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:364 "withoutRepeatedGoogleClick > keeps a different click id"
#[test]
fn withoutrepeatedgoogleclick_keeps_a_different_click_id() {
    let next = to_google_click_referral(
        &GoogleClickPayload {
            campaign_id: s("1"),
            ..payload(GoogleClickIdType::Gclid, "ABCDEFGHIJ1234567890")
        },
        Some(received_at()),
        0,
    );
    assert!(std::ptr::eq(
        without_repeated_google_click(Some(&ids(Some("OTHEROTHER1234567890"), None)), Some(&next))
            .unwrap(),
        &next
    ));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:370 "withoutRepeatedGoogleClick > keeps the same id carried by the other click type"
#[test]
fn withoutrepeatedgoogleclick_keeps_the_same_id_carried_by_the_other_click_type() {
    let next = to_google_click_referral(
        &GoogleClickPayload {
            campaign_id: s("1"),
            ..payload(GoogleClickIdType::Gclid, "ABCDEFGHIJ1234567890")
        },
        Some(received_at()),
        0,
    );
    assert!(std::ptr::eq(
        without_repeated_google_click(Some(&ids(None, Some("ABCDEFGHIJ1234567890"))), Some(&next))
            .unwrap(),
        &next
    ));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:376 "withoutRepeatedGoogleClick > returns null when the next referral is null"
#[test]
fn withoutrepeatedgoogleclick_returns_null_when_the_next_referral_is_null() {
    assert_eq!(
        without_repeated_google_click::<GoogleClickIds, GoogleClickReferral>(None, None),
        None
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:376 "withoutRepeatedGoogleClick > returns null when the next referral is undefined"
#[test]
fn withoutrepeatedgoogleclick_returns_null_when_the_next_referral_is_undefined() {
    assert_eq!(
        without_repeated_google_click::<GoogleClickIds, GoogleClickReferral>(None, None),
        None
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:376 "withoutRepeatedGoogleClick > returns null when the next referral is a referral without a click"
#[test]
fn withoutrepeatedgoogleclick_returns_null_when_the_next_referral_is_a_referral_without_a_click() {
    let value = to_google_click_referral(
        &payload(GoogleClickIdType::Gclid, "ABCDEFGHIJ1234567890"),
        Some(received_at()),
        0,
    );
    let without_click = GoogleClickReferral {
        gclid: None,
        gbraid: None,
        ..value
    };
    assert_eq!(
        without_repeated_google_click::<GoogleClickIds, _>(None, Some(&without_click)),
        None
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:391 "withoutRepeatedGoogleClick > keeps the click when there is no stored referral"
#[test]
fn withoutrepeatedgoogleclick_keeps_the_click_when_there_is_no_stored_referral() {
    let next = to_google_click_referral(
        &GoogleClickPayload {
            campaign_id: s("1"),
            ..payload(GoogleClickIdType::Gclid, "ABCDEFGHIJ1234567890")
        },
        Some(received_at()),
        0,
    );
    assert!(std::ptr::eq(
        without_repeated_google_click::<GoogleClickIds, _>(None, Some(&next)).unwrap(),
        &next
    ));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:391 "withoutRepeatedGoogleClick > keeps the click when there is a stored referral without a click"
#[test]
fn withoutrepeatedgoogleclick_keeps_the_click_when_there_is_a_stored_referral_without_a_click() {
    let next = to_google_click_referral(
        &GoogleClickPayload {
            campaign_id: s("1"),
            ..payload(GoogleClickIdType::Gclid, "ABCDEFGHIJ1234567890")
        },
        Some(received_at()),
        0,
    );
    assert!(std::ptr::eq(
        without_repeated_google_click(Some(&ids(None, None)), Some(&next)).unwrap(),
        &next
    ));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:400 "consumeGoogleClickRef > consumes a Google ref: ref becomes null and the referral is built"
#[test]
fn consumegoogleclickref_consumes_a_google_ref_ref_becomes_null_and_the_referral_is_built() {
    let result = consume_google_click_ref(
        Some(&format!("gclid:{GCLID},adid:3")),
        Some(received_at()),
        0,
    );
    assert_eq!(result.reference, None);
    assert_eq!(
        result.google_referral,
        Some(to_google_click_referral(
            &parse_google_click_ref(&format!("gclid:{GCLID},adid:3")).unwrap(),
            Some(received_at()),
            0
        ))
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:411 "consumeGoogleClickRef > keeps an ordinary reflink for the ref router"
#[test]
fn consumegoogleclickref_keeps_an_ordinary_reflink_for_the_ref_router() {
    assert_eq!(
        consume_google_click_ref(Some("f_abc123"), Some(received_at()), 0),
        GoogleClickCapture {
            reference: s("f_abc123"),
            google_referral: None
        }
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:411 "consumeGoogleClickRef > keeps an empty ref for the ref router"
#[test]
fn consumegoogleclickref_keeps_an_empty_ref_for_the_ref_router() {
    assert_eq!(
        consume_google_click_ref(Some(""), Some(received_at()), 0),
        GoogleClickCapture {
            reference: s(""),
            google_referral: None
        }
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:421 "consumeGoogleClickRef > maps null to a null ref"
#[test]
fn consumegoogleclickref_maps_null_to_a_null_ref() {
    assert_eq!(
        consume_google_click_ref(None, Some(received_at()), 0),
        GoogleClickCapture {
            reference: None,
            google_referral: None
        }
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:421 "consumeGoogleClickRef > maps undefined to a null ref"
#[test]
fn consumegoogleclickref_maps_undefined_to_a_null_ref() {
    assert_eq!(
        consume_google_click_ref(None, Some(received_at()), 0),
        GoogleClickCapture {
            reference: None,
            google_referral: None
        }
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:430 "extractInvisibleGoogleClick > strips only the consumed run and builds the referral"
#[test]
fn extractinvisiblegoogleclick_strips_only_the_consumed_run_and_builds_the_referral() {
    let text = format!("{}Hello", hide(json!({ "gclid": GCLID })));
    let result = extract_invisible_google_click(&text, Some(received_at()), 0);
    assert_eq!(result.text, "Hello");
    let referral = result.google_referral.unwrap();
    assert_eq!(referral.gclid, s(GCLID));
    assert_eq!(
        referral.google_click_received_at,
        support::iso(received_at())
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:440 "extractInvisibleGoogleClick > returns plain text untouched (including unrelated variation selectors)"
#[test]
fn extractinvisiblegoogleclick_returns_plain_text_untouched_including_unrelated_variation_selectors()
 {
    let text = "\u{845B}\u{E0100}\u{57CE} hi";
    let result = extract_invisible_google_click(text, Some(received_at()), 0);
    assert_eq!(result.text, text);
    assert_eq!(result.google_referral, None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:450 "isRfc3339WithZone > accepts 2026-10-08T14:30:00Z"
#[test]
fn isrfc3339withzone_accepts_2026_10_08t14_30_00z() {
    assert!(is_rfc3339_with_zone("2026-10-08T14:30:00Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:450 "isRfc3339WithZone > accepts 2026-10-08T14:30:00+07:00"
#[test]
fn isrfc3339withzone_accepts_2026_10_08t14_30_00_07_00() {
    assert!(is_rfc3339_with_zone("2026-10-08T14:30:00+07:00"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:450 "isRfc3339WithZone > accepts 2026-10-08T14:30:00-05:30"
#[test]
fn isrfc3339withzone_accepts_2026_10_08t14_30_00_05_30() {
    assert!(is_rfc3339_with_zone("2026-10-08T14:30:00-05:30"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:450 "isRfc3339WithZone > accepts 2026-10-08T14:30:00.123Z"
#[test]
fn isrfc3339withzone_accepts_2026_10_08t14_30_00_123z() {
    assert!(is_rfc3339_with_zone("2026-10-08T14:30:00.123Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:450 "isRfc3339WithZone > accepts 2024-02-29T00:00:00Z"
#[test]
fn isrfc3339withzone_accepts_2024_02_29t00_00_00z() {
    assert!(is_rfc3339_with_zone("2024-02-29T00:00:00Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:450 "isRfc3339WithZone > accepts 2026-10-08t14:30:00z"
#[test]
fn isrfc3339withzone_accepts_2026_10_08t14_30_00z_2() {
    assert!(is_rfc3339_with_zone("2026-10-08t14:30:00z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-10-08T14:30:00""
#[test]
fn isrfc3339withzone_rejects_2026_10_08t14_30_00() {
    assert!(!is_rfc3339_with_zone("2026-10-08T14:30:00"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-10-08 14:30:00Z""
#[test]
fn isrfc3339withzone_rejects_2026_10_08_14_30_00z() {
    assert!(!is_rfc3339_with_zone("2026-10-08 14:30:00Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-10-08""
#[test]
fn isrfc3339withzone_rejects_2026_10_08() {
    assert!(!is_rfc3339_with_zone("2026-10-08"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-10-08T14:30Z""
#[test]
fn isrfc3339withzone_rejects_2026_10_08t14_30z() {
    assert!(!is_rfc3339_with_zone("2026-10-08T14:30Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-02-30T10:00:00Z""
#[test]
fn isrfc3339withzone_rejects_2026_02_30t10_00_00z() {
    assert!(!is_rfc3339_with_zone("2026-02-30T10:00:00Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2025-02-29T10:00:00Z""
#[test]
fn isrfc3339withzone_rejects_2025_02_29t10_00_00z() {
    assert!(!is_rfc3339_with_zone("2025-02-29T10:00:00Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-13-01T10:00:00Z""
#[test]
fn isrfc3339withzone_rejects_2026_13_01t10_00_00z() {
    assert!(!is_rfc3339_with_zone("2026-13-01T10:00:00Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-10-08T24:00:00Z""
#[test]
fn isrfc3339withzone_rejects_2026_10_08t24_00_00z() {
    assert!(!is_rfc3339_with_zone("2026-10-08T24:00:00Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-10-08T14:60:00Z""
#[test]
fn isrfc3339withzone_rejects_2026_10_08t14_60_00z() {
    assert!(!is_rfc3339_with_zone("2026-10-08T14:60:00Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-10-08T14:30:60Z""
#[test]
fn isrfc3339withzone_rejects_2026_10_08t14_30_60z() {
    assert!(!is_rfc3339_with_zone("2026-10-08T14:30:60Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-10-08T14:30:00+24:00""
#[test]
fn isrfc3339withzone_rejects_2026_10_08t14_30_00_24_00() {
    assert!(!is_rfc3339_with_zone("2026-10-08T14:30:00+24:00"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-10-08T14:30:00+07:60""
#[test]
fn isrfc3339withzone_rejects_2026_10_08t14_30_00_07_60() {
    assert!(!is_rfc3339_with_zone("2026-10-08T14:30:00+07:60"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-10-08T14:30:00+0700""
#[test]
fn isrfc3339withzone_rejects_2026_10_08t14_30_00_0700() {
    assert!(!is_rfc3339_with_zone("2026-10-08T14:30:00+0700"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "2026-10-08T14:30:00.Z""
#[test]
fn isrfc3339withzone_rejects_2026_10_08t14_30_00_z() {
    assert!(!is_rfc3339_with_zone("2026-10-08T14:30:00.Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects "not a date""
#[test]
fn isrfc3339withzone_rejects_not_a_date() {
    assert!(!is_rfc3339_with_zone("not a date"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:461 "isRfc3339WithZone > rejects """
#[test]
fn isrfc3339withzone_rejects() {
    assert!(!is_rfc3339_with_zone(""));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:484 "parseConversionTime > returns the UTC instant for a Z value"
#[test]
fn parseconversiontime_returns_the_utc_instant_for_a_z_value() {
    assert_eq!(
        parse_conversion_time("2026-10-08T14:30:00Z").map(support::iso),
        s("2026-10-08T14:30:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:490 "parseConversionTime > applies a positive and a negative offset"
#[test]
fn parseconversiontime_applies_a_positive_and_a_negative_offset() {
    assert_eq!(
        parse_conversion_time("2026-10-08T14:30:00+07:00").map(support::iso),
        s("2026-10-08T07:30:00.000Z")
    );
    assert_eq!(
        parse_conversion_time("2026-10-08T14:30:00-05:30").map(support::iso),
        s("2026-10-08T20:00:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:499 "parseConversionTime > drops fractional seconds"
#[test]
fn parseconversiontime_drops_fractional_seconds() {
    assert_eq!(
        parse_conversion_time("2026-10-08T14:30:00.999Z"),
        Some(chatbotx_core::js::date_utc(2026, 9, 8, 14, 30, 0, 0))
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:505 "parseConversionTime > rolls the date across an offset boundary"
#[test]
fn parseconversiontime_rolls_the_date_across_an_offset_boundary() {
    assert_eq!(
        parse_conversion_time("2026-01-01T00:30:00+01:00").map(support::iso),
        s("2025-12-31T23:30:00.000Z")
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:511 "parseConversionTime > returns null for a missing zone or an impossible date"
#[test]
fn parseconversiontime_returns_null_for_a_missing_zone_or_an_impossible_date() {
    assert_eq!(parse_conversion_time("2026-10-08T14:30:00"), None);
    assert_eq!(parse_conversion_time("2026-02-30T10:00:00Z"), None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:518 "hasRfc3339Shape > accepts the shape even for a date that does not exist"
#[test]
fn hasrfc3339shape_accepts_the_shape_even_for_a_date_that_does_not_exist() {
    assert!(has_rfc3339_shape("2026-02-30T10:00:00Z"));
    assert!(!is_rfc3339_with_zone("2026-02-30T10:00:00Z"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:523 "hasRfc3339Shape > rejects "2026-10-08T14:30:00""
#[test]
fn hasrfc3339shape_rejects_2026_10_08t14_30_00() {
    assert!(!has_rfc3339_shape("2026-10-08T14:30:00"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:523 "hasRfc3339Shape > rejects "2026-10-08""
#[test]
fn hasrfc3339shape_rejects_2026_10_08() {
    assert!(!has_rfc3339_shape("2026-10-08"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:523 "hasRfc3339Shape > rejects "not a date""
#[test]
fn hasrfc3339shape_rejects_not_a_date() {
    assert!(!has_rfc3339_shape("not a date"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:533 "googleAdsConversionErrorCodes > has a unique google_ads_ code per reason, including the option codes"
#[test]
fn googleadsconversionerrorcodes_has_a_unique_google_ads_code_per_reason_including_the_option_codes()
 {
    let codes: Vec<&str> = GOOGLE_ADS_CONVERSION_ERROR_CODES
        .iter()
        .map(|(_, code)| *code)
        .collect();
    assert_eq!(
        codes.iter().collect::<std::collections::HashSet<_>>().len(),
        codes.len()
    );
    assert!(codes.iter().all(|code| code.starts_with("google_ads_")));
    for (name, code) in [
        ("invalidConsentConfig", "google_ads_invalid_consent_config"),
        ("invalidConsentValue", "google_ads_invalid_consent_value"),
        (
            "invalidConversionTime",
            "google_ads_invalid_conversion_time",
        ),
        ("missingDedupId", "google_ads_missing_dedup_id"),
        ("invalidDedupId", "google_ads_invalid_dedup_id"),
    ] {
        assert_eq!(google_ads_conversion_error_code(name), Some(code));
    }
}

/// Port of: packages/utils/__tests__/google-click.test.ts:549 "isGoogleAdsMatchTemplate > accepts {{email}}"
#[test]
fn isgoogleadsmatchtemplate_accepts_email() {
    assert!(is_google_ads_match_template("{{email}}"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:549 "isGoogleAdsMatchTemplate > accepts {{ phone }}"
#[test]
fn isgoogleadsmatchtemplate_accepts_phone() {
    assert!(is_google_ads_match_template("{{ phone }}"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:549 "isGoogleAdsMatchTemplate > accepts {{custom_work_email}}"
#[test]
fn isgoogleadsmatchtemplate_accepts_custom_work_email() {
    assert!(is_google_ads_match_template("{{custom_work_email}}"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:549 "isGoogleAdsMatchTemplate > accepts {{aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}}"
#[test]
fn isgoogleadsmatchtemplate_accepts_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa()
 {
    assert!(is_google_ads_match_template(
        "{{aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}}"
    ));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:558 "isGoogleAdsMatchTemplate > refuses """
#[test]
fn isgoogleadsmatchtemplate_refuses() {
    assert!(!is_google_ads_match_template(""));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:558 "isGoogleAdsMatchTemplate > refuses "off""
#[test]
fn isgoogleadsmatchtemplate_refuses_off() {
    assert!(!is_google_ads_match_template("off"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:558 "isGoogleAdsMatchTemplate > refuses "contact""
#[test]
fn isgoogleadsmatchtemplate_refuses_contact() {
    assert!(!is_google_ads_match_template("contact"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:558 "isGoogleAdsMatchTemplate > refuses "11690032856629248""
#[test]
fn isgoogleadsmatchtemplate_refuses_11690032856629248() {
    assert!(!is_google_ads_match_template("11690032856629248"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:558 "isGoogleAdsMatchTemplate > refuses "jane@example.com""
#[test]
fn isgoogleadsmatchtemplate_refuses_jane_example_com() {
    assert!(!is_google_ads_match_template("jane@example.com"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:558 "isGoogleAdsMatchTemplate > refuses "+84901234567""
#[test]
fn isgoogleadsmatchtemplate_refuses_84901234567() {
    assert!(!is_google_ads_match_template("+84901234567"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:558 "isGoogleAdsMatchTemplate > refuses "{{}}""
#[test]
fn isgoogleadsmatchtemplate_refuses_2() {
    assert!(!is_google_ads_match_template("{{}}"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:558 "isGoogleAdsMatchTemplate > refuses "mail {{email}}""
#[test]
fn isgoogleadsmatchtemplate_refuses_mail_email() {
    assert!(!is_google_ads_match_template("mail {{email}}"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:558 "isGoogleAdsMatchTemplate > refuses "{{email}} ""
#[test]
fn isgoogleadsmatchtemplate_refuses_email() {
    assert!(!is_google_ads_match_template("{{email}} "));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:558 "isGoogleAdsMatchTemplate > refuses "{{email}}{{phone}}""
#[test]
fn isgoogleadsmatchtemplate_refuses_email_phone() {
    assert!(!is_google_ads_match_template("{{email}}{{phone}}"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:558 "isGoogleAdsMatchTemplate > refuses "{{a{{b}}}}""
#[test]
fn isgoogleadsmatchtemplate_refuses_a_b() {
    assert!(!is_google_ads_match_template("{{a{{b}}}}"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:558 "isGoogleAdsMatchTemplate > refuses "{{aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}}""
#[test]
fn isgoogleadsmatchtemplate_refuses_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa()
 {
    assert!(!is_google_ads_match_template(
        "{{aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}}"
    ));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:581 "real-world starter messages and refs > captures the click at the start and keeps only the visible text"
#[test]
fn real_world_starter_messages_and_refs_captures_the_click_at_the_start_and_keeps_only_the_visible_text()
 {
    const CLICK: &str = "Cj0KCQiAabcdefghij_-KLMN";
    let text = format!("{}Hello", hide(json!({ "gclid": CLICK })));
    let result =
        extract_invisible_google_click(&text, Some(support::iso_ms("2026-10-08T00:00:00Z")), 0);
    assert_eq!(
        result.google_referral.unwrap().gclid.as_deref(),
        Some(CLICK)
    );
    assert_eq!(result.text, "Hello");
}

/// Port of: packages/utils/__tests__/google-click.test.ts:581 "real-world starter messages and refs > captures the click at the end and keeps only the visible text"
#[test]
fn real_world_starter_messages_and_refs_captures_the_click_at_the_end_and_keeps_only_the_visible_text()
 {
    const CLICK: &str = "Cj0KCQiAabcdefghij_-KLMN";
    let text = format!("Hello{}", hide(json!({ "gclid": CLICK })));
    let result =
        extract_invisible_google_click(&text, Some(support::iso_ms("2026-10-08T00:00:00Z")), 0);
    assert_eq!(
        result.google_referral.unwrap().gclid.as_deref(),
        Some(CLICK)
    );
    assert_eq!(result.text, "Hello");
}

/// Port of: packages/utils/__tests__/google-click.test.ts:581 "real-world starter messages and refs > captures the click inside the text and keeps only the visible text"
#[test]
fn real_world_starter_messages_and_refs_captures_the_click_inside_the_text_and_keeps_only_the_visible_text()
 {
    const CLICK: &str = "Cj0KCQiAabcdefghij_-KLMN";
    let text = format!("Hel{}lo", hide(json!({ "gclid": CLICK })));
    let result =
        extract_invisible_google_click(&text, Some(support::iso_ms("2026-10-08T00:00:00Z")), 0);
    assert_eq!(
        result.google_referral.unwrap().gclid.as_deref(),
        Some(CLICK)
    );
    assert_eq!(result.text, "Hello");
}

/// Port of: packages/utils/__tests__/google-click.test.ts:581 "real-world starter messages and refs > captures the click between emoji and keeps only the visible text"
#[test]
fn real_world_starter_messages_and_refs_captures_the_click_between_emoji_and_keeps_only_the_visible_text()
 {
    const CLICK: &str = "Cj0KCQiAabcdefghij_-KLMN";
    let text = format!("😀{}👍", hide(json!({ "gclid": CLICK })));
    let result =
        extract_invisible_google_click(&text, Some(support::iso_ms("2026-10-08T00:00:00Z")), 0);
    assert_eq!(
        result.google_referral.unwrap().gclid.as_deref(),
        Some(CLICK)
    );
    assert_eq!(result.text, "😀👍");
}

/// Port of: packages/utils/__tests__/google-click.test.ts:581 "real-world starter messages and refs > captures the click with the visible text edited and keeps only the visible text"
#[test]
fn real_world_starter_messages_and_refs_captures_the_click_with_the_visible_text_edited_and_keeps_only_the_visible_text()
 {
    const CLICK: &str = "Cj0KCQiAabcdefghij_-KLMN";
    let text = format!("{}other words", hide(json!({ "gclid": CLICK })));
    let result =
        extract_invisible_google_click(&text, Some(support::iso_ms("2026-10-08T00:00:00Z")), 0);
    assert_eq!(
        result.google_referral.unwrap().gclid.as_deref(),
        Some(CLICK)
    );
    assert_eq!(result.text, "other words");
}

/// Port of: packages/utils/__tests__/google-click.test.ts:581 "real-world starter messages and refs > captures the click with no visible text left and keeps only the visible text"
#[test]
fn real_world_starter_messages_and_refs_captures_the_click_with_no_visible_text_left_and_keeps_only_the_visible_text()
 {
    const CLICK: &str = "Cj0KCQiAabcdefghij_-KLMN";
    let text = hide(json!({ "gclid": CLICK }));
    let result =
        extract_invisible_google_click(&text, Some(support::iso_ms("2026-10-08T00:00:00Z")), 0);
    assert_eq!(
        result.google_referral.unwrap().gclid.as_deref(),
        Some(CLICK)
    );
    assert_eq!(result.text, "");
}

/// Port of: packages/utils/__tests__/google-click.test.ts:581 "real-world starter messages and refs > captures the click with unknown extra keys and keeps only the visible text"
#[test]
fn real_world_starter_messages_and_refs_captures_the_click_with_unknown_extra_keys_and_keeps_only_the_visible_text()
 {
    const CLICK: &str = "Cj0KCQiAabcdefghij_-KLMN";
    let text = format!("{}Hi", hide(json!({ "gclid": CLICK, "gad_source": 1 })));
    let result =
        extract_invisible_google_click(&text, Some(support::iso_ms("2026-10-08T00:00:00Z")), 0);
    assert_eq!(
        result.google_referral.unwrap().gclid.as_deref(),
        Some(CLICK)
    );
    assert_eq!(result.text, "Hi");
}

/// Port of: packages/utils/__tests__/google-click.test.ts:603 "real-world starter messages and refs > keeps gclid when a payload carries both ids, and reads ids sent as strings"
#[test]
fn real_world_starter_messages_and_refs_keeps_gclid_when_a_payload_carries_both_ids_and_reads_ids_sent_as_strings()
 {
    const CLICK: &str = "Cj0KCQiAabcdefghij_-KLMN";
    let text = format!(
        "{}Hi",
        hide(json!({ "gclid": CLICK, "gbraid": "BBBBBBBBBBBB", "campaignid": "12", "adid": "9" }))
    );
    let referral =
        extract_invisible_google_click(&text, Some(support::iso_ms("2026-10-08T00:00:00Z")), 0)
            .google_referral
            .unwrap();
    assert_eq!(referral.gclid.as_deref(), Some(CLICK));
    assert_eq!(referral.gbraid, None);
    assert_eq!(referral.google_campaign_id, s("12"));
    assert_eq!(referral.google_ad_id, s("9"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:616 "real-world starter messages and refs > leaves the text untouched for truncated JSON"
#[test]
fn real_world_starter_messages_and_refs_leaves_the_text_untouched_for_truncated_json() {
    const CLICK: &str = "Cj0KCQiAabcdefghij_-KLMN";
    let _ = CLICK;
    let text = format!(
        "{}Hi",
        encode(&{
            let json = chatbotx_core::js::json_stringify(&json!({ "gclid": CLICK }));
            json[..json.len() - 3].to_owned()
        })
    );
    let result =
        extract_invisible_google_click(&text, Some(support::iso_ms("2026-10-08T00:00:00Z")), 0);
    assert_eq!(result.google_referral, None);
    assert_eq!(result.text, text);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:616 "real-world starter messages and refs > leaves the text untouched for a too-short id"
#[test]
fn real_world_starter_messages_and_refs_leaves_the_text_untouched_for_a_too_short_id() {
    const CLICK: &str = "Cj0KCQiAabcdefghij_-KLMN";
    let _ = CLICK;
    let text = format!("{}Hi", hide(json!({ "gclid": "abc" })));
    let result =
        extract_invisible_google_click(&text, Some(support::iso_ms("2026-10-08T00:00:00Z")), 0);
    assert_eq!(result.google_referral, None);
    assert_eq!(result.text, text);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:616 "real-world starter messages and refs > leaves the text untouched for a numeric id"
#[test]
fn real_world_starter_messages_and_refs_leaves_the_text_untouched_for_a_numeric_id() {
    const CLICK: &str = "Cj0KCQiAabcdefghij_-KLMN";
    let _ = CLICK;
    let text = format!("{}Hi", hide(json!({ "gclid": 12_345_678_901_234_i64 })));
    let result =
        extract_invisible_google_click(&text, Some(support::iso_ms("2026-10-08T00:00:00Z")), 0);
    assert_eq!(result.google_referral, None);
    assert_eq!(result.text, text);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:616 "real-world starter messages and refs > leaves the text untouched for a JSON array"
#[test]
fn real_world_starter_messages_and_refs_leaves_the_text_untouched_for_a_json_array() {
    const CLICK: &str = "Cj0KCQiAabcdefghij_-KLMN";
    let _ = CLICK;
    let text = format!("{}Hi", encode("[1,2]"));
    let result =
        extract_invisible_google_click(&text, Some(support::iso_ms("2026-10-08T00:00:00Z")), 0);
    assert_eq!(result.google_referral, None);
    assert_eq!(result.text, text);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:628 "real-world starter messages and refs > reads a Messenger ref with metadata before the id"
#[test]
fn real_world_starter_messages_and_refs_reads_a_messenger_ref_with_metadata_before_the_id() {
    let result = consume_google_click_ref(
        Some("k1:v1,gclid:Cj0KCQiAabcdefghij_-KLMN"),
        Some(support::iso_ms("2026-10-08T00:00:00Z")),
        0,
    );
    assert_eq!(
        result.google_referral.unwrap().gclid,
        s("Cj0KCQiAabcdefghij_-KLMN")
    );
    assert_eq!(result.reference, None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:628 "real-world starter messages and refs > reads a Messenger ref with upper-case keys"
#[test]
fn real_world_starter_messages_and_refs_reads_a_messenger_ref_with_upper_case_keys() {
    let result = consume_google_click_ref(
        Some("GCLID:Cj0KCQiAabcdefghij_-KLMN,CampaignId:1"),
        Some(support::iso_ms("2026-10-08T00:00:00Z")),
        0,
    );
    assert_eq!(
        result.google_referral.unwrap().gclid,
        s("Cj0KCQiAabcdefghij_-KLMN")
    );
    assert_eq!(result.reference, None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:628 "real-world starter messages and refs > reads a Messenger ref with spaces around pairs"
#[test]
fn real_world_starter_messages_and_refs_reads_a_messenger_ref_with_spaces_around_pairs() {
    let result = consume_google_click_ref(
        Some("gclid: Cj0KCQiAabcdefghij_-KLMN , campaignid: 7"),
        Some(support::iso_ms("2026-10-08T00:00:00Z")),
        0,
    );
    assert_eq!(
        result.google_referral.unwrap().gclid,
        s("Cj0KCQiAabcdefghij_-KLMN")
    );
    assert_eq!(result.reference, None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:628 "real-world starter messages and refs > reads a Messenger ref with URL-encoded separators"
#[test]
fn real_world_starter_messages_and_refs_reads_a_messenger_ref_with_url_encoded_separators() {
    let result = consume_google_click_ref(
        Some("gclid:Cj0KCQiAabcdefghij_-KLMN%2Ccampaignid%3A1"),
        Some(support::iso_ms("2026-10-08T00:00:00Z")),
        0,
    );
    assert_eq!(
        result.google_referral.unwrap().gclid,
        s("Cj0KCQiAabcdefghij_-KLMN")
    );
    assert_eq!(result.reference, None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:628 "real-world starter messages and refs > reads a Messenger ref with a bare id with no metadata"
#[test]
fn real_world_starter_messages_and_refs_reads_a_messenger_ref_with_a_bare_id_with_no_metadata() {
    let result = consume_google_click_ref(
        Some("gclid:Cj0KCQiAabcdefghij_-KLMN"),
        Some(support::iso_ms("2026-10-08T00:00:00Z")),
        0,
    );
    assert_eq!(
        result.google_referral.unwrap().gclid,
        s("Cj0KCQiAabcdefghij_-KLMN")
    );
    assert_eq!(result.reference, None);
}

/// Port of: packages/utils/__tests__/google-click.test.ts:640 "real-world starter messages and refs > keeps a semicolon-separated ref as a normal ref"
#[test]
fn real_world_starter_messages_and_refs_keeps_a_semicolon_separated_ref_as_a_normal_ref() {
    let result = consume_google_click_ref(
        Some("gclid:Cj0KCQiAabcdefghij_-KLMN;campaignid:1"),
        Some(support::iso_ms("2026-10-08T00:00:00Z")),
        0,
    );
    assert_eq!(result.google_referral, None);
    assert_eq!(
        result.reference,
        s("gclid:Cj0KCQiAabcdefghij_-KLMN;campaignid:1")
    );
}

/// Port of: packages/utils/__tests__/google-click.test.ts:640 "real-world starter messages and refs > keeps an empty id as a normal ref"
#[test]
fn real_world_starter_messages_and_refs_keeps_an_empty_id_as_a_normal_ref() {
    let result = consume_google_click_ref(
        Some("gclid:"),
        Some(support::iso_ms("2026-10-08T00:00:00Z")),
        0,
    );
    assert_eq!(result.google_referral, None);
    assert_eq!(result.reference, s("gclid:"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:640 "real-world starter messages and refs > keeps a raw legacy id as a normal ref"
#[test]
fn real_world_starter_messages_and_refs_keeps_a_raw_legacy_id_as_a_normal_ref() {
    let result = consume_google_click_ref(
        Some("Cj0KCQiAabcdefghij_-KLMN"),
        Some(support::iso_ms("2026-10-08T00:00:00Z")),
        0,
    );
    assert_eq!(result.google_referral, None);
    assert_eq!(result.reference, s("Cj0KCQiAabcdefghij_-KLMN"));
}

/// Port of: packages/utils/__tests__/google-click.test.ts:640 "real-world starter messages and refs > keeps a reflink as a normal ref"
#[test]
fn real_world_starter_messages_and_refs_keeps_a_reflink_as_a_normal_ref() {
    let result = consume_google_click_ref(
        Some("f_abc123"),
        Some(support::iso_ms("2026-10-08T00:00:00Z")),
        0,
    );
    assert_eq!(result.google_referral, None);
    assert_eq!(result.reference, s("f_abc123"));
}
