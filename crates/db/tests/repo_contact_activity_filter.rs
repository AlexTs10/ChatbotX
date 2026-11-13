//! Port of `packages/database/__tests__/contact-activity-filter.test.ts` (P3.2b slice C2).
//! The TS rendered the RAW predicate with `PgDialect`; here the SQLite text and binds of the
//! same filter, prepared on the migrated schema (adapted: `$n::bigint` -> `?`, ids bind as
//! INTEGER, the row-value list is `IN (VALUES ..)`, a date window binds two integer ms instead
//! of `::timestamptz` text). The kill switch is a [`Keys`] argument
//! (`apply_contact_filter_with_keys`) instead of a mutated global `env`.

mod common;
#[path = "common/repo8.rs"]
mod repo8;

use chatbotx_db::repo::keys::Keys;
use chatbotx_db::repo::queries::contact_filter::apply_contact_filter_with_keys;
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::rusqlite::types::Value;
use repo8::{NOW, apply, apply_ws, crit, has, has_param, int, lacks, render, text};
use serde_json::{Value as Json, json};

const WORKSPACE_ID: i64 = 42;

/// `renderCondition(condition)` in workspace 42.
fn rc(c: Json) -> Sql {
    render(apply_ws(
        json!({ "operator": "and", "conditions": [c] }),
        WORKSPACE_ID,
    ))
}

/// `renderCondition(condition, null)`: no workspace.
fn rc_no_ws(c: Json) -> Sql {
    render(apply(json!({ "operator": "and", "conditions": [c] })))
}

/// `renderCondition(condition, WORKSPACE_ID, timezone)`.
fn rc_tz(c: Json, timezone: &str) -> Sql {
    render(apply_ws(
        json!({ "operator": "and", "conditions": [c], "timezone": timezone }),
        WORKSPACE_ID,
    ))
}

fn params_eq(q: &Sql, expected: &[Value]) {
    assert_eq!(q.params, expected, "{}", q.text);
}

/// The 9 `CONTACT_ACTIVITY_FILTERS` rows: `(field, activityType, referenceType, value)`.
macro_rules! activity_field_tests {
    ($module:ident, $field:literal, $activity:literal, $reference:literal, $sample:literal) => {
        mod $module {
            use super::*;

            fn cond(operator: &str) -> Json {
                json!({ "field": $field, "operator": operator, "value": [$sample] })
            }

            // ts:46 adapted
            #[test]
            fn in_is_an_exists_over_the_contact_s_activity() {
                let q = rc(cond("in"));
                has(&q, "EXISTS (SELECT 1 FROM \"ContactActivity\" a");
                lacks(&q, "NOT EXISTS");
                has_param(&q, text($activity));
                has_param(&q, text($reference));
            }

            // ts:46 adapted
            #[test]
            fn eq_is_an_exists_over_the_contact_s_activity() {
                let q = rc(cond("eq"));
                has(&q, "EXISTS (SELECT 1 FROM \"ContactActivity\" a");
                lacks(&q, "NOT EXISTS");
                has_param(&q, text($activity));
                has_param(&q, text($reference));
            }

            // ts:63 adapted
            #[test]
            fn not_in_is_the_exact_not_exists_complement() {
                has(&rc(cond("notIn")), "NOT EXISTS");
            }

            // ts:63 adapted
            #[test]
            fn ne_is_the_exact_not_exists_complement() {
                has(&rc(cond("ne")), "NOT EXISTS");
            }

            // ts:75 adapted
            #[test]
            fn is_empty_is_not_exists_of_any_activity_of_that_type_is_not_empty_is_exists() {
                let empty = rc(json!({ "field": $field, "operator": "isEmpty" }));
                let not_empty = rc(json!({ "field": $field, "operator": "isNotEmpty" }));
                has(&empty, "NOT EXISTS");
                lacks(&not_empty, "NOT EXISTS");
                lacks(&empty, "\"referenceId\"");
                lacks(&not_empty, "\"referenceId\"");
            }

            // ts:84 adapted
            #[test]
            fn scopes_every_subquery_by_a_parameterized_workspace_id_partition_pruning() {
                let q = rc(cond("in"));
                has(&q, "a.\"workspaceId\" = ?");
                assert_eq!(q.params[0], int(WORKSPACE_ID));
            }

            // ts:94 adapted
            #[test]
            fn fails_closed_without_a_workspace() {
                has(&rc_no_ws(cond("in")), "FALSE");
            }

            // ts:103 adapted
            #[test]
            fn fails_closed_for_an_unsupported_operator() {
                let q = rc(cond("contains"));
                has(&q, "FALSE");
                lacks(&q, "ContactActivity");
            }
        }
    };
}

