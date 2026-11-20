//! `integrations/whatsapp/__tests__/google-click-capture.test.ts` and
//! `incoming-message-referral.test.ts`: the WhatsApp parser's Google click
//! (invisible characters in the starter text) and Click-to-WhatsApp referral.

use chatbotx_core::utils::google_click::{
    GOOGLE_INVISIBLE_BASE_CODE_POINT, GOOGLE_INVISIBLE_CHAR_SET,
};
use chatbotx_worker::channel::Received;
use chatbotx_worker::channels::whatsapp::parse_incoming;
use serde_json::{Value, json};

const GCLID: &str = "ABCDEFGHIJ1234567890";
const GBRAID: &str = "ZYXWVUTSRQ0987654321";

fn encode_invisible(text: &str) -> String {
    text.chars()
        .map(|c| {
            let index = GOOGLE_INVISIBLE_CHAR_SET
                .find(c)
                .unwrap_or_else(|| panic!("char not encodable: {c}"));
            char::from_u32(GOOGLE_INVISIBLE_BASE_CODE_POINT + index as u32).unwrap()
        })
        .collect()
}

fn gclid_run() -> String {
    encode_invisible(
        r#"{"gclid":"ABCDEFGHIJ1234567890","campaignid":123,"adgroupid":456,"adid":789}"#,
    )
}

fn parse(body: &str, referral: Option<Value>, timestamp: Option<&str>) -> Received {
    let mut message = json!({ "id": "wamid.test-1", "type": "text", "text": { "body": body } });
    if let Some(r) = referral {
        message["referral"] = r;
    }
    if let Some(t) = timestamp {
        message["timestamp"] = json!(t);
    }
    parse_incoming(
        &json!({ "phoneID": "phone-1", "from": "84900000001", "name": "Alice", "message": message }),
    )
}

fn text(r: &Received) -> Option<&str> {
    r.message.as_ref()?.text.as_deref()
}

fn referral(r: &Received) -> &Value {
    r.referral.as_ref().expect("a referral")
}

#[test]
fn decodes_a_gclid_run_at_the_start_and_cleans_the_text() {
    let r = parse(&format!("{}Hello there", gclid_run()), None, None);
    assert_eq!(text(&r), Some("Hello there"));
    let got = referral(&r);
    assert_eq!(got["gclid"], GCLID);
    assert_eq!(got["gbraid"], Value::Null);
    assert_eq!(got["googleCampaignId"], "123");
    assert_eq!(got["googleAdGroupId"], "456");
    assert_eq!(got["googleAdId"], "789");
    // An ISO string with ms: the clock fallback, close to now.
    let stamp = got["googleClickReceivedAt"].as_str().unwrap();
    let at = chrono::DateTime::parse_from_rfc3339(stamp).unwrap();
    assert!(stamp.ends_with('Z') && stamp.len() == 24, "{stamp}");
    assert!((chrono::Utc::now().timestamp_millis() - at.timestamp_millis()).abs() < 60_000);
}

#[test]
fn google_click_received_at_is_the_message_timestamp_else_the_clock() {
    let r = parse(&format!("{}Hello", gclid_run()), None, Some("1781000000"));
    assert_eq!(
        referral(&r)["googleClickReceivedAt"],
        "2026-06-09T10:13:20.000Z"
    );
    let r = parse(&format!("{}Hello", gclid_run()), None, Some("not-a-number"));
    let stamp = referral(&r)["googleClickReceivedAt"].as_str().unwrap();
    let at = chrono::DateTime::parse_from_rfc3339(stamp).unwrap();
    assert!((chrono::Utc::now().timestamp_millis() - at.timestamp_millis()).abs() < 60_000);
}

