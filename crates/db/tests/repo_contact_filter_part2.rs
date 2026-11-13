//! Port of `packages/database/__tests__/contact-filter.test.ts` lines 2099-4576 (P3.2b slice
//! B: relation fields, Instagram, CTWA, custom / coupon / bot / leaderboard fields, keyword
//! search, working hours). Same adaptation as `repo_contact_filter.rs` (SQLite text + binds,
//! every predicate prepared on the migrated schema). Token map of the Postgres text:
//! `ILIKE` -> `ilike(`, `::numeric` -> `AS REAL)`, `~` -> `regexp(`, `::timestamptz` ->
//! `round(unixepoch(` (integer-ms instant), `::timestamp` -> `unixepoch(substr(` (wall
//! clock), `left(` -> `substr(.., 1, 10)`, `lower(btrim(` -> `lower_u(trim(`, `::boolean` ->
//! the truthy literal set `IN ('t', 'true', ..)`, `= true` -> `= 1`, `NOW()` -> bound now.
//! The boolean literal oracle runs on stored rows (`repo9`), as the TS comment asked.
//! Workspace `"ws-1"` is id 1; the non-numeric integration id `"iw-1"` is id 71.

mod common;
#[path = "common/repo8.rs"]
mod repo8;
#[path = "common/repo9.rs"]
mod repo9;

use chatbotx_db::repo::queries::contact_filter::{
    ContactWhereInput, SmartKeywordOptions, apply_contact_filter, build_contact_where,
    build_smart_keyword_where, contact_filter_uses_field, contact_inbox_has_dm_sql,
    prune_email_phone_filter_conditions,
};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::rusqlite::types::Value;
use repo8::{
    NOW, and1, apply, apply_ws, at, crit, first_and, has, has_param, int, lacks, lacks_param, one,
    prepares, real, render, same, text,
};
use serde_json::{Value as Json, json};

const TZ: &str = "round(unixepoch(";
const WALL: &str = "unixepoch(substr(";
const BOOL_CAST: &str = "IN ('t', 'true'";