activity_field_tests!(executed_flow, "executedFlow", "flowExecuted", "flow", "9");
activity_field_tests!(
    executed_step,
    "executedStep",
    "flowNodeExecuted",
    "flowNode",
    "7:9"
);
activity_field_tests!(
    keywords_received,
    "keywordsReceived",
    "automatedResponseMatched",
    "automatedResponse",
    "9"
);
activity_field_tests!(
    completed_whats_app_flows,
    "completedWhatsAppFlows",
    "formCompleted",
    "whatsappFlow",
    "9"
);
activity_field_tests!(email_sent, "emailSent", "messageSent", "emailTopic", "9");
activity_field_tests!(
    email_delivered,
    "emailDelivered",
    "messageDelivered",
    "emailTopic",
    "9"
);
activity_field_tests!(
    email_opened,
    "emailOpened",
    "messageOpened",
    "emailTopic",
    "9"
);
activity_field_tests!(
    email_clicked,
    "emailClicked",
    "messageClicked",
    "emailTopic",
    "9"
);
activity_field_tests!(clicked_ad, "clickedAd", "adClicked", "ad", "9");

// ts:114 adapted
#[test]
fn activity_backed_filters_a_flow_node_is_matched_by_the_flow_id_node_id_pair_never_the_node_alone()
{
    let q = rc(json!({ "field": "executedStep", "operator": "in", "value": ["7:9", "7:10"] }));
    has(&q, "(a.\"parentReferenceId\", a.\"referenceId\") IN");
    params_eq(
        &q,
        &[
            int(WORKSPACE_ID),
            text("flowNodeExecuted"),
            text("flowNode"),
            int(7),
            int(9),
            int(7),
            int(10),
        ],
    );
}

// ts:132 adapted
#[test]
fn activity_backed_filters_an_unscoped_reference_pins_the_parent_sentinel_so_the_primary_key_is_fully_used()
 {
    let q = rc(json!({ "field": "executedFlow", "operator": "in", "value": ["5"] }));
    has(&q, "a.\"parentReferenceId\" = ?");
    assert_eq!(q.params[3], int(0));
}

// ts:142 adapted
#[test]
fn activity_backed_filters_the_fixed_no_topic_value_maps_to_the_no_reference_sentinel_for_email_only()
 {
    let email = rc(json!({ "field": "emailSent", "operator": "in", "value": ["noTopic", "12"] }));
    params_eq(
        &email,
        &[
            int(WORKSPACE_ID),
            text("messageSent"),
            text("emailTopic"),
            int(0),
            int(-1),
            int(12),
        ],
    );
    let flow = rc(json!({ "field": "executedFlow", "operator": "in", "value": ["noTopic"] }));
    has(&flow, "FALSE");
}

// ts:165 adapted
#[test]
fn activity_backed_filters_duplicate_values_collapse_to_one_bound_id_each() {
    let q = rc(json!({
        "field": "keywordsReceived",
        "operator": "in",
        "value": ["5", "5", "6", "5"],
    }));
    assert_eq!(q.params.iter().filter(|p| **p == int(5)).count(), 1);
}

// ts:174 adapted
#[test]
fn activity_backed_filters_accepts_a_bare_string_as_a_single_value() {
    let q = rc(json!({ "field": "keywordsReceived", "operator": "eq", "value": "5" }));
    has_param(&q, int(5));
}