#[test]
fn decodes_a_run_in_the_middle_and_a_gbraid_click() {
    let r = parse(&format!("Hello {}there", gclid_run()), None, None);
    assert_eq!(text(&r), Some("Hello there"));
    assert_eq!(referral(&r)["gclid"], GCLID);

    let run = encode_invisible(r#"{"gbraid":"ZYXWVUTSRQ0987654321"}"#);
    let r = parse(&format!("Hi{run}"), None, None);
    assert_eq!(text(&r), Some("Hi"));
    let got = referral(&r);
    assert_eq!(got["gclid"], Value::Null);
    assert_eq!(got["gbraid"], GBRAID);
    for key in ["googleCampaignId", "googleAdGroupId", "googleAdId"] {
        assert_eq!(got[key], Value::Null, "{key}");
    }
}

#[test]
fn plain_ivs_and_malformed_text_stay_untouched_without_a_referral() {
    let malformed = format!(
        "Hello{}",
        encode_invisible(r#"{"gclid":"ABCDEFGHIJ1234567890""#)
    );
    for body in ["just text", "葛\u{E0100}", malformed.as_str()] {
        let r = parse(body, None, None);
        assert_eq!(text(&r), Some(body));
        assert!(r.referral.is_none(), "{body}");
    }
}

#[test]
fn merges_a_meta_ctwa_referral_with_the_google_click() {
    let ad = json!({ "source_url": "https://fb.me/3cr4Wqqkv", "source_id": "ad-1",
        "source_type": "ad", "ctwa_clid": "ctwa-1" });
    let r = parse(&format!("{}Hello", gclid_run()), Some(ad), None);
    let got = referral(&r);
    assert_eq!(got["adId"], "ad-1");
    assert_eq!(got["ctwaClid"], "ctwa-1");
    assert_eq!(got["sourcePlatform"], "facebook");
    assert_eq!(got["gclid"], GCLID);
    assert_eq!(got["googleCampaignId"], "123");
    // `referral.raw carries no click id`.
    assert!(got["raw"].is_object());
    assert!(!got["raw"].to_string().contains(GCLID));
}

#[test]
fn a_ref_text_with_a_run_keeps_the_ref_and_referral_source_stays_meta_only() {
    let r = parse(&format!("/ref-xyz{}", gclid_run()), None, None);
    assert_eq!(r.r#ref.as_deref(), Some("xyz"));
    assert_eq!(referral(&r)["gclid"], GCLID);
    let r = parse(&format!("{}Hello", gclid_run()), None, None);
    assert_eq!(r.referral_source, None);
}

#[test]
fn the_ctwa_referral_maps_the_ad_and_derives_the_platform() {
    let r = parse(
        "hello",
        Some(
            json!({ "source_url": "https://fb.me/3cr4Wqqkv", "source_id": "ad-1",
            "source_type": "ad", "headline": "Summer sale", "body": "Up to 50% off this week",
            "ctwa_clid": "ctwa-1" }),
        ),
        None,
    );
    let got = referral(&r);
    assert_eq!(got["adId"], "ad-1");
    assert_eq!(got["sourceUrl"], "https://fb.me/3cr4Wqqkv");
    assert_eq!(got["sourcePlatform"], "facebook");
    assert_eq!(got["ctwaClid"], "ctwa-1");
    assert_eq!(got["adTitle"], "Summer sale");
    assert_eq!(got["adBody"], "Up to 50% off this week");

    let r = parse(
        "hello",
        Some(json!({ "source_id": "ad-1", "source_type": "ad", "headline": "Sale" })),
        None,
    );
    assert_eq!(referral(&r)["adBody"], Value::Null);

    let r = parse(
        "hello",
        Some(
            json!({ "source_url": "https://www.instagram.com/p/Cxyz/", "source_id": "ad-2",
            "source_type": "ad" }),
        ),
        None,
    );
    assert_eq!(referral(&r)["sourcePlatform"], "instagram");

    let r = parse(
        "hello",
        Some(json!({ "source_id": "ad-3", "source_type": "ad" })),
        None,
    );
    assert_eq!(referral(&r)["sourcePlatform"], Value::Null);
    assert_eq!(referral(&r)["adId"], "ad-3");

    assert!(parse("hello", None, None).referral.is_none());
}