macro_rules! rows {
    ($( $name:ident => $body:expr; )*) => { $( #[test] fn $name() { $body; } )* };
}

macro_rules! async_rows {
    ($( $name:ident => $body:expr; )*) => {
        $( #[tokio::test] async fn $name() { $body.await; } )*
    };
}

fn contains_all(q: &Sql, tokens: &[&str]) {
    for t in tokens {
        has(q, t);
    }
}

fn lacks_all(q: &Sql, tokens: &[&str]) {
    for t in tokens {
        lacks(q, t);
    }
}

// ---------------------------------------------------------------------------
// contactInbox relation fields
// ---------------------------------------------------------------------------

fn contact_inbox_relation(field: &str, column: &str) {
    let col = format!("\"ContactInbox\".\"{column}\" IN (");
    for op in ["in", "eq"] {
        let q = one(json!({ "field": field, "operator": op, "value": ["a"] }));
        has(&q, "EXISTS (SELECT 1 FROM \"ContactInbox\"");
        lacks(&q, "NOT EXISTS");
        has(&q, &col);
    }
    for op in ["notIn", "ne"] {
        let q = one(json!({ "field": field, "operator": op, "value": ["a"] }));
        has(&q, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
        has(&q, &col);
        lacks(&q, &format!("\"ContactInbox\".\"{column}\" NOT IN"));
    }
    let empty = one(json!({ "field": field, "operator": "isEmpty" }));
    has(&empty, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
}

rows! {
    // ts:2100 adapted
    apply_contact_filter_contact_inbox_relation_fields_renders_all_supported_current_channel_operators_as_exists_on_contact_inbox_channel => contact_inbox_relation("currentChannel", "channel");
    // ts:2100 adapted
    apply_contact_filter_contact_inbox_relation_fields_renders_all_supported_inbox_operators_as_exists_on_contact_inbox_inbox_id => contact_inbox_relation("inbox", "inboxId");
    // ts:2100 adapted
    apply_contact_filter_contact_inbox_relation_fields_renders_all_supported_language_operators_as_exists_on_contact_inbox_language => contact_inbox_relation("language", "language");
    // ts:2100 adapted
    apply_contact_filter_contact_inbox_relation_fields_renders_all_supported_source_operators_as_exists_on_contact_inbox_source => contact_inbox_relation("source", "source");
}

// ts:2138 adapted
#[test]
fn apply_contact_filter_contact_inbox_relation_fields_renders_language_is_empty_as_no_contact_inbox_with_a_non_empty_language()
 {
    let q = one(json!({ "field": "language", "operator": "isEmpty" }));
    has(&q, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(&q, "\"ContactInbox\".\"language\" IS NOT NULL");
    has(&q, "\"ContactInbox\".\"language\" <> ''");
}

// ts:2153 ported
#[test]
fn apply_contact_filter_contact_inbox_relation_fields_drops_unsupported_operators_for_relation_fields()
 {
    assert!(first_and("currentChannel", "contains", Some(json!("x"))).is_none());
}

// ts:2168 adapted
#[test]
fn apply_contact_filter_contact_inbox_relation_fields_renders_interacted_in_last24h_true_false_as_exists_not_exists()
 {
    let positive =
        one(json!({ "field": "interactedInLast24h", "operator": "eq", "value": "true" }));
    has(&positive, "EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(&positive, "\"ContactInbox\".\"lastIncomingMessageAt\" >= ?");
    has_param(&positive, int(NOW - 86_400_000));
    let negative =
        one(json!({ "field": "interactedInLast24h", "operator": "eq", "value": "false" }));
    has(&negative, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
    let empty = one(json!({ "field": "interactedInLast24h", "operator": "isEmpty" }));
    has(&empty, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(&empty, "\"ContactInbox\".\"lastIncomingMessageAt\" >= ?");
}

// ts:2219 adapted
#[test]
fn apply_contact_filter_contact_inbox_relation_fields_last_interaction_is_empty_is_not_empty_test_the_latest_inbound_message_for_null()
 {
    let never = one(json!({ "field": "lastInteraction", "operator": "isEmpty" }));
    has(
        &never,
        "SELECT MAX(\"ContactInbox\".\"lastIncomingMessageAt\")",
    );
    has(&never, "\"latestInteraction\".\"latest\" IS NULL");
    let ever = one(json!({ "field": "lastInteraction", "operator": "isNotEmpty" }));
    has(&ever, "\"latestInteraction\".\"latest\" IS NOT NULL");
}

// ts:2238 adapted
#[test]
fn apply_contact_filter_contact_inbox_relation_fields_renders_last_interaction_date_filters_against_latest_contact_inbox_last_incoming_message_at()
 {
    let max = "SELECT MAX(\"ContactInbox\".\"lastIncomingMessageAt\")";
    let eq = one(
        json!({ "field": "lastInteraction", "operator": "eq", "value": "2026-05-19T10:00:00Z" }),
    );
    lacks(&eq, "EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(&eq, max);
    has(&eq, "\"latestInteraction\".\"latest\" >=");
    lacks(&eq, "date_trunc");
    assert_eq!(
        eq.params,
        vec![
            at("2026-05-19T10:00:00.000Z"),
            at("2026-05-19T10:00:01.000Z")
        ]
    );
    let ne = one(
        json!({ "field": "lastInteraction", "operator": "ne", "value": "2026-05-19T10:00:00Z" }),
    );
    lacks(&ne, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(&ne, max);
    has(&ne, "\"latestInteraction\".\"latest\" >=");
    has(&ne, "IS NULL");
    let lt = one(
        json!({ "field": "lastInteraction", "operator": "lt", "value": "2026-02-01T00:00:00Z" }),
    );
    has(&lt, max);
    has(&lt, "<");
    let empty = one(json!({ "field": "lastInteraction", "operator": "isEmpty" }));
    lacks(&empty, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(&empty, max);
    has(&empty, "AS \"latest\"");
    has(&empty, "IS NULL");
    assert!(first_and("lastInteraction", "gt", Some(json!("not-a-date"))).is_none());
}

// ts:2328 adapted
#[test]
fn apply_contact_filter_contact_inbox_relation_fields_renders_last_user_input_filters_against_the_latest_inbound_contact_inbox_row()
 {
    let input =
        one(json!({ "field": "lastUserInput", "operator": "contains", "value": "invoice" }));
    has(&input, "SELECT \"ContactInbox\".\"lastUserInput\"");
    has(
        &input,
        "\"ContactInbox\".\"lastIncomingMessageAt\" IS NOT NULL",
    );
    has(
        &input,
        "ORDER BY \"ContactInbox\".\"lastIncomingMessageAt\" DESC",
    );
    has(
        &input,
        "ilike(CAST(\"latestInteraction\".\"latest\" AS TEXT), ?)",
    );
    has_param(&input, text("%invoice%"));
    let kind = one(json!({ "field": "lastUserInputType", "operator": "eq", "value": "image" }));
    has(&kind, "SELECT \"ContactInbox\".\"lastUserInputType\"");
    has(
        &kind,
        "ilike(CAST(\"latestInteraction\".\"latest\" AS TEXT), ?)",
    );
    has_param(&kind, text("image"));
}

// ---------------------------------------------------------------------------
// Instagram snapshots and commented posts
// ---------------------------------------------------------------------------

fn tri_state(field: &str, column: &str) {
    for (value, lit) in [("true", "1"), ("false", "0")] {
        let q = one(json!({ "field": field, "operator": "eq", "value": value }));
        has(&q, &format!("\"ContactInbox\".\"{column}\" = {lit}"));
    }
    let empty = one(json!({ "field": field, "operator": "isEmpty" }));
    has(&empty, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(
        &empty,
        &format!("\"ContactInbox\".\"{column}\" IS NOT NULL"),
    );
}

rows! {
    // ts:2372 adapted
    apply_contact_filter_instagram_snapshots_and_commented_posts_keeps_explicit_true_false_and_empty_distinct_for_follows_business_on_instagram => tri_state("followsBusinessOnInstagram", "followsBusiness");
    // ts:2372 adapted
    apply_contact_filter_instagram_snapshots_and_commented_posts_keeps_explicit_true_false_and_empty_distinct_for_business_follows_user_on_instagram => tri_state("businessFollowsUserOnInstagram", "businessFollowsContact");
    // ts:2372 adapted
    apply_contact_filter_instagram_snapshots_and_commented_posts_keeps_explicit_true_false_and_empty_distinct_for_verified_account_on_instagram => tri_state("verifiedAccountOnInstagram", "accountVerified");
}

// ts:2396 adapted
#[test]
fn apply_contact_filter_instagram_snapshots_and_commented_posts_uses_the_latest_instagram_follower_count_and_keeps_null_semantics()
 {
    for op in ["eq", "ne", "gt", "gte", "lt", "lte", "isEmpty"] {
        let c = if op == "isEmpty" {
            json!({ "field": "followerCountOnInstagram", "operator": op })
        } else {
            json!({ "field": "followerCountOnInstagram", "operator": op, "value": "10" })
        };
        let q = one(c);
        has(&q, "SELECT MAX(\"ContactInbox\".\"followerCount\")");
        if op == "ne" || op == "isEmpty" {
            has(&q, "IS NULL");
        }
    }
    assert!(first_and("followerCountOnInstagram", "eq", None).is_none());
}

fn commented(value: Option<Json>, op: &str, ws: Option<i64>) -> Sql {
    let c = match value {
        Some(v) => json!({ "field": "commentedOnPost", "operator": op, "value": v }),
        None => json!({ "field": "commentedOnPost", "operator": op }),
    };
    render(apply_contact_filter(&crit(and1(c)), ws, NOW))
}

// ts:2423 adapted
#[test]
fn apply_contact_filter_instagram_snapshots_and_commented_posts_scopes_commented_posts_to_a_workspace_and_rejects_malformed_persisted_ids()
 {
    has(
        &commented(Some(json!(["4", "4", "9"])), "eq", None),
        "FALSE",
    );
    let q = commented(Some(json!(["4", "4", "9"])), "eq", Some(42));
    has(&q, "JOIN \"ContactInboxPost\" p");
    has(&q, "p.\"workspaceId\" = ?");
    assert_eq!(q.params, vec![int(42), int(4), int(9)]);
    for value in [
        None,
        Some(json!([])),
        Some(json!(["0"])),
        Some(json!(["01"])),
        Some(json!(["x"])),
        Some(json!(["9223372036854775808"])),
    ] {
        let q = commented(value, "eq", Some(42));
        has(&q, "FALSE");
        lacks(&q, "ContactInboxPost");
    }
    let maximum = commented(Some(json!(["9223372036854775807"])), "eq", Some(42));
    has_param(&maximum, int(i64::MAX));
    let hundred: Vec<String> = (1..=100).map(|i| i.to_string()).collect();
    assert_eq!(
        commented(Some(json!(hundred)), "eq", Some(42)).params.len(),
        101
    );
    let mut too_many = hundred.clone();
    too_many.push("101".to_owned());
    has(&commented(Some(json!(too_many)), "eq", Some(42)), "FALSE");
}

// ts:2509 adapted
#[test]
fn apply_contact_filter_instagram_snapshots_and_commented_posts_a_malformed_commented_on_post_condition_never_widens_a_mixed_and_audience()
 {
    let q = render(apply_ws(
        json!({
            "operator": "and",
            "conditions": [
                { "field": "followsBusinessOnInstagram", "operator": "eq", "value": "true" },
                { "field": "commentedOnPost", "operator": "eq", "value": ["not-an-id"] },
            ],
        }),
        42,
    ));
    has(&q, "\"followsBusiness\"");
    has(&q, "FALSE");
}

// ts:2532 adapted
#[test]
fn apply_contact_filter_instagram_snapshots_and_commented_posts_an_unsupported_commented_on_post_operator_fails_closed()
 {
    has(
        &commented(Some(json!(["4"])), "contains", Some(42)),
        "FALSE",
    );
}

// ts:2548 adapted
#[test]
fn apply_contact_filter_instagram_snapshots_and_commented_posts_renders_ne_and_is_empty_as_not_exists_without_reading_an_empty_value()
 {
    has(&commented(Some(json!(["7"])), "ne", Some(42)), "NOT EXISTS");
    let empty = commented(Some(json!("junk")), "isEmpty", Some(42));
    has(&empty, "NOT EXISTS");
    lacks(&empty, "p.\"postId\"");
    assert_eq!(empty.params, vec![int(42)]);
}

// ---------------------------------------------------------------------------
// CTWA fields
// ---------------------------------------------------------------------------

// ts:2583 adapted
#[test]
fn apply_contact_filter_ctwa_fields_renders_from_ctwa_ad_true_false_as_exists_not_exists_on_contact_inbox_referral()
 {
    let positive = one(json!({ "field": "fromCtwaAd", "operator": "eq", "value": "true" }));
    has(&positive, "EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(
        &positive,
        "\"ContactInbox\".\"referral\"->>'ctwaClid' IS NOT NULL",
    );
    has(
        &positive,
        "\"ContactInbox\".\"referral\"->>'ctwaClid' <> ''",
    );
    has(
        &positive,
        "\"ContactInbox\".\"referral\"->>'adId' IS NOT NULL",
    );
    has(&positive, "\"ContactInbox\".\"referral\"->>'source' = ");
    has_param(&positive, text("ADS"));
    let negative = one(json!({ "field": "fromCtwaAd", "operator": "eq", "value": "false" }));
    has(&negative, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
    let empty = one(json!({ "field": "fromCtwaAd", "operator": "isEmpty" }));
    has(&empty, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
}

// ts:2638 adapted
#[test]
fn apply_contact_filter_ctwa_fields_renders_from_google_ad_as_exists_not_exists_on_the_gclid_gbraid_click_not_the_meta_predicate()
 {
    let positive = one(json!({ "field": "fromGoogleAd", "operator": "eq", "value": "true" }));
    has(&positive, "EXISTS (SELECT 1 FROM \"ContactInbox\"");
    lacks(&positive, "NOT EXISTS");
    has(
        &positive,
        "\"ContactInbox\".\"referral\"->>'gclid' IS NOT NULL",
    );
    has(
        &positive,
        "\"ContactInbox\".\"referral\"->>'gbraid' IS NOT NULL",
    );
    lacks(&positive, "ctwaClid");
    lacks(&positive, "'adId'");
    has(
        &one(json!({ "field": "fromGoogleAd", "operator": "eq", "value": "false" })),
        "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"",
    );
    has(
        &one(json!({ "field": "fromGoogleAd", "operator": "isEmpty" })),
        "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"",
    );
}

// ts:2670 adapted
#[test]
fn apply_contact_filter_ctwa_fields_renders_ctwa_conversion_in_not_in_as_exists_not_exists_joining_ads_conversion_event_through_contact_inbox()
 {
    let q =
        one(json!({ "field": "ctwaConversion", "operator": "in", "value": ["lead", "purchase"] }));
    has(&q, "EXISTS (SELECT 1 FROM \"AdsConversionEvent\"");
    has(
        &q,
        "INNER JOIN \"ContactInbox\" ON \"ContactInbox\".\"id\" = \"AdsConversionEvent\".\"contactInboxId\"",
    );
    has(&q, "\"ContactInbox\".\"contactId\" =");
    has(&q, "\"AdsConversionEvent\".\"eventType\" IN (");
    has_param(&q, text("lead"));
    has_param(&q, text("purchase"));
    let not_in = one(json!({ "field": "ctwaConversion", "operator": "notIn", "value": ["lead"] }));
    has(&not_in, "NOT EXISTS (SELECT 1 FROM \"AdsConversionEvent\"");
    has(&not_in, "\"AdsConversionEvent\".\"eventType\" IN (");
}

// ts:2709 adapted
#[test]
fn apply_contact_filter_ctwa_fields_renders_ctwa_conversion_is_empty_as_not_exists_with_no_event_type_predicate()
 {
    let q = one(json!({ "field": "ctwaConversion", "operator": "isEmpty" }));
    has(&q, "NOT EXISTS (SELECT 1 FROM \"AdsConversionEvent\"");
    lacks(&q, "\"AdsConversionEvent\".\"eventType\"");
}

// ---------------------------------------------------------------------------
// ctwaRetarget (the shared Rust predicate reads the aliases `ci` / `ace`)
// ---------------------------------------------------------------------------

fn retarget(extra: Json, ws: Option<i64>) -> Sql {
    let mut c = json!({ "field": "ctwaRetarget", "since": "2026-07-01", "until": "2026-07-31" });
    for (k, v) in extra.as_object().expect("object") {
        c[k] = v.clone();
    }
    render(apply_contact_filter(&crit(and1(c)), ws, NOW))
}

// ts:2726 adapted
#[test]
fn apply_contact_filter_ctwa_retarget_renders_conversations_segment_as_exists_on_contact_inbox_first_interaction_at_not_event_occurred_at()
 {
    let q = retarget(
        json!({ "segment": "conversations", "adId": "ad-1" }),
        Some(1),
    );
    has(&q, "EXISTS (SELECT 1 FROM \"ContactInbox\" AS ci");
    has(&q, "ci.\"contactId\" = \"Contact\".\"id\"");
    lacks(&q, "INNER JOIN \"Contact\"");
    has(&q, "ci.\"referral\"->>'ctwaClid' IS NOT NULL");
    has(&q, "ci.\"firstInteractionAt\"");
    lacks(&q, "ace.\"occurredAt\"");
    has(&q, "ci.\"referral\"->>'adId' = ");
    has_param(&q, text("ad-1"));
    has_param(&q, at("2026-07-01T00:00:00.000Z"));
    has_param(&q, at("2026-07-31T23:59:59.999Z"));
}

// ts:2767 adapted
#[test]
fn apply_contact_filter_ctwa_retarget_renders_leads_purchases_segments_as_exists_joining_ads_conversion_event_through_contact_inbox_on_the_same_row()
 {
    let leads = retarget(json!({ "segment": "leads" }), Some(1));
    has(
        &leads,
        "EXISTS (SELECT 1 FROM \"AdsConversionEvent\" AS ace",
    );
    has(
        &leads,
        "INNER JOIN \"ContactInbox\" AS ci ON ci.\"id\" = ace.\"contactInboxId\"",
    );
    has(&leads, "ci.\"contactId\" =");
    has(&leads, "ace.\"eventType\" = ");
    has_param(&leads, text("lead"));
    has_param(&leads, int(1));
    let purchases = retarget(json!({ "segment": "purchases", "adId": "ad-2" }), Some(1));
    has_param(&purchases, text("purchase"));
    has_param(&purchases, text("ad-2"));
    has_param(&purchases, int(1));
}

// ts:2816 adapted
#[test]
fn apply_contact_filter_ctwa_retarget_scopes_conversations_to_the_whats_app_integration_for_parity_with_the_facebook_path()
 {
    let q = retarget(
        json!({ "segment": "conversations", "integrationWhatsappId": "71" }),
        Some(1),
    );
    has(&q, "EXISTS (SELECT 1 FROM \"IntegrationWhatsapp\" AS i");
    has(&q, "i.\"inboxId\" = ci.\"inboxId\"");
    has_param(&q, int(71));
    has_param(&q, int(1));
}

// ts:2843 adapted
#[test]
fn apply_contact_filter_ctwa_retarget_scopes_leads_purchases_to_ads_conversion_event_integration_whatsapp_id()
 {
    let q = retarget(
        json!({ "segment": "purchases", "integrationWhatsappId": "71" }),
        Some(1),
    );
    has(&q, "ace.\"integrationWhatsappId\" = ");
    has_param(&q, text("purchase"));
    has_param(&q, int(71));
    has_param(&q, int(1));
}

// ts:2869 adapted
#[test]
fn apply_contact_filter_ctwa_retarget_omits_workspace_scoping_when_workspace_id_is_not_provided() {
    let q = retarget(json!({ "segment": "conversations" }), None);
    lacks(&q, "\"workspaceId\"");
    lacks(&q, "INNER JOIN");
}

// ts:2887 ported
#[test]
fn apply_contact_filter_ctwa_retarget_renders_no_predicate_for_a_malformed_ctwa_retarget_condition()
{
    assert!(
        apply(and1(
            json!({ "field": "ctwaRetarget", "segment": "not-a-real-segment" })
        ))
        .is_none()
    );
}

// ts:2901 adapted
#[test]
fn apply_contact_filter_ctwa_retarget_channel_omitted_matches_any_channel_ctwa_clid_or_ad_referral_conversations()
 {
    let q = retarget(json!({ "segment": "conversations" }), Some(1));
    has(&q, "ci.\"referral\"->>'ctwaClid' IS NOT NULL");
    has(&q, "ci.\"referral\"->>'source' = ");
    has_param(&q, text("ADS"));
}

// ts:2925 adapted
#[test]
fn apply_contact_filter_ctwa_retarget_channel_whatsapp_renders_the_ctwa_clid_keyed_predicate_pre_generalization_behavior()
 {
    let q = retarget(
        json!({ "segment": "conversations", "channel": "whatsapp" }),
        Some(1),
    );
    has(&q, "ci.\"referral\"->>'ctwaClid' IS NOT NULL");
}

// ts:2948 adapted
#[test]
fn apply_contact_filter_ctwa_retarget_channel_messenger_narrows_conversations_to_the_ad_referral_predicate_ad_id_source_ads()
 {
    let q = retarget(
        json!({ "segment": "conversations", "channel": "messenger" }),
        Some(1),
    );
    has(&q, "ci.\"referral\"->>'adId' IS NOT NULL");
    has(&q, "ci.\"referral\"->>'source' = ");
    has_param(&q, text("ADS"));
    lacks(&q, "ci.\"referral\"->>'ctwaClid' IS NOT NULL");
}

// ts:2976 adapted
#[test]
fn apply_contact_filter_ctwa_retarget_channel_instagram_narrows_leads_purchases_to_ads_conversion_event_channel()
 {
    let q = retarget(
        json!({ "segment": "leads", "channel": "instagram" }),
        Some(1),
    );
    has(&q, "ace.\"channel\" = ");
    has_param(&q, text("lead"));
    has_param(&q, text("instagram"));
    has_param(&q, int(1));
}

// ts:3000 ported
#[test]
fn apply_contact_filter_ctwa_retarget_renders_no_predicate_for_an_unrecognized_ctwa_retarget_channel()
 {
    assert!(
        apply(and1(json!({
            "field": "ctwaRetarget",
            "segment": "conversations",
            "channel": "facebook",
            "since": "2026-07-01",
            "until": "2026-07-31",
        })))
        .is_none()
    );
}

// ts:3016 ported
#[test]
fn apply_contact_filter_ctwa_retarget_prune_email_phone_filter_conditions_leaves_ctwa_retarget_intact()
 {
    let retarget = json!({
        "field": "ctwaRetarget",
        "segment": "purchases",
        "adId": "ad-1",
        "since": "2026-07-01",
        "until": "2026-07-31",
    });
    let f = crit(json!({
        "operator": "and",
        "conditions": [retarget.clone(), { "field": "email", "operator": "eq", "value": "ada@example.com" }],
    }));
    assert_eq!(
        prune_email_phone_filter_conditions(Some(&f), false),
        Some(crit(json!({ "operator": "and", "conditions": [retarget] })))
    );
}

// ---------------------------------------------------------------------------
// tags relation / conversation relation fields
// ---------------------------------------------------------------------------

fn tags(op: &str) {
    let q = one(json!({ "field": "tags", "operator": op, "value": ["tag-1"] }));
    if op == "in" || op == "eq" {
        has(&q, "EXISTS (SELECT 1 FROM \"ContactToTag\"");
        lacks(&q, "NOT EXISTS");
    } else {
        has(&q, "NOT EXISTS (SELECT 1 FROM \"ContactToTag\"");
    }
    has(&q, "\"ContactToTag\".\"tagId\" IN (");
    lacks(&q, "\"ContactToTag\".\"tagId\" NOT IN");
}

rows! {
    // ts:3039 adapted
    apply_contact_filter_tags_relation_renders_tags_in_as_exists_contact_to_tag_tag_id_in => tags("in");
    // ts:3039 adapted
    apply_contact_filter_tags_relation_renders_tags_eq_as_exists_contact_to_tag_tag_id_in => tags("eq");
    // ts:3039 adapted
    apply_contact_filter_tags_relation_renders_tags_not_in_as_exists_contact_to_tag_tag_id_in => tags("notIn");
    // ts:3039 adapted
    apply_contact_filter_tags_relation_renders_tags_ne_as_exists_contact_to_tag_tag_id_in => tags("ne");
}

// ts:3060 adapted
#[test]
fn apply_contact_filter_tags_relation_renders_tags_is_empty_as_not_exists_contact_to_tag() {
    has(
        &one(json!({ "field": "tags", "operator": "isEmpty" })),
        "NOT EXISTS (SELECT 1 FROM \"ContactToTag\"",
    );
}

// ts:3072 adapted
#[test]
fn apply_contact_filter_conversation_relation_fields_archived_is_empty_is_not_empty_eq_map_to_archived_at_null_checks()
 {
    let col = "\"Conversation\".\"archivedAt\" IS NOT NULL";
    let empty = one(json!({ "field": "archived", "operator": "isEmpty" }));
    has(&empty, "NOT EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&empty, col);
    let not_empty = one(json!({ "field": "archived", "operator": "isNotEmpty" }));
    has(&not_empty, "EXISTS (SELECT 1 FROM \"Conversation\"");
    lacks(&not_empty, "NOT EXISTS");
    has(&not_empty, col);
    let truthy = one(json!({ "field": "archived", "operator": "eq", "value": "true" }));
    has(&truthy, "EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&truthy, col);
    let falsy = one(json!({ "field": "archived", "operator": "eq", "value": "false" }));
    has(&falsy, "NOT EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&falsy, col);
}

// ts:3122 adapted
#[test]
fn apply_contact_filter_conversation_relation_fields_follow_up_eq_true_false_and_is_empty_map_to_the_followed_column()
 {
    let truthy = one(json!({ "field": "followUp", "operator": "eq", "value": "true" }));
    has(&truthy, "EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&truthy, "\"Conversation\".\"followed\" = 1");
    let falsy = one(json!({ "field": "followUp", "operator": "eq", "value": "false" }));
    has(&falsy, "NOT EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&falsy, "\"Conversation\".\"followed\" = 1");
    let empty = one(json!({ "field": "followUp", "operator": "isEmpty" }));
    has(&empty, "NOT EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&empty, "\"Conversation\".\"followed\" = 1");
}

// ts:3162 adapted
#[test]
fn apply_contact_filter_conversation_relation_fields_conversation_transferred_to_human_maps_to_active_bot_handoff_window()
 {
    let yes = one(
        json!({ "field": "conversationTransferredToHuman", "operator": "eq", "value": "true" }),
    );
    has(&yes, "EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&yes, "\"Conversation\".\"botEnabled\" = 0");
    has(&yes, "\"Conversation\".\"botResumeAt\" IS NULL");
    has(&yes, "\"Conversation\".\"botResumeAt\" > ?");
    has_param(&yes, int(NOW));
    let no = one(
        json!({ "field": "conversationTransferredToHuman", "operator": "eq", "value": "false" }),
    );
    has(&no, "NOT EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&no, "\"Conversation\".\"botEnabled\" = 0");
    has(&no, "\"Conversation\".\"botResumeAt\" > ?");
}

// ts:3203 ported
#[test]
fn contact_inbox_has_dm_sql_is_an_exists_over_the_contact_s_dm_never_a_comment_thread() {
    assert_eq!(
        contact_inbox_has_dm_sql().text,
        "EXISTS (SELECT 1 FROM \"Conversation\" WHERE \"Conversation\".\"contactId\" = \"ContactInbox\".\"contactId\" AND \"Conversation\".\"sourceId\" IS NULL)"
    );
}

// ts:3212 ported
#[test]
fn contact_filter_uses_field_is_true_only_when_a_condition_is_on_the_field() {
    let f = crit(json!({
        "operator": "or",
        "conditions": [
            { "field": "tag", "operator": "in", "value": ["vip"] },
            { "field": "lastInteraction", "operator": "isEmpty" },
        ],
    }));
    assert!(contact_filter_uses_field(Some(&f), "lastInteraction"));
    assert!(!contact_filter_uses_field(Some(&f), "lastSeen"));
}

// ts:3225 ported
#[test]
fn contact_filter_uses_field_is_false_for_a_missing_filter_or_malformed_conditions() {
    assert!(!contact_filter_uses_field(None, "lastInteraction"));
    let f = crit(json!({ "operator": "and", "conditions": [null, "lastInteraction", 1] }));
    assert!(!contact_filter_uses_field(Some(&f), "lastInteraction"));
}

// ---------------------------------------------------------------------------
// custom fields
// ---------------------------------------------------------------------------

/// The describe's `customField(valueType, operator, value, customFieldType)` helper
/// (`customFieldType` defaults to `datetime` for datetime values).
fn cf(value_type: &str, operator: &str, value: Option<Json>, cf_type: Option<&str>) -> Option<Sql> {
    let cf_type = cf_type.or(if value_type == "datetime" {
        Some("datetime")
    } else {
        None
    });
    let mut c = json!({ "field": "customField", "customFieldId": "cf-1", "valueType": value_type, "operator": operator });
    if let Some(t) = cf_type {
        c["customFieldType"] = json!(t);
    }
    if let Some(v) = value {
        c["value"] = v;
    }
    apply(and1(c))
}

fn negated_cf(
    value_type: &str,
    op: &str,
    value: Option<Json>,
    contains: &[&str],
    absent: &[&str],
    params: Vec<Value>,
) {
    let q = render(cf(value_type, op, value, None));
    has(&q, "NOT EXISTS (");
    contains_all(&q, contains);
    lacks_all(&q, absent);
    for p in params {
        has_param(&q, p);
    }
}

rows! {
    // ts:3268 adapted
    apply_contact_filter_custom_fields_renders_custom_field_number_ne_as_not_exists_over_a_positive_predicate => negated_cf("number", "ne", Some(json!("2")), &["AS REAL)", "regexp("], &["<>"], vec![real(2.0)]);
    // ts:3268 adapted
    apply_contact_filter_custom_fields_renders_custom_field_text_ne_as_not_exists_over_a_positive_predicate => negated_cf("text", "ne", Some(json!("vip")), &["\"ContactCustomField\".\"value\" ="], &["<>"], vec![text("vip")]);
    // ts:3268 adapted
    apply_contact_filter_custom_fields_renders_custom_field_datetime_ne_as_not_exists_over_a_positive_predicate => negated_cf("datetime", "ne", Some(json!("2026-05-19T10:00:00Z")), &[">=", "<"], &[" IS NOT TRUE OR ", " OR "], vec![text("cf-1"), at("2026-05-19T10:00:00.000Z"), at("2026-05-19T10:00:01.000Z")]);
    // ts:3268 adapted
    apply_contact_filter_custom_fields_renders_custom_field_text_not_contains_as_not_exists_over_a_positive_predicate => negated_cf("text", "notContains", Some(json!("vip")), &["ilike("], &["NOT ilike("], vec![text("%vip%")]);
    // ts:3268 adapted
    apply_contact_filter_custom_fields_renders_custom_field_number_not_contains_as_not_exists_over_a_positive_predicate => negated_cf("number", "notContains", Some(json!("2")), &["ilike("], &["NOT ilike("], vec![text("%2%")]);
    // ts:3268 adapted
    apply_contact_filter_custom_fields_renders_custom_field_number_not_between_as_not_exists_over_a_positive_predicate => negated_cf("number", "notBetween", Some(json!(["1", "5"])), &["AS REAL)", ">=", "<="], &[" OR "], vec![real(1.0), real(5.0)]);
    // ts:3268 adapted
    apply_contact_filter_custom_fields_renders_custom_field_datetime_not_between_as_not_exists_over_a_positive_predicate => negated_cf("datetime", "notBetween", Some(json!(["2026-01-01T00:00:00Z", "2026-12-31T00:00:00Z"])), &[">=", "<"], &[" OR "], vec![at("2026-01-01T00:00:00.000Z"), at("2026-12-31T00:00:01.000Z")]);
    // ts:3268 adapted
    apply_contact_filter_custom_fields_renders_custom_field_text_is_empty_as_not_exists_over_a_positive_predicate => negated_cf("text", "isEmpty", None, &["IS NOT NULL", "<> ''"], &[], vec![]);
    // ts:3268 adapted
    apply_contact_filter_custom_fields_renders_custom_field_boolean_is_empty_as_not_exists_over_a_positive_predicate => negated_cf("boolean", "isEmpty", None, &["IS NOT NULL"], &[], vec![]);
}

fn positive_cf(value_type: &str, op: &str, value: Option<Json>, contains: &[&str]) {
    let q = render(cf(value_type, op, value, None));
    has(&q, "EXISTS (");
    lacks(&q, "NOT EXISTS");
    contains_all(&q, contains);
}

rows! {
    // ts:3366 adapted
    apply_contact_filter_custom_fields_renders_positive_custom_field_text_eq_vip_as_exists => positive_cf("text", "eq", Some(json!("vip")), &["\"ContactCustomField\".\"value\" ="]);
    // ts:3366 adapted
    apply_contact_filter_custom_fields_renders_positive_custom_field_text_eq_1_as_exists => positive_cf("text", "eq", Some(json!("1")), &["\"ContactCustomField\".\"value\" ="]);
    // ts:3366 adapted
    apply_contact_filter_custom_fields_renders_positive_custom_field_text_is_not_empty_as_exists => positive_cf("text", "isNotEmpty", None, &["IS NOT NULL"]);
    // ts:3366 adapted
    apply_contact_filter_custom_fields_renders_positive_custom_field_text_contains_as_exists => positive_cf("text", "contains", Some(json!("vip")), &["ilike("]);
    // ts:3366 adapted
    apply_contact_filter_custom_fields_renders_positive_custom_field_text_starts_with_as_exists => positive_cf("text", "startsWith", Some(json!("vip")), &["ilike("]);
    // ts:3366 adapted
    apply_contact_filter_custom_fields_renders_positive_custom_field_text_ends_with_as_exists => positive_cf("text", "endsWith", Some(json!("vip")), &["ilike("]);
    // ts:3366 adapted
    apply_contact_filter_custom_fields_renders_positive_custom_field_number_gt_as_exists => positive_cf("number", "gt", Some(json!("12")), &["AS REAL)", ">"]);
    // ts:3366 adapted
    apply_contact_filter_custom_fields_renders_positive_custom_field_number_gte_as_exists => positive_cf("number", "gte", Some(json!("12")), &["AS REAL)", ">="]);
    // ts:3366 adapted
    apply_contact_filter_custom_fields_renders_positive_custom_field_number_lt_as_exists => positive_cf("number", "lt", Some(json!("12")), &["AS REAL)", "<"]);
    // ts:3366 adapted
    apply_contact_filter_custom_fields_renders_positive_custom_field_number_lte_as_exists => positive_cf("number", "lte", Some(json!("12")), &["AS REAL)", "<="]);
    // ts:3366 adapted
    apply_contact_filter_custom_fields_renders_positive_custom_field_number_is_between_as_exists => positive_cf("number", "isBetween", Some(json!(["10", "20"])), &["AS REAL)", ">=", "<="]);
}

// ts:3394 adapted
#[test]
fn apply_contact_filter_custom_fields_escapes_like_wildcards_for_custom_field_text_search() {
    let q = render(cf("text", "contains", Some(json!("100%_ready\\ok")), None));
    has_param(&q, text("cf-1"));
    has_param(&q, text("%100\\%\\_ready\\\\ok%"));
}

// ts:3403 ported
#[test]
fn apply_contact_filter_custom_fields_drops_numeric_custom_field_conditions_with_non_numeric_values()
 {
    assert!(cf("number", "gt", Some(json!("abc")), None).is_none());
}

// ts:3407 ported
#[test]
fn apply_contact_filter_custom_fields_drops_numeric_custom_field_negation_with_a_non_numeric_value()
{
    assert!(cf("number", "ne", Some(json!("abc")), None).is_none());
}

// ts:3411 ported
#[test]
fn apply_contact_filter_custom_fields_drops_datetime_custom_field_negation_with_an_invalid_date() {
    assert!(cf("datetime", "ne", Some(json!("not-a-date")), None).is_none());
}

// ts:3417 adapted
#[test]
fn apply_contact_filter_custom_fields_renders_date_custom_field_equality_as_a_day_only_comparison()
{
    let q = render(cf(
        "datetime",
        "eq",
        Some(json!("2026-07-22")),
        Some("date"),
    ));
    has(&q, "substr(\"ContactCustomField\".\"value\", 1, 10) = ?");
    lacks(&q, TZ);
    has_param(&q, text("2026-07-22"));
}

// ts:3427 adapted
#[test]
fn apply_contact_filter_custom_fields_resolves_legacy_temporal_custom_field_filters_without_saved_custom_field_type()
 {
    let q = one(json!({
        "field": "customField",
        "customFieldId": "cf-1",
        "valueType": "datetime",
        "operator": "eq",
        "value": "2026-07-22 09:30",
    }));
    has(&q, "FROM \"CustomField\"");
    has(&q, "\"CustomField\".\"type\" =");
    has(&q, WALL);
    has(&q, TZ);
    has_param(&q, text("date"));
    has_param(&q, text("2026-07-22T09:30:00"));
}

// ts:3451 adapted
#[test]
fn apply_contact_filter_custom_fields_renders_date_custom_field_equality_with_time_as_a_naive_wall_clock_minute_window()
 {
    let q = render(cf(
        "datetime",
        "eq",
        Some(json!("2026-07-22 09:30")),
        Some("date"),
    ));
    lacks(&q, TZ);
    has(&q, WALL);
    lacks(&q, ", 1, 10) = ?");
    has_param(&q, text("2026-07-22T09:30:00"));
    has_param(&q, text("2026-07-22T09:31:00"));
}

// ts:3468 adapted
#[test]
fn apply_contact_filter_custom_fields_compares_a_date_custom_field_with_an_explicit_offset_by_an_instant_window()
 {
    let q = render(cf(
        "datetime",
        "eq",
        Some(json!("2026-07-22T09:30:00+07:00")),
        Some("date"),
    ));
    has(&q, TZ);
    has_param(&q, at("2026-07-22T02:30:00.000Z"));
    has_param(&q, at("2026-07-22T02:30:01.000Z"));
}

fn date_only_window(op: &str, boundary: &str) {
    let q = render(cf("datetime", op, Some(json!("2026-07-22")), Some("date")));
    lacks(&q, TZ);
    has(&q, WALL);
    has_param(&q, text(boundary));
}

rows! {
    // ts:3485 adapted
    apply_contact_filter_custom_fields_compares_a_date_only_custom_field_gt_by_a_naive_day_window => date_only_window("gt", "2026-07-23T00:00:00");
    // ts:3485 adapted
    apply_contact_filter_custom_fields_compares_a_date_only_custom_field_gte_by_a_naive_day_window => date_only_window("gte", "2026-07-22T00:00:00");
    // ts:3485 adapted
    apply_contact_filter_custom_fields_compares_a_date_only_custom_field_lt_by_a_naive_day_window => date_only_window("lt", "2026-07-22T00:00:00");
    // ts:3485 adapted
    apply_contact_filter_custom_fields_compares_a_date_only_custom_field_lte_by_a_naive_day_window => date_only_window("lte", "2026-07-23T00:00:00");
}

// ts:3500 adapted
#[test]
fn apply_contact_filter_custom_fields_renders_date_custom_field_inequality_as_not_exists_over_a_day_comparison()
 {
    let q = render(cf(
        "datetime",
        "ne",
        Some(json!("2026-07-22")),
        Some("date"),
    ));
    has(&q, "NOT EXISTS (");
    has(&q, ", 1, 10) = ?");
    has_param(&q, text("2026-07-22"));
}

// ts:3510 adapted
#[test]
fn apply_contact_filter_custom_fields_compares_a_date_custom_field_range_by_naive_wall_clock() {
    let q = render(cf(
        "datetime",
        "isBetween",
        Some(json!(["2026-01-01", "2026-12-31"])),
        Some("date"),
    ));
    lacks(&q, TZ);
    has_param(&q, text("2026-01-01T00:00:00"));
    has_param(&q, text("2027-01-01T00:00:00"));
}

// ts:3526 adapted
#[test]
fn apply_contact_filter_custom_fields_compares_a_date_custom_field_range_by_instant_when_both_bounds_carry_an_offset()
 {
    let q = render(cf(
        "datetime",
        "isBetween",
        Some(json!([
            "2026-01-01T00:00:00+07:00",
            "2026-12-31T00:00:00+07:00"
        ])),
        Some("date"),
    ));
    has(&q, TZ);
    has_param(&q, at("2025-12-31T17:00:00.000Z"));
    has_param(&q, at("2026-12-30T17:00:01.000Z"));
}

// ts:3541 adapted
#[test]
fn apply_contact_filter_custom_fields_extends_a_datetime_range_upper_bound_to_the_end_of_the_typed_minute()
 {
    let q = render(apply(json!({
        "operator": "and",
        "timezone": "Asia/Ho_Chi_Minh",
        "conditions": [{
            "field": "customField",
            "customFieldId": "cf-1",
            "valueType": "datetime",
            "customFieldType": "datetime",
            "operator": "isBetween",
            "value": ["2026-07-02 12:12", "2026-07-02 12:12"],
        }],
    })));
    has(&q, ">=");
    has(&q, "<");
    lacks(&q, "<=");
    has_param(&q, at("2026-07-02T05:12:00.000Z"));
    has_param(&q, at("2026-07-02T05:13:00.000Z"));
}

fn guarded_cast(op: &str) {
    let q = render(cf(
        "datetime",
        op,
        Some(json!("2026-05-19T10:00:00Z")),
        None,
    ));
    has(&q, "EXISTS (");
    lacks(&q, "NOT EXISTS");
    has(&q, "CASE WHEN");
    has(&q, TZ);
}

fn plain_value(value_type: &str) {
    let q = render(cf(value_type, "eq", Some(json!("yes")), None));
    has(&q, "EXISTS (");
    lacks(&q, "NOT EXISTS");
    has(&q, "\"ContactCustomField\".\"value\" =");
    has_param(&q, text("yes"));
}

rows! {
    // ts:3569 adapted
    apply_contact_filter_custom_fields_renders_datetime_custom_field_comparison_gt_with_guarded_cast => guarded_cast("gt");
    // ts:3569 adapted
    apply_contact_filter_custom_fields_renders_datetime_custom_field_comparison_gte_with_guarded_cast => guarded_cast("gte");
    // ts:3569 adapted
    apply_contact_filter_custom_fields_renders_datetime_custom_field_comparison_lt_with_guarded_cast => guarded_cast("lt");
    // ts:3569 adapted
    apply_contact_filter_custom_fields_renders_datetime_custom_field_comparison_lte_with_guarded_cast => guarded_cast("lte");
    // ts:3584 adapted
    apply_contact_filter_custom_fields_renders_select_custom_field_eq_as_a_plain_value_comparison => plain_value("select");
    // ts:3584 adapted
    apply_contact_filter_custom_fields_renders_text_custom_field_eq_as_a_plain_value_comparison => plain_value("text");
}

// ---------------------------------------------------------------------------
// boolean custom fields
// ---------------------------------------------------------------------------

fn boolean_field(op: &str, value: Option<Json>) -> Option<Sql> {
    let mut c = json!({ "field": "customField", "customFieldId": repo9::CUSTOM_FIELD.to_string(), "valueType": "boolean", "operator": op });
    if let Some(v) = value {
        c["value"] = v;
    }
    apply(and1(c))
}

/// Stored value -> (eq true matches, eq false matches), evaluated on a stored row (the
/// row query runs the predicate, so no `render` inside the async test).
async fn boolean_oracle(stored: &str, eq_true: bool, eq_false: bool) {
    let db = repo9::custom_field_rows(&[stored]).await;
    let t = boolean_field("eq", Some(json!("true"))).expect("predicate");
    let f = boolean_field("eq", Some(json!("false"))).expect("predicate");
    assert_eq!(
        !repo9::matching(&db, &t).await.is_empty(),
        eq_true,
        "{stored:?} eq true"
    );
    assert_eq!(
        !repo9::matching(&db, &f).await.is_empty(),
        eq_false,
        "{stored:?} eq false"
    );
}

async_rows! {
    // ts:3643 adapted
    apply_contact_filter_boolean_custom_fields_stored_upper_true => boolean_oracle("TRUE", true, false);
    // ts:3643 adapted
    apply_contact_filter_boolean_custom_fields_stored_padded_upper_true => boolean_oracle(" TRUE ", true, false);
    // ts:3643 adapted
    apply_contact_filter_boolean_custom_fields_stored_true => boolean_oracle("true", true, false);
    // ts:3643 adapted
    apply_contact_filter_boolean_custom_fields_stored_1 => boolean_oracle("1", true, false);
    // ts:3643 adapted
    apply_contact_filter_boolean_custom_fields_stored_0 => boolean_oracle("0", false, true);
    // ts:3643 adapted
    apply_contact_filter_boolean_custom_fields_stored_upper_false => boolean_oracle("FALSE", false, true);
    // ts:3643 adapted
    apply_contact_filter_boolean_custom_fields_stored_f => boolean_oracle("f", false, true);
    // ts:3643 adapted
    apply_contact_filter_boolean_custom_fields_stored_yes => boolean_oracle("yes", true, false);
    // ts:3643 adapted
    apply_contact_filter_boolean_custom_fields_stored_off => boolean_oracle("off", false, true);
    // ts:3643 adapted
    apply_contact_filter_boolean_custom_fields_stored_empty => boolean_oracle("", false, false);
    // ts:3643 adapted
    apply_contact_filter_boolean_custom_fields_stored_12313 => boolean_oracle("12313", false, false);
}

// ts:3664 adapted
#[tokio::test]
async fn apply_contact_filter_boolean_custom_fields_drift_guard_every_canonical_boolean_literal_passes_the_sql_guard_regex_and_maps_to_the_same_boolean()
 {
    let truthy = ["t", "true", "y", "yes", "on", "1"];
    let falsy = ["f", "false", "n", "no", "off", "0"];
    let mut values: Vec<&str> = truthy.to_vec();
    values.extend(falsy);
    values.push("12313");
    let db = repo9::custom_field_rows(&values).await;
    let t = boolean_field("eq", Some(json!("true"))).expect("predicate");
    let f = boolean_field("eq", Some(json!("false"))).expect("predicate");
    assert_eq!(
        repo9::matching(&db, &t).await,
        (100..106).collect::<Vec<i64>>()
    );
    assert_eq!(
        repo9::matching(&db, &f).await,
        (106..112).collect::<Vec<i64>>()
    );
}

// ts:3680 adapted
#[test]
fn apply_contact_filter_boolean_custom_fields_renders_eq_true_with_a_whitespace_case_tolerant_guard_and_boolean_cast()
 {
    let q = render(boolean_field("eq", Some(json!("true"))));
    has(&q, "EXISTS (");
    lacks(&q, "NOT EXISTS");
    has(&q, "lower_u(trim(");
    has(&q, BOOL_CAST);
    has_param(&q, int(1));
}

// ts:3692 adapted
#[test]
fn apply_contact_filter_boolean_custom_fields_renders_eq_false_comparing_against_a_literal_false_parameter()
 {
    let q = render(boolean_field("eq", Some(json!("false"))));
    has(&q, "EXISTS (");
    lacks(&q, "NOT EXISTS");
    has_param(&q, int(0));
}

// ts:3702 adapted
#[test]
fn apply_contact_filter_boolean_custom_fields_is_empty_is_not_empty_for_boolean_fields_only_treat_empty_string_as_empty()
 {
    let empty = render(boolean_field("isEmpty", None));
    has(&empty, "NOT EXISTS (");
    has(&empty, "IS NOT NULL");
    has(&empty, "<> ''");
    let not_empty = render(boolean_field("isNotEmpty", None));
    has(&not_empty, "EXISTS (");
    lacks(&not_empty, "NOT EXISTS");
    has(&not_empty, "IS NOT NULL");
    has(&not_empty, "<> ''");
}

// ts:3719 ported
#[test]
fn apply_contact_filter_boolean_custom_fields_drops_a_boolean_custom_field_condition_with_a_blank_eq_value()
 {
    assert!(boolean_field("eq", Some(json!(""))).is_none());
}

// ts:3726 adapted
#[test]
fn apply_contact_filter_boolean_custom_fields_canonicalizes_a_non_canonical_truthy_operand_yes_to_a_true_comparison()
 {
    let q = render(boolean_field("eq", Some(json!("Yes"))));
    has(&q, BOOL_CAST);
    has_param(&q, int(1));
    lacks_param(&q, int(0));
}

// ts:3736 adapted
#[test]
fn apply_contact_filter_boolean_custom_fields_an_unrecognized_eq_operand_matches_nothing_false_predicate()
 {
    let q = render(boolean_field("eq", Some(json!("maybe"))));
    has(&q, "EXISTS (");
    has(&q, "FALSE");
    lacks(&q, BOOL_CAST);
}

// ts:3746 adapted
#[test]
fn apply_contact_filter_boolean_custom_fields_ne_with_an_unrecognized_operand_matches_every_row_not_exists_false()
 {
    let q = render(boolean_field("ne", Some(json!("maybe"))));
    has(&q, "NOT EXISTS (");
    has(&q, "FALSE");
}

// ts:3755 adapted
#[test]
fn apply_contact_filter_boolean_custom_fields_negates_eq_via_not_exists_if_ne_is_ever_received_for_a_boolean_field()
 {
    let q = render(boolean_field("ne", Some(json!("true"))));
    has(&q, "NOT EXISTS (");
    has(&q, BOOL_CAST);
    has_param(&q, int(1));
}

// ---------------------------------------------------------------------------
// couponTopic
// ---------------------------------------------------------------------------

fn coupon(op: &str, value: Option<Json>) -> Option<Sql> {
    let mut c = json!({ "field": "couponTopic", "topicId": "topic-1", "operator": op });
    if let Some(v) = value {
        c["value"] = v;
    }
    apply(and1(c))
}

// ts:3769 adapted
#[test]
fn apply_contact_filter_coupon_topic_dynamic_per_topic_field_is_not_empty_matches_contacts_issued_a_coupon_of_this_topic()
 {
    let q = render(coupon("isNotEmpty", None));
    has(&q, "EXISTS");
    lacks(&q, "usedAt");
    has_param(&q, text("topic-1"));
}

// ts:3787 adapted
#[test]
fn apply_contact_filter_coupon_topic_dynamic_per_topic_field_used_matches_contacts_who_redeemed_a_coupon_of_this_topic()
 {
    let q = render(coupon("used", None));
    has(&q, "EXISTS");
    has(&q, "usedAt");
    has_param(&q, text("topic-1"));
}

// ts:3805 not-applicable the Rust predicate always reads the outer table by its name "Contact" (no Drizzle table alias to correlate)
#[test]
fn apply_contact_filter_coupon_topic_dynamic_per_topic_field_used_correlates_workspace_through_the_active_contact_table_alias()
 {
    let q = render(coupon("used", None));
    has(
        &q,
        "\"Coupon\".\"workspaceId\" = \"Contact\".\"workspaceId\"",
    );
}

// ts:3828 adapted
#[test]
fn apply_contact_filter_coupon_topic_dynamic_per_topic_field_eq_scopes_to_a_specific_coupon_code_within_the_topic()
 {
    let q = render(coupon("eq", Some(json!("SAVE10"))));
    has(&q, "EXISTS");
    lacks(&q, "usedAt");
    assert_eq!(q.params, vec![text("topic-1"), text("SAVE10")]);
}

// ts:3847 adapted
#[test]
fn apply_contact_filter_coupon_topic_dynamic_per_topic_field_eq_escapes_like_wildcards_in_the_coupon_code_so_it_matches_exactly()
 {
    let q = render(coupon("eq", Some(json!("A%B_C"))));
    assert_eq!(q.params, vec![text("topic-1"), text("A\\%B\\_C")]);
    lacks_param(&q, text("A%B_C"));
}

// ts:3865 ported
#[test]
fn apply_contact_filter_coupon_topic_dynamic_per_topic_field_drops_the_condition_when_topic_id_is_missing()
 {
    assert!(first_and("couponTopic", "isNotEmpty", None).is_none());
}

// ts:3876 ported
#[test]
fn apply_contact_filter_coupon_topic_dynamic_per_topic_field_drops_unsupported_operators() {
    assert!(coupon("contains", Some(json!("x"))).is_none());
}

// ---------------------------------------------------------------------------
// bot fields
// ---------------------------------------------------------------------------

fn bf(
    value_type: &str,
    op: &str,
    value: Option<Json>,
    bf_type: Option<&str>,
    ws: Option<i64>,
) -> Option<Sql> {
    let mut c = json!({ "field": "botField", "botFieldId": "bf-1", "valueType": value_type, "operator": op });
    if let Some(t) = bf_type {
        c["botFieldType"] = json!(t);
    }
    if let Some(v) = value {
        c["value"] = v;
    }
    apply_contact_filter(&crit(and1(c)), ws, NOW)
}

// ts:3927 adapted
#[test]
fn apply_contact_filter_bot_fields_workspace_level_not_per_contact_renders_a_positive_comparison_as_an_exists_over_bot_field_scoped_by_workspace_id_bot_field_id()
 {
    let q = render(bf("text", "eq", Some(json!("vip")), None, Some(1)));
    has(&q, "EXISTS (");
    lacks(&q, "NOT EXISTS");
    has(&q, "\"BotField\"");
    has(&q, "\"BotField\".\"workspaceId\"");
    has(&q, "\"BotField\".\"id\"");
    has(&q, "\"BotField\".\"value\" =");
    lacks(&q, "ContactCustomField");
    lacks(&q, "\"contactId\"");
    has_param(&q, int(1));
    has_param(&q, text("bf-1"));
    has_param(&q, text("vip"));
}

// ts:3946 adapted
#[test]
fn apply_contact_filter_bot_fields_workspace_level_not_per_contact_renders_ne_as_not_exists_over_a_positive_eq_predicate_three_valued_negation()
 {
    let q = render(bf("text", "ne", Some(json!("vip")), None, Some(1)));
    has(&q, "NOT EXISTS (");
    has(&q, "\"BotField\".\"value\" =");
}

// ts:3955 adapted
#[test]
fn apply_contact_filter_bot_fields_workspace_level_not_per_contact_is_empty_is_not_empty_only_treat_null_or_empty_string_as_empty()
 {
    let empty = render(bf("text", "isEmpty", None, None, Some(1)));
    has(&empty, "NOT EXISTS (");
    has(&empty, "IS NOT NULL");
    has(&empty, "<> ''");
    let not_empty = render(bf("text", "isNotEmpty", None, None, Some(1)));
    has(&not_empty, "EXISTS (");
    lacks(&not_empty, "NOT EXISTS");
}

fn positive_bf(value_type: &str, op: &str, value: Json, contains: &[&str]) {
    let q = render(bf(value_type, op, Some(value), None, Some(1)));
    has(&q, "EXISTS (");
    lacks(&q, "NOT EXISTS");
    contains_all(&q, contains);
}

rows! {
    // ts:3970 adapted
    apply_contact_filter_bot_fields_workspace_level_not_per_contact_renders_positive_bot_field_number_gt_as_exists => positive_bf("number", "gt", json!("12"), &["AS REAL)", ">"]);
    // ts:3970 adapted
    apply_contact_filter_bot_fields_workspace_level_not_per_contact_renders_positive_bot_field_number_gte_as_exists => positive_bf("number", "gte", json!("12"), &["AS REAL)", ">="]);
    // ts:3970 adapted
    apply_contact_filter_bot_fields_workspace_level_not_per_contact_renders_positive_bot_field_number_is_between_as_exists => positive_bf("number", "isBetween", json!(["10", "20"]), &["AS REAL)", ">=", "<="]);
    // ts:3970 adapted
    apply_contact_filter_bot_fields_workspace_level_not_per_contact_renders_positive_bot_field_text_contains_as_exists => positive_bf("text", "contains", json!("vip"), &["ilike("]);
    // ts:3970 adapted
    apply_contact_filter_bot_fields_workspace_level_not_per_contact_renders_positive_bot_field_text_starts_with_as_exists => positive_bf("text", "startsWith", json!("vip"), &["ilike("]);
}

// ts:3990 adapted
#[test]
fn apply_contact_filter_bot_fields_workspace_level_not_per_contact_renders_boolean_eq_with_the_same_whitespace_case_tolerant_guard_as_custom_fields()
 {
    let q = render(bf("boolean", "eq", Some(json!("true")), None, Some(1)));
    has(&q, "EXISTS (");
    has(&q, "lower_u(trim(");
    has(&q, BOOL_CAST);
    has_param(&q, int(1));
}

// ts:4001 adapted
#[test]
fn apply_contact_filter_bot_fields_workspace_level_not_per_contact_renders_a_date_typed_datetime_condition_as_a_wall_clock_day_comparison_no_timestamptz()
 {
    let q = render(bf(
        "datetime",
        "eq",
        Some(json!("2026-07-22")),
        Some("date"),
        Some(1),
    ));
    has(&q, "EXISTS (");
    has(&q, "substr(\"BotField\".\"value\", 1, 10) = ?");
    lacks(&q, TZ);
    has_param(&q, text("2026-07-22"));
}

// ts:4012 adapted
#[test]
fn apply_contact_filter_bot_fields_workspace_level_not_per_contact_renders_a_datetime_typed_condition_as_a_zone_aware_instant_window()
 {
    let q = render(bf(
        "datetime",
        "eq",
        Some(json!("2026-05-19T10:00:00Z")),
        Some("datetime"),
        Some(1),
    ));
    has(&q, "EXISTS (");
    has(&q, TZ);
    has_param(&q, at("2026-05-19T10:00:00.000Z"));
}

// ts:4027 ported
#[test]
fn apply_contact_filter_bot_fields_workspace_level_not_per_contact_drops_a_datetime_condition_when_bot_field_type_is_omitted_no_legacy_fallback_for_bot_fields()
 {
    assert!(
        bf(
            "datetime",
            "eq",
            Some(json!("2026-05-19T10:00:00Z")),
            None,
            Some(1)
        )
        .is_none()
    );
}

// ts:4033 ported
#[test]
fn apply_contact_filter_bot_fields_workspace_level_not_per_contact_no_ops_without_bot_field_id() {
    assert!(
        apply_ws(
            and1(json!({ "field": "botField", "operator": "isNotEmpty" })),
            1
        )
        .is_none()
    );
}

// ts:4046 ported
#[test]
fn apply_contact_filter_bot_fields_workspace_level_not_per_contact_no_ops_without_a_workspace_id_scope_even_with_a_valid_condition()
 {
    assert!(bf("text", "isNotEmpty", None, None, None).is_none());
}

// ts:4065 adapted
#[test]
fn apply_contact_filter_bot_fields_workspace_level_not_per_contact_scopes_strictly_by_workspace_id_different_workspace_ids_render_different_params()
 {
    let a = render(bf("text", "isNotEmpty", None, None, Some(7)));
    let b = render(bf("text", "isNotEmpty", None, None, Some(8)));
    has_param(&a, int(7));
    has_param(&b, int(8));
    lacks_param(&a, int(8));
}

// ---------------------------------------------------------------------------
// operator combining / buildContactWhere base
// ---------------------------------------------------------------------------

// ts:4092 adapted
#[test]
fn apply_contact_filter_operator_combining_wraps_multiple_conditions_in_or_when_operator_is_or() {
    let q = render(apply(json!({
        "operator": "or",
        "conditions": [
            { "field": "fullName", "operator": "eq", "value": "Ada" },
            { "field": "email", "operator": "eq", "value": "a@b.co" },
        ],
    })));
    has(&q, "ilike(\"Contact\".\"fullName\", ?)");
    has(&q, ") OR (");
    has(&q, "ilike(\"Contact\".\"email\", ?)");
    assert_eq!(q.params, vec![text("Ada"), text("a@b.co")]);
}

// ts:4107 ported
#[test]
fn apply_contact_filter_operator_combining_returns_an_empty_object_for_empty_conditions() {
    assert!(apply(json!({ "operator": "and", "conditions": [] })).is_none());
}

// ts:4111 adapted
#[test]
fn apply_contact_filter_operator_combining_drops_unknown_fields_and_keeps_only_recognized_conditions()
 {
    let q = render(apply(json!({
        "operator": "and",
        "conditions": [
            { "field": "notARealField", "operator": "eq", "value": "x" },
            { "field": "email", "operator": "eq", "value": "a@b.co" },
        ],
    })));
    has(&q, "ilike(\"Contact\".\"email\", ?)");
    assert_eq!(q.params, vec![text("a@b.co")]);
}

fn workspace_where(contact_filter: Option<Json>) -> Sql {
    build_contact_where(&ContactWhereInput {
        workspace_id: 1,
        keyword: None,
        contact_filter: contact_filter.map(crit),
        now: NOW,
    })
}

// ts:4127 adapted
#[test]
fn apply_contact_filter_build_contact_where_base_returns_a_workspace_only_where_when_no_keyword_and_no_filter()
 {
    same(
        &Some(workspace_where(None)),
        "(\"Contact\".\"workspaceId\" = ?)",
        vec![int(1)],
    );
}

// ts:4133 adapted
#[test]
fn apply_contact_filter_build_contact_where_base_returns_a_workspace_only_where_when_the_filter_has_no_conditions()
 {
    same(
        &Some(workspace_where(Some(
            json!({ "operator": "and", "conditions": [] }),
        ))),
        "(\"Contact\".\"workspaceId\" = ?)",
        vec![int(1)],
    );
}

// ---------------------------------------------------------------------------
// buildSmartKeywordWhere
// ---------------------------------------------------------------------------

fn hidden() -> SmartKeywordOptions {
    SmartKeywordOptions {
        include_email_and_phone: false,
        ..SmartKeywordOptions::default()
    }
}

/// `renderContactWhere({ workspaceId: "ws-1", ...keywordWhere })`.
fn with_ws(keyword: Option<Sql>) -> Sql {
    let q = Sql::and([
        Some(Sql::bind("\"Contact\".\"workspaceId\" = ", 1_i64)),
        keyword,
    ]);
    prepares(&q);
    q
}

// ts:4144 adapted
#[test]
fn build_smart_keyword_where_searches_email_like_input_by_email_only_when_email_is_visible() {
    same(
        &build_smart_keyword_where("john@x.com", SmartKeywordOptions::default()),
        "ilike(\"Contact\".\"email\", ?)",
        vec![text("%john@x.com%")],
    );
}

fn names_and_source_id(q: &Sql) {
    has(q, "ilike(\"Contact\".\"firstName\", ?)");
    has(q, "ilike(\"Contact\".\"lastName\", ?)");
    has(q, "\"ContactInbox\".\"sourceId\" = ?");
}

// ts:4150 adapted
#[test]
fn build_smart_keyword_where_falls_back_to_names_and_source_id_for_email_like_input_when_email_is_hidden()
 {
    let q = with_ws(build_smart_keyword_where("john@x.com", hidden()));
    names_and_source_id(&q);
    lacks(&q, "ilike(\"Contact\".\"email\"");
}

// ts:4164 adapted
#[test]
fn build_smart_keyword_where_searches_phone_like_input_by_normalized_phone_only() {
    same(
        &build_smart_keyword_where("+84 90-123-4567", SmartKeywordOptions::default()),
        "ilike(\"Contact\".\"phoneNumber\", ?)",
        vec![text("%+84901234567%")],
    );
}

// ts:4170 adapted
#[test]
fn build_smart_keyword_where_falls_back_to_names_and_source_id_for_phone_like_input_when_phone_is_hidden()
 {
    let q = with_ws(build_smart_keyword_where("+84 90-123-4567", hidden()));
    names_and_source_id(&q);
    lacks(&q, "ilike(\"Contact\".\"phoneNumber\"");
}

// ts:4186 adapted
#[test]
fn build_smart_keyword_where_searches_pure_digits_across_visible_columns_and_exact_source_id() {
    let q = with_ws(build_smart_keyword_where(
        "12345",
        SmartKeywordOptions::default(),
    ));
    names_and_source_id(&q);
    has(&q, "ilike(\"Contact\".\"email\", ?)");
    has(&q, "ilike(\"Contact\".\"phoneNumber\", ?)");
    has(&q, "\"Contact\".\"id\" = ?");
    let like = text("%12345%");
    assert_eq!(
        q.params,
        vec![
            int(1),
            int(12345),
            like.clone(),
            like.clone(),
            like.clone(),
            like,
            text("12345")
        ]
    );
}

// ts:4209 adapted
#[test]
fn build_smart_keyword_where_a_pasted_contact_id_matches_that_contact_exactly() {
    let q = with_ws(build_smart_keyword_where(
        " 11753941635047880 ",
        SmartKeywordOptions::default(),
    ));
    has(&q, "\"Contact\".\"id\" = ?");
    assert_eq!(q.params[1], int(11_753_941_635_047_880));
}

// ts:4219 adapted
#[test]
fn build_smart_keyword_where_digit_runs_too_long_for_a_bigint_never_compare_against_the_id() {
    let q = with_ws(build_smart_keyword_where(
        "99999999999999999999",
        SmartKeywordOptions::default(),
    ));
    lacks(&q, "\"Contact\".\"id\" =");
    has(&q, "ilike(\"Contact\".\"firstName\", ?)");
}

// ts:4229 adapted
#[test]
fn build_smart_keyword_where_searches_plain_text_by_names_and_exact_source_id_only() {
    let q = with_ws(build_smart_keyword_where(
        "john",
        SmartKeywordOptions::default(),
    ));
    names_and_source_id(&q);
    lacks(&q, "ilike(\"Contact\".\"email\"");
    lacks(&q, "ilike(\"Contact\".\"phoneNumber\"");
    assert_eq!(
        q.params,
        vec![int(1), text("%john%"), text("%john%"), text("john")]
    );
}

// ts:4245 adapted
#[test]
fn build_smart_keyword_where_escapes_like_metacharacters_in_keyword_search() {
    let q = with_ws(build_smart_keyword_where(
        "100%_ready\\",
        SmartKeywordOptions::default(),
    ));
    has_param(&q, text("%100\\%\\_ready\\\\%"));
    has_param(&q, text("100%_ready\\"));
}

// ---------------------------------------------------------------------------
// unsupported operator fallbacks
// ---------------------------------------------------------------------------

fn dropped(field: &str, op: &str, value: Json) {
    assert!(first_and(field, op, Some(value)).is_none(), "{field} {op}");
}

rows! {
    // ts:4257 ported
    apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_interacted_in_last24h_with_unsupported_operator_contains => dropped("interactedInLast24h", "contains", json!("x"));
    // ts:4257 ported
    apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_tags_with_unsupported_operator_contains => dropped("tags", "contains", json!(["t"]));
    // ts:4257 ported
    apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_source_with_unsupported_operator_contains => dropped("source", "contains", json!(["s"]));
    // ts:4257 ported
    apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_inbox_with_unsupported_operator_gt => dropped("inbox", "gt", json!(["i"]));
    // ts:4257 ported
    apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_current_channel_with_unsupported_operator_gt => dropped("currentChannel", "gt", json!(["c"]));
    // ts:4257 ported
    apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_conversation_transferred_to_human_with_unsupported_operator_contains => dropped("conversationTransferredToHuman", "contains", json!("x"));
    // ts:4257 ported
    apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_subscribed_to_broadcast_with_unsupported_operator_contains => dropped("subscribedToBroadcast", "contains", json!("x"));
    // ts:4257 ported
    apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_blocked_with_unsupported_operator_gt => dropped("blocked", "gt", json!("x"));
    // ts:4257 ported
    apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_email_was_verified_with_unsupported_operator_ne => dropped("emailWasVerified", "ne", json!("true"));
    // ts:4257 ported
    apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_opted_in_for_email_with_unsupported_operator_ne => dropped("optedInForEmail", "ne", json!("true"));
    // ts:4257 ported
    apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_follow_up_with_unsupported_operator_ne => dropped("followUp", "ne", json!("true"));
    // ts:4257 ported
    apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_archived_with_unsupported_operator_in => dropped("archived", "in", json!(["x"]));
}

// ts:4279 ported
#[test]
fn apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_a_date_field_with_an_unsupported_operator_on_a_valid_value()
 {
    dropped("lastSeen", "contains", json!("2026-05-19T10:00:00Z"));
}

// ts:4294 ported
#[test]
fn apply_contact_filter_unsupported_operator_fallbacks_dropped_drops_an_unrecognized_column_operator()
 {
    dropped("fullName", "weirdOp", json!("x"));
}

// ts:4303 adapted
#[test]
fn apply_contact_filter_unsupported_operator_fallbacks_dropped_keeps_known_column_operators_unchanged()
 {
    same(
        &first_and("country", "eq", Some(json!("VN"))),
        "(\"Contact\".\"country\" = ?)",
        vec![text("VN")],
    );
}

// ---------------------------------------------------------------------------
// custom field remaining branches (no saved customFieldType)
// ---------------------------------------------------------------------------

fn cf_raw(value_type: &str, op: &str, value: Option<Json>) -> Option<Sql> {
    let mut c = json!({ "field": "customField", "customFieldId": "cf-1", "valueType": value_type, "operator": op });
    if let Some(v) = value {
        c["value"] = v;
    }
    apply(and1(c))
}

// ts:4335 ported
#[test]
fn apply_contact_filter_custom_field_remaining_branches_drops_number_is_between_when_the_value_is_not_a_valid_interval()
 {
    assert!(cf_raw("number", "isBetween", Some(json!("10"))).is_none());
}

// ts:4338 ported
#[test]
fn apply_contact_filter_custom_field_remaining_branches_drops_number_is_between_when_interval_bounds_are_non_numeric()
 {
    assert!(cf_raw("number", "isBetween", Some(json!(["a", "b"]))).is_none());
}

// ts:4341 adapted
#[test]
fn apply_contact_filter_custom_field_remaining_branches_renders_number_not_between_with_a_negated_numeric_guard()
 {
    let q = render(cf_raw("number", "notBetween", Some(json!(["10", "20"]))));
    has(&q, "AS REAL)");
    has(&q, "NOT");
}

// ts:4346 ported
#[test]
fn apply_contact_filter_custom_field_remaining_branches_drops_number_comparison_with_an_empty_value()
 {
    assert!(cf_raw("number", "eq", Some(json!(""))).is_none());
}

fn number_text_search(op: &str) {
    has(&render(cf_raw("number", op, Some(json!("12")))), "ilike(");
}

rows! {
    // ts:4349 adapted
    apply_contact_filter_custom_field_remaining_branches_renders_number_text_search_operator_not_contains_as_ilike => number_text_search("notContains");
    // ts:4349 adapted
    apply_contact_filter_custom_field_remaining_branches_renders_number_text_search_operator_starts_with_as_ilike => number_text_search("startsWith");
    // ts:4349 adapted
    apply_contact_filter_custom_field_remaining_branches_renders_number_text_search_operator_ends_with_as_ilike => number_text_search("endsWith");
}

// ts:4356 ported
#[test]
fn apply_contact_filter_custom_field_remaining_branches_drops_number_with_an_unsupported_operator()
{
    assert!(cf_raw("number", "in", Some(json!("12"))).is_none());
}

// ts:4361 ported
#[test]
fn apply_contact_filter_custom_field_remaining_branches_drops_datetime_is_between_when_a_bound_is_invalid()
 {
    assert!(
        cf_raw(
            "datetime",
            "isBetween",
            Some(json!(["not-a-date", "2026-05-31T23:59:59Z"]))
        )
        .is_none()
    );
}

// ts:4369 adapted
#[test]
fn apply_contact_filter_custom_field_remaining_branches_renders_datetime_not_between_with_a_guarded_cast()
 {
    let q = render(cf_raw(
        "datetime",
        "notBetween",
        Some(json!(["2026-05-01T00:00:00Z", "2026-05-31T23:59:59Z"])),
    ));
    has(&q, "CASE WHEN");
    has(&q, TZ);
}

// ts:4377 adapted
#[test]
fn apply_contact_filter_custom_field_remaining_branches_renders_datetime_is_between_with_timezone_aware_range_bounds()
 {
    let q = render(apply(json!({
        "operator": "and",
        "timezone": "Asia/Ho_Chi_Minh",
        "conditions": [{
            "field": "customField",
            "customFieldId": "cf-1",
            "valueType": "datetime",
            "operator": "isBetween",
            "value": ["2026-05-01 00:00", "2026-05-31 23:59"],
        }],
    })));
    has(&q, "CASE WHEN");
    has(&q, ">=");
    has(&q, "<");
    has_param(&q, at("2026-04-30T17:00:00.000Z"));
    has_param(&q, at("2026-05-31T17:00:00.000Z"));
}

// ts:4401 adapted
#[test]
fn apply_contact_filter_custom_field_remaining_branches_renders_datetime_ne_as_a_guarded_outside_the_day_range()
 {
    has(
        &render(cf_raw(
            "datetime",
            "ne",
            Some(json!("2026-05-19T10:00:00Z")),
        )),
        "CASE WHEN",
    );
}

// ts:4405 ported
#[test]
fn apply_contact_filter_custom_field_remaining_branches_drops_datetime_with_an_unsupported_operator()
 {
    assert!(cf_raw("datetime", "in", Some(json!("2026-05-19T10:00:00Z"))).is_none());
}

// ts:4412 ported
#[test]
fn apply_contact_filter_custom_field_remaining_branches_drops_text_custom_field_with_an_empty_value()
 {
    assert!(cf_raw("text", "eq", Some(json!(""))).is_none());
}

// ts:4415 ported
#[test]
fn apply_contact_filter_custom_field_remaining_branches_drops_text_custom_field_with_an_unsupported_operator()
 {
    assert!(cf_raw("text", "gt", Some(json!("x"))).is_none());
}

// ---------------------------------------------------------------------------
// W1 relation activations
// ---------------------------------------------------------------------------

fn broadcast_event(field: &str, predicate: &str) {
    let q = one(json!({ "field": field, "operator": "in", "value": ["option-1"] }));
    has(&q, "EXISTS (SELECT 1 FROM \"ContactOnBroadcast\"");
    has(&q, "\"ContactOnBroadcast\".\"broadcastId\" IN (");
    has(&q, predicate);
}

rows! {
    // ts:4435 adapted
    apply_contact_filter_w1_relation_activations_renders_broadcast_sent_against_contact_on_broadcast => broadcast_event("broadcastSent", "\"ContactOnBroadcast\".\"sent\" = 1");
    // ts:4435 adapted
    apply_contact_filter_w1_relation_activations_renders_broadcast_delivered_against_contact_on_broadcast => broadcast_event("broadcastDelivered", "\"ContactOnBroadcast\".\"deliveredAt\" IS NOT NULL");
    // ts:4435 adapted
    apply_contact_filter_w1_relation_activations_renders_broadcast_seen_against_contact_on_broadcast => broadcast_event("broadcastSeen", "\"ContactOnBroadcast\".\"seenAt\" IS NOT NULL");
    // ts:4435 adapted
    apply_contact_filter_w1_relation_activations_renders_broadcast_clicked_against_contact_on_broadcast => broadcast_event("broadcastClicked", "\"ContactOnBroadcast\".\"clickedAt\" IS NOT NULL");
    // ts:4435 adapted
    apply_contact_filter_w1_relation_activations_renders_broadcast_failed_against_contact_on_broadcast => broadcast_event("broadcastFailed", "\"ContactOnBroadcast\".\"failedAt\" IS NOT NULL");
}

// ts:4449 adapted
#[test]
fn apply_contact_filter_w1_relation_activations_renders_subscribed_to_drip_campaign_against_contact_on_sequence()
 {
    let q = one(
        json!({ "field": "subscribedToDripCampaign", "operator": "in", "value": ["option-1"] }),
    );
    has(&q, "EXISTS (SELECT 1 FROM \"ContactOnSequence\"");
    has(&q, "\"ContactOnSequence\".\"sequenceId\" IN (");
}

// ts:4456 adapted
#[test]
fn apply_contact_filter_w1_relation_activations_renders_entry_points_links_against_ref_link_stat() {
    let q = one(json!({ "field": "entryPointsLinks", "operator": "in", "value": ["option-1"] }));
    has(&q, "EXISTS (SELECT 1 FROM \"RefLinkStat\"");
    has(&q, "\"RefLinkStat\".\"linkId\" IN (");
}

// ts:4463 adapted
#[test]
fn apply_contact_filter_w1_relation_activations_preserves_extra_predicates_for_empty_broadcast_event_filters()
 {
    let q = one(json!({ "field": "broadcastDelivered", "operator": "isEmpty" }));
    has(&q, "NOT EXISTS (SELECT 1 FROM \"ContactOnBroadcast\"");
    has(&q, "\"ContactOnBroadcast\".\"deliveredAt\" IS NOT NULL");
}

// ---------------------------------------------------------------------------
// leaderboardScore
// ---------------------------------------------------------------------------

fn score_where(op: &str, value: Json) -> Option<Sql> {
    apply(and1(
        json!({ "field": "leaderboardScore", "leaderboardId": "77", "operator": op, "value": value }),
    ))
}

fn score_compare(op: &str, symbol: &str) {
    let q = render(score_where(op, json!("100")));
    has(&q, "COALESCE((");
    has(&q, &format!("), 0) {symbol} ?"));
    has_param(&q, int(77));
    has_param(&q, int(100));
}

rows! {
    // ts:4494 adapted
    apply_contact_filter_leaderboard_score_dynamic_per_leaderboard_field_eq_compares_the_score_with_eq_sign => score_compare("eq", "=");
    // ts:4494 adapted
    apply_contact_filter_leaderboard_score_dynamic_per_leaderboard_field_lt_compares_the_score_with_lt_sign => score_compare("lt", "<");
    // ts:4494 adapted
    apply_contact_filter_leaderboard_score_dynamic_per_leaderboard_field_lte_compares_the_score_with_lte_sign => score_compare("lte", "<=");
    // ts:4494 adapted
    apply_contact_filter_leaderboard_score_dynamic_per_leaderboard_field_gt_compares_the_score_with_gt_sign => score_compare("gt", ">");
    // ts:4494 adapted
    apply_contact_filter_leaderboard_score_dynamic_per_leaderboard_field_gte_compares_the_score_with_gte_sign => score_compare("gte", ">=");
}

// ts:4508 adapted
#[test]
fn apply_contact_filter_leaderboard_score_dynamic_per_leaderboard_field_a_contact_without_a_score_counts_as_0_and_the_lookup_is_workspace_scoped()
 {
    let q = render(score_where("lt", json!(5)));
    has(&q, "\"LeaderboardParticipant\".\"score\"");
    has(
        &q,
        "\"LeaderboardParticipant\".\"workspaceId\" = \"Contact\".\"workspaceId\"",
    );
    has(&q, ", 0)");
}

// ts:4517 ported
#[test]
fn apply_contact_filter_leaderboard_score_dynamic_per_leaderboard_field_unsupported_operators_or_missing_values_produce_no_condition()
 {
    assert!(score_where("contains", json!("1")).is_none());
    assert!(score_where("gt", json!("abc")).is_none());
}

// ---------------------------------------------------------------------------
// isWithinWorkingHours (the local "now" of the zone is computed in Rust and bound as the
// day text, the minute of the day and the weekday)
// ---------------------------------------------------------------------------

fn working_hours(op: &str, value: Json) -> Json {
    and1(json!({ "field": "isWithinWorkingHours", "operator": op, "value": value }))
}

// ts:4533 ported
#[test]
fn is_within_working_hours_predicate_no_ops_without_a_workspace_scope() {
    assert!(apply(working_hours("in", json!(["1"]))).is_none());
}

// ts:4537 ported
#[test]
fn is_within_working_hours_predicate_no_ops_for_unsupported_operators_and_unusable_selections() {
    assert!(apply_ws(working_hours("eq", json!(["1"])), 1).is_none());
    assert!(apply_ws(working_hours("in", json!(["abc"])), 1).is_none());
    assert!(apply_ws(working_hours("in", json!([])), 1).is_none());
}

// ts:4543 adapted
#[test]
fn is_within_working_hours_predicate_parameterises_schedule_ids_and_the_workspace_instead_of_inlining_them()
 {
    let q = render(apply_ws(working_hours("in", json!(["42", "43"])), 1));
    lacks(&q, "pg_timezone_names");
    has_param(&q, int(1));
    has_param(&q, int(42));
    has_param(&q, int(43));
    // NOW = 2026-06-01T00:00Z, a Monday: UTC day and minute 0.
    has_param(&q, text("2026-06-01"));
    has_param(&q, int(0));
}

// ts:4553 adapted
#[test]
fn is_within_working_hours_predicate_binds_the_caller_resolved_workspace_timezone_as_a_parameter() {
    let mut f = working_hours("in", json!(["42"]));
    f["workingHoursTimezone"] = json!("Asia/Ho_Chi_Minh");
    let q = render(apply_ws(f, 1));
    // 07:00 local time in Ho Chi Minh City.
    has_param(&q, int(420));
}

// ts:4566 adapted
#[test]
fn is_within_working_hours_predicate_not_in_negates_the_whole_selection_and_assigned_agent_is_correlated_to_the_contact()
 {
    let q = render(apply_ws(
        working_hours("notIn", json!(["assignedAgent"])),
        1,
    ));
    assert!(q.text.starts_with("(NOT ("), "{}", q.text);
    has(&q, "\"Conversation\".\"contactId\" = \"Contact\".\"id\"");
}