/// ts:183 "fails closed for %s" (keywordsReceived in).
fn fails_closed_keywords(value: Option<Json>) {
    let c = match value {
        Some(v) => json!({ "field": "keywordsReceived", "operator": "in", "value": v }),
        None => json!({ "field": "keywordsReceived", "operator": "in" }),
    };
    let q = rc(c);
    has(&q, "FALSE");
    lacks(&q, "ContactActivity");
}

// ts:183 adapted
#[test]
fn activity_backed_filters_fails_closed_for_no_value() {
    fails_closed_keywords(None);
}

// ts:183 adapted
#[test]
fn activity_backed_filters_fails_closed_for_an_empty_list() {
    fails_closed_keywords(Some(json!([])));
}

// ts:183 adapted
#[test]
fn activity_backed_filters_fails_closed_for_zero() {
    fails_closed_keywords(Some(json!(["0"])));
}

// ts:183 adapted
#[test]
fn activity_backed_filters_fails_closed_for_a_leading_zero() {
    fails_closed_keywords(Some(json!(["05"])));
}

// ts:183 adapted
#[test]
fn activity_backed_filters_fails_closed_for_a_negative_id() {
    fails_closed_keywords(Some(json!(["-1"])));
}

// ts:183 adapted
#[test]
fn activity_backed_filters_fails_closed_for_a_non_numeric_id() {
    fails_closed_keywords(Some(json!(["abc"])));
}

// ts:183 adapted
#[test]
fn activity_backed_filters_fails_closed_for_a_non_string_id() {
    fails_closed_keywords(Some(json!([5])));
}

// ts:183 adapted
#[test]
fn activity_backed_filters_fails_closed_for_an_id_past_bigint() {
    fails_closed_keywords(Some(json!(["9223372036854775808"])));
}

// ts:183 adapted
#[test]
fn activity_backed_filters_fails_closed_for_sql_in_the_value() {
    fails_closed_keywords(Some(json!(["1); DROP TABLE \"Contact\"; --"])));
}

/// ts:203 "fails closed for an executedStep value with %s".
fn fails_closed_step(value: &str) {
    has(
        &rc(json!({ "field": "executedStep", "operator": "in", "value": [value] })),
        "FALSE",
    );
}

// ts:203 adapted
#[test]
fn activity_backed_filters_fails_closed_for_an_executed_step_value_with_a_bare_node_id() {
    fails_closed_step("9");
}

// ts:203 adapted
#[test]
fn activity_backed_filters_fails_closed_for_an_executed_step_value_with_too_many_parts() {
    fails_closed_step("7:9:1");
}

// ts:203 adapted
#[test]
fn activity_backed_filters_fails_closed_for_an_executed_step_value_with_an_empty_flow() {
    fails_closed_step(":9");
}

// ts:203 adapted
#[test]
fn activity_backed_filters_fails_closed_for_an_executed_step_value_with_an_empty_node() {
    fails_closed_step("7:");
}

// ts:203 adapted
#[test]
fn activity_backed_filters_fails_closed_for_an_executed_step_value_with_a_non_numeric_node() {
    fails_closed_step("7:start-node");
}

// ts:203 adapted
#[test]
fn activity_backed_filters_fails_closed_for_an_executed_step_value_with_a_zero_flow() {
    fails_closed_step("0:9");
}

// ts:219 adapted
#[test]
fn activity_backed_filters_accepts_the_largest_bigint_and_rejects_more_than_500_values() {
    let maximum = rc(json!({
        "field": "executedFlow",
        "operator": "in",
        "value": ["9223372036854775807"],
    }));
    has_param(&maximum, int(i64::MAX));

    let many: Vec<String> = (1..=500).map(|i| i.to_string()).collect();
    assert_eq!(
        rc(json!({ "field": "executedFlow", "operator": "in", "value": many }))
            .params
            .len(),
        1 + 2 + 1 + 500
    );
    let mut more = many.clone();
    more.push("501".to_owned());
    has(
        &rc(json!({ "field": "executedFlow", "operator": "in", "value": more })),
        "FALSE",
    );
}

