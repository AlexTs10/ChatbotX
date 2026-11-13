//! Port of `apps/builder/src/features/contact-filter/schema/__tests__/ctwa-retarget-filter.test.ts`
//! (P3.6-D2 slice contact filter schema). Ported: `safeParse(..).success`
//! becomes `is_ok()`.

use chatbotx_business::contact_filter::parse_ctwa_retarget_condition;
use serde_json::{Value, json};

fn ok(condition: Value) -> bool {
    parse_ctwa_retarget_condition(&condition).is_ok()
}

#[test]
fn ctwa_retarget_condition_schema_accepts_a_valid_s_condition_with_an_ad_id() {
    for segment in ["conversations", "leads", "purchases"] {
        assert!(
            ok(
                json!({"field": "ctwaRetarget", "segment": segment, "adId": "238512000000102", "since": "2026-07-01", "until": "2026-07-31"})
            ),
            "{segment}"
        );
    }
}

#[test]
fn ctwa_retarget_condition_schema_accepts_a_condition_without_an_ad_id_all_ads() {
    assert!(ok(
        json!({"field": "ctwaRetarget", "segment": "purchases", "since": "2026-07-01", "until": "2026-07-31"})
    ));
}

#[test]
fn ctwa_retarget_condition_schema_rejects_an_unknown_segment() {
    assert!(!ok(
        json!({"field": "ctwaRetarget", "segment": "not-a-real-segment", "since": "2026-07-01", "until": "2026-07-31"})
    ));
}

#[test]
fn ctwa_retarget_condition_schema_rejects_a_non_yyyy_mm_dd_date() {
    assert!(!ok(
        json!({"field": "ctwaRetarget", "segment": "purchases", "since": "2026-07-01T00:00:00Z", "until": "2026-07-31"})
    ));
}

#[test]
fn ctwa_retarget_condition_schema_rejects_since_after_until() {
    assert!(!ok(
        json!({"field": "ctwaRetarget", "segment": "purchases", "since": "2026-07-31", "until": "2026-07-01"})
    ));
}

#[test]
fn ctwa_retarget_condition_schema_rejects_a_date_range_beyond_the_ads_analytics_cap_366_days() {
    assert!(!ok(
        json!({"field": "ctwaRetarget", "segment": "purchases", "since": "2024-01-01", "until": "2026-01-01"})
    ));
}

#[test]
fn ctwa_retarget_condition_schema_accepts_a_date_range_at_the_cap_boundary_366_days() {
    assert!(ok(
        json!({"field": "ctwaRetarget", "segment": "purchases", "since": "2025-01-01", "until": "2026-01-01"})
    ));
}

#[test]
fn ctwa_retarget_condition_schema_accepts_a_condition_narrowed_to_channel_s() {
    for channel in ["whatsapp", "messenger", "instagram"] {
        assert!(
            ok(
                json!({"field": "ctwaRetarget", "segment": "conversations", "channel": channel, "since": "2026-07-01", "until": "2026-07-31"})
            ),
            "{channel}"
        );
    }
}

#[test]
fn ctwa_retarget_condition_schema_accepts_a_condition_with_channel_omitted_any_channel_backward_compatible_with_saved_filters()
 {
    let parsed = parse_ctwa_retarget_condition(
        &json!({"field": "ctwaRetarget", "segment": "conversations", "since": "2026-07-01", "until": "2026-07-31"}),
    )
    .expect("parse");
    assert!(parsed.get("channel").is_none());
}

#[test]
fn ctwa_retarget_condition_schema_rejects_an_unknown_channel() {
    assert!(!ok(
        json!({"field": "ctwaRetarget", "segment": "conversations", "channel": "facebook", "since": "2026-07-01", "until": "2026-07-31"})
    ));
}
