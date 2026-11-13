//! Port of `packages/database/__tests__/ctwa-retarget-predicate.test.ts` (P3.2b slice D1) on
//! `queries::ctwa_retarget::build_ctwa_segment_predicate` (ported in P3.2; the predicate reads
//! the aliases `ci` = `ContactInbox`, `ace` = `AdsConversionEvent`). Adapted: SQLite text +
//! binds; the non-numeric integration ids `"im-1"` / `"ig-1"` are ids 81 / 82, workspace
//! `"ws-1"` is 1; the always-false predicate is `0`.

use chatbotx_db::repo::queries::ctwa_retarget::{
    CtwaSegmentPredicateInput, build_ctwa_segment_predicate,
};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::rusqlite::types::Value;

const SINCE: i64 = 1_785_542_400_000; // 2026-08-01T00:00:00.000Z
const UNTIL: i64 = 1_786_406_399_999; // 2026-08-10T23:59:59.999Z

fn input(segment: &str, channel: Option<&str>) -> CtwaSegmentPredicateInput {
    CtwaSegmentPredicateInput {
        segment: segment.to_owned(),
        channel: channel.map(str::to_owned),
        since: SINCE,
        until: UNTIL,
        ..CtwaSegmentPredicateInput::default()
    }
}

fn messenger_id(mut i: CtwaSegmentPredicateInput) -> CtwaSegmentPredicateInput {
    i.integration_messenger_id = Some(81);
    i.workspace_id = Some(1);
    i
}

fn instagram_id(mut i: CtwaSegmentPredicateInput) -> CtwaSegmentPredicateInput {
    i.integration_instagram_id = Some(82);
    i.workspace_id = Some(1);
    i
}

fn text(v: &str) -> Value {
    Value::Text(v.to_owned())
}

fn has(q: &Sql, s: &str) {
    assert!(q.text.contains(s), "expected {s:?} in {}", q.text);
}

fn lacks(q: &Sql, s: &str) {
    assert!(!q.text.contains(s), "unexpected {s:?} in {}", q.text);
}

fn param(q: &Sql, v: Value, present: bool) {
    assert_eq!(q.params.contains(&v), present, "{v:?} in {:?}", q.params);
}

// ts:12 adapted
#[test]
fn build_ctwa_segment_predicate_conversations_segment_channel_scoping_messenger_without_an_integration_id_still_excludes_instagram_channel_contact_inbox_rows()
 {
    let q = build_ctwa_segment_predicate(&input("conversations", Some("messenger")));
    has(&q, "ci.\"channel\" = ");
    param(&q, text("messenger"), true);
    param(&q, text("instagram"), false);
}

// ts:27 adapted
#[test]
fn build_ctwa_segment_predicate_conversations_segment_channel_scoping_instagram_without_an_integration_id_still_excludes_messenger_channel_contact_inbox_rows()
 {
    let q = build_ctwa_segment_predicate(&input("conversations", Some("instagram")));
    has(&q, "ci.\"channel\" = ");
    param(&q, text("instagram"), true);
    param(&q, text("messenger"), false);
}

// ts:42 adapted
#[test]
fn build_ctwa_segment_predicate_conversations_segment_channel_scoping_messenger_with_an_integration_id_keeps_the_channel_scope_alongside_the_exists_scope()
 {
    let q = build_ctwa_segment_predicate(&messenger_id(input("conversations", Some("messenger"))));
    has(&q, "ci.\"channel\" = ");
    has(&q, "EXISTS");
    param(&q, text("messenger"), true);
    param(&q, Value::Integer(81), true);
}

// ts:60 adapted
#[test]
fn build_ctwa_segment_predicate_conversations_segment_channel_scoping_instagram_with_an_integration_id_keeps_the_channel_scope_alongside_the_exists_scope()
 {
    let q = build_ctwa_segment_predicate(&instagram_id(input("conversations", Some("instagram"))));
    has(&q, "ci.\"channel\" = ");
    has(&q, "EXISTS");
    param(&q, text("instagram"), true);
    param(&q, Value::Integer(82), true);
}

// ts:78 adapted
#[test]
fn build_ctwa_segment_predicate_conversations_segment_channel_scoping_whatsapp_channel_omitted_keeps_the_original_ctwa_clid_predicate_with_no_contact_inbox_channel_scope()
 {
    let q = build_ctwa_segment_predicate(&input("conversations", None));
    has(&q, "ctwaClid");
    lacks(&q, "ci.\"channel\" = ");
}

// ts:91 adapted
#[test]
fn build_ctwa_segment_predicate_conversations_segment_channel_scoping_whatsapp_channel_explicit_keeps_the_ctwa_clid_predicate_with_no_contact_inbox_channel_scope()
 {
    let q = build_ctwa_segment_predicate(&input("conversations", Some("whatsapp")));
    has(&q, "ctwaClid");
    lacks(&q, "ci.\"channel\" = ");
}

// ts:105 adapted
#[test]
fn build_ctwa_segment_predicate_conversations_segment_channel_scoping_integration_messenger_id_alone_no_channel_still_scopes_to_messenger_the_legacy_caller_fallback()
 {
    let q = build_ctwa_segment_predicate(&messenger_id(input("conversations", None)));
    has(&q, "ci.\"channel\" = ");
    has(&q, "EXISTS");
    param(&q, text("messenger"), true);
    param(&q, Value::Integer(81), true);
    param(&q, text("instagram"), false);
}

// ts:123 adapted
#[test]
fn build_ctwa_segment_predicate_conversations_segment_channel_scoping_integration_instagram_id_alone_no_channel_still_scopes_to_instagram_the_legacy_caller_fallback()
 {
    let q = build_ctwa_segment_predicate(&instagram_id(input("conversations", None)));
    has(&q, "ci.\"channel\" = ");
    has(&q, "EXISTS");
    param(&q, text("instagram"), true);
    param(&q, Value::Integer(82), true);
    param(&q, text("messenger"), false);
}

// ts:141 adapted
#[test]
fn build_ctwa_segment_predicate_conversations_segment_channel_scoping_facebook_workspace_wide_lead_ads_produces_an_always_false_predicate_never_the_any_channel_fallback()
 {
    let q = build_ctwa_segment_predicate(&input("conversations", Some("facebook")));
    assert_eq!(q.text.trim(), "0");
    assert!(q.params.is_empty());
    lacks(&q, "ctwaClid");
    lacks(&q, "referral");
}

// ts:161 adapted
#[test]
fn build_ctwa_segment_predicate_leads_purchases_segment_channel_scoping_integration_messenger_id_alone_no_channel_scopes_ads_conversion_event_channel_to_messenger()
 {
    let q = build_ctwa_segment_predicate(&messenger_id(input("leads", None)));
    has(&q, "ace.\"channel\" = ");
    param(&q, text("messenger"), true);
    param(&q, text("instagram"), false);
}

// ts:177 adapted
#[test]
fn build_ctwa_segment_predicate_leads_purchases_segment_channel_scoping_integration_instagram_id_alone_no_channel_scopes_ads_conversion_event_channel_to_instagram()
 {
    let q = build_ctwa_segment_predicate(&instagram_id(input("purchases", None)));
    has(&q, "ace.\"channel\" = ");
    param(&q, text("instagram"), true);
    param(&q, text("messenger"), false);
}