// ts:241 adapted
#[test]
fn activity_backed_filters_a_malformed_condition_never_widens_a_mixed_and_audience() {
    let q = render(apply_ws(
        json!({
            "operator": "and",
            "conditions": [
                { "field": "executedFlow", "operator": "in", "value": ["abc"] },
                { "field": "executedFlow", "operator": "in", "value": ["5"] },
            ],
        }),
        WORKSPACE_ID,
    ));
    has(&q, "FALSE");
    has(&q, "AND");
}

/// ts:260 "%s is a positive EXISTS over the contact's keyword messages".
fn sent_message_positive(operator: &str, fragment: &str) {
    let q = rc(json!({ "field": "sentMessage", "operator": operator, "value": "hi" }));
    has(&q, "FROM \"ContactKeywordMessage\" m");
    has(&q, fragment);
    lacks(&q, "NOT EXISTS");
}

// ts:260 adapted
#[test]
fn sent_message_eq_is_a_positive_exists_over_the_contact_s_keyword_messages() {
    sent_message_positive("eq", "\"normalizedText\" = ?");
}

// ts:260 adapted
#[test]
fn sent_message_starts_with_is_a_positive_exists_over_the_contact_s_keyword_messages() {
    sent_message_positive("startsWith", "\"normalizedText\" LIKE ?");
}

// ts:260 adapted
#[test]
fn sent_message_contains_is_a_positive_exists_over_the_contact_s_keyword_messages() {
    sent_message_positive("contains", "\"normalizedText\" LIKE ?");
}

// ts:260 adapted
#[test]
fn sent_message_ends_with_is_a_positive_exists_over_the_contact_s_keyword_messages() {
    sent_message_positive("endsWith", "\"normalizedText\" LIKE ?");
}

// ts:276 adapted
#[test]
fn sent_message_ne_is_not_exists_of_the_positive_form() {
    has(
        &rc(json!({ "field": "sentMessage", "operator": "ne", "value": "hi" })),
        "NOT EXISTS",
    );
}

// ts:276 adapted
#[test]
fn sent_message_not_contains_is_not_exists_of_the_positive_form() {
    has(
        &rc(json!({ "field": "sentMessage", "operator": "notContains", "value": "hi" })),
        "NOT EXISTS",
    );
}

// ts:288 adapted
#[test]
fn sent_message_is_empty_is_not_empty_test_for_any_stored_message() {
    has(
        &rc(json!({ "field": "sentMessage", "operator": "isEmpty" })),
        "NOT EXISTS",
    );
    lacks(
        &rc(json!({ "field": "sentMessage", "operator": "isNotEmpty" })),
        "NOT EXISTS",
    );
}

// ts:297 adapted
#[test]
fn sent_message_normalizes_the_query_value_exactly_like_the_stored_text() {
    let q = rc(json!({ "field": "sentMessage", "operator": "eq", "value": "  Xin Chào ĐẸP " }));
    params_eq(&q, &[int(WORKSPACE_ID), text("xin chao dep")]);
}

// ts:306 adapted
#[test]
fn sent_message_escapes_like_wildcards_in_the_value() {
    params_eq(
        &rc(json!({ "field": "sentMessage", "operator": "contains", "value": "100%_off" })),
        &[int(WORKSPACE_ID), text("%100\\%\\_off%")],
    );
    params_eq(
        &rc(json!({ "field": "sentMessage", "operator": "startsWith", "value": "a\\b" })),
        &[int(WORKSPACE_ID), text("a\\\\b%")],
    );
}

// ts:323 adapted
#[test]
fn sent_message_scopes_by_a_parameterized_workspace_id() {
    has(
        &rc(json!({ "field": "sentMessage", "operator": "eq", "value": "hi" })),
        "m.\"workspaceId\" = ?",
    );
}

/// ts:332 "fails closed for %s" (sentMessage contains).
fn sent_message_fails_closed(value: Option<Json>) {
    let c = match value {
        Some(v) => json!({ "field": "sentMessage", "operator": "contains", "value": v }),
        None => json!({ "field": "sentMessage", "operator": "contains" }),
    };
    has(&rc(c), "FALSE");
}

// ts:332 adapted
#[test]
fn sent_message_fails_closed_for_an_empty_value() {
    sent_message_fails_closed(Some(json!("")));
}

// ts:332 adapted
#[test]
fn sent_message_fails_closed_for_a_blank_value() {
    sent_message_fails_closed(Some(json!("   ")));
}

// ts:332 adapted
#[test]
fn sent_message_fails_closed_for_no_value() {
    sent_message_fails_closed(None);
}

// ts:332 adapted
#[test]
fn sent_message_fails_closed_for_a_non_string_value() {
    sent_message_fails_closed(Some(json!(["hi"])));
}

// ts:332 adapted
#[test]
fn sent_message_fails_closed_for_an_oversized_value() {
    sent_message_fails_closed(Some(json!("x".repeat(1001))));
}

// ts:345 adapted
#[test]
fn sent_message_text_that_is_blank_once_normalized_fails_closed_instead_of_matching_every_message()
{
    for operator in ["contains", "startsWith", "endsWith", "eq", "notContains"] {
        has(
            &rc(json!({ "field": "sentMessage", "operator": operator, "value": "\u{0301}" })),
            "FALSE",
        );
    }
}

// ts:364 adapted
#[test]
fn sent_message_fails_closed_for_an_unsupported_operator_or_without_a_workspace() {
    has(
        &rc(json!({ "field": "sentMessage", "operator": "gt", "value": "x" })),
        "FALSE",
    );
    has(
        &rc_no_ws(json!({ "field": "sentMessage", "operator": "eq", "value": "x" })),
        "FALSE",
    );
}

/// ts:378 "%s compares the contact's lastDeliveredAt inside EXISTS".
fn last_delivered_compares(operator: &str) {
    let q = rc(json!({ "field": "lastDelivered", "operator": operator, "value": "2026-05-19" }));
    has(&q, "FROM \"ContactState\" s");
    has(&q, "s.\"lastDeliveredAt\"");
    lacks(&q, "NOT EXISTS");
    has(&q, "s.\"workspaceId\" = ?");
    assert_eq!(q.params[0], int(WORKSPACE_ID));
}

// ts:378 adapted
#[test]
fn last_delivered_eq_compares_the_contact_s_last_delivered_at_inside_exists() {
    last_delivered_compares("eq");
}

// ts:378 adapted
#[test]
fn last_delivered_gt_compares_the_contact_s_last_delivered_at_inside_exists() {
    last_delivered_compares("gt");
}

// ts:378 adapted
#[test]
fn last_delivered_gte_compares_the_contact_s_last_delivered_at_inside_exists() {
    last_delivered_compares("gte");
}

// ts:378 adapted
#[test]
fn last_delivered_lt_compares_the_contact_s_last_delivered_at_inside_exists() {
    last_delivered_compares("lt");
}

// ts:378 adapted
#[test]
fn last_delivered_lte_compares_the_contact_s_last_delivered_at_inside_exists() {
    last_delivered_compares("lte");
}

// ts:396 adapted: the window binds two integer ms (TS: `::timestamptz` text).
#[test]
fn last_delivered_is_between_is_a_positive_exists_over_the_window() {
    let q = rc(json!({
        "field": "lastDelivered",
        "operator": "isBetween",
        "value": ["2026-05-01", "2026-05-31"],
    }));
    lacks(&q, "NOT EXISTS");
    params_eq(
        &q,
        &[
            int(WORKSPACE_ID),
            int(repo8::ms("2026-05-01T00:00:00.000Z")),
            int(repo8::ms("2026-06-01T00:00:00.000Z")),
        ],
    );
}

/// ts:406 "%s is NOT EXISTS so a contact with no delivery is included".
fn last_delivered_negative(operator: &str, value: Json) {
    let q = rc(json!({ "field": "lastDelivered", "operator": operator, "value": value }));
    has(&q, "NOT EXISTS");
    lacks(&q, "IS NULL OR");
}

// ts:406 adapted
#[test]
fn last_delivered_ne_is_not_exists_so_a_contact_with_no_delivery_is_included() {
    last_delivered_negative("ne", json!("2026-05-19"));
}

// ts:406 adapted
#[test]
fn last_delivered_not_between_is_not_exists_so_a_contact_with_no_delivery_is_included() {
    last_delivered_negative("notBetween", json!(["2026-05-01", "2026-05-31"]));
}

// ts:415 adapted
#[test]
fn last_delivered_is_empty_is_not_exists_of_a_value_is_not_empty_is_exists() {
    let empty = rc(json!({ "field": "lastDelivered", "operator": "isEmpty" }));
    let not_empty = rc(json!({ "field": "lastDelivered", "operator": "isNotEmpty" }));
    has(&empty, "NOT EXISTS");
    has(&empty, "\"lastDeliveredAt\" IS NOT NULL");
    lacks(&not_empty, "NOT EXISTS");
}

// ts:429 adapted
#[test]
fn last_delivered_interprets_a_naive_date_in_the_filter_s_timezone() {
    let c = json!({ "field": "lastDelivered", "operator": "eq", "value": "2026-05-19" });
    let utc = rc(c.clone());
    let tokyo = rc_tz(c, "Asia/Tokyo");
    assert_ne!(utc.params, tokyo.params);
}

// ts:443 adapted
#[test]
fn last_delivered_fails_closed_for_an_unusable_date() {
    has(
        &rc(json!({ "field": "lastDelivered", "operator": "eq", "value": "not-a-date" })),
        "FALSE",
    );
}

// ts:443 adapted
#[test]
fn last_delivered_fails_closed_for_no_value() {
    has(
        &rc(json!({ "field": "lastDelivered", "operator": "gt" })),
        "FALSE",
    );
}

// ts:443 adapted
#[test]
fn last_delivered_fails_closed_for_an_unsupported_operator() {
    has(
        &rc(json!({ "field": "lastDelivered", "operator": "contains", "value": "2026-05-19" })),
        "FALSE",
    );
}

// ts:453 adapted
#[test]
fn last_delivered_fails_closed_without_a_workspace() {
    has(
        &rc_no_ws(json!({ "field": "lastDelivered", "operator": "isEmpty" })),
        "FALSE",
    );
}

// ts:466 adapted: the switch is the `Keys` argument (no global env mutation).
#[test]
fn activity_filters_kill_switch_every_activity_condition_fails_closed_when_the_switch_is_off() {
    let off = Keys {
        contact_activity_filters_enabled: false,
    };
    let conditions = [
        json!({ "field": "executedFlow", "operator": "in", "value": ["5"] }),
        json!({ "field": "sentMessage", "operator": "eq", "value": "hi" }),
        json!({ "field": "lastDelivered", "operator": "isEmpty" }),
    ];
    for c in conditions {
        let q = render(apply_contact_filter_with_keys(
            &crit(json!({ "operator": "and", "conditions": [c] })),
            Some(WORKSPACE_ID),
            NOW,
            off,
        ));
        has(&q, "FALSE");
        lacks(&q, "ContactActivity");
        lacks(&q, "ContactKeywordMessage");
        lacks(&q, "ContactState");
    }
}

// ts:483 adapted
#[test]
fn activity_filters_kill_switch_the_conditions_work_again_once_it_is_back_on() {
    let on = Keys {
        contact_activity_filters_enabled: true,
    };
    let q = render(apply_contact_filter_with_keys(
        &crit(json!({
            "operator": "and",
            "conditions": [{ "field": "executedFlow", "operator": "in", "value": ["5"] }],
        })),
        Some(WORKSPACE_ID),
        NOW,
        on,
    ));
    has(&q, "ContactActivity");
}
