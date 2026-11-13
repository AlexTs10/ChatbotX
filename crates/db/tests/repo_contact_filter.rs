//! Port of `packages/database/__tests__/contact-filter.test.ts` lines 1-1765 (P3.2b slice
//! A: helpers, permission pruning, the first `applyContactFilter` block). The TS rendered the
//! Drizzle where with `PgDialect` and checked the Postgres text; here the same checks run on
//! the SQLite text and bind values (adapted: `ilike(col, ?)` for `ILIKE`, integer unix-ms
//! binds for `::timestamptz` instants, bound `now` for `NOW()`), and every rendered
//! predicate is also prepared and run on the migrated schema (`repo8::prepares`).
//! Workspace `"ws-1"` is workspace id 1.

mod common;
#[path = "common/repo8.rs"]
mod repo8;

use chatbotx_db::repo::queries::contact_filter::value_format::is_valid_date_time_filter_value;
use chatbotx_db::repo::queries::contact_filter::{
    ContactWhereInput, ConversationAssigneeSelection, build_contact_inbox_contact_filter_sql,
    build_contact_where, contact_filter_has_predicate, parse_conversation_assignee_values,
    prune_email_phone_filter_conditions,
};
use chatbotx_db::repo::utils::{escape_like_pattern, like_contains};
use repo8::{
    NOW, and1, apply, at, count, crit, first_and, has, has_param, int, lacks, lacks_param, one,
    prepares, real, render, same, text,
};
use serde_json::json;

const MIN: i64 = 60_000;

// ---------------------------------------------------------------------------
// LIKE pattern helpers / value-format helpers
// ---------------------------------------------------------------------------

// ts:44 ported
#[test]
fn like_pattern_helpers_escapes_like_metacharacters_and_wraps_contains_patterns() {
    assert_eq!(escape_like_pattern("100%_ready\\"), "100\\%\\_ready\\\\");
    assert_eq!(like_contains("100%_ready\\"), "%100\\%\\_ready\\\\%");
}

fn accepts(v: &str) {
    assert!(is_valid_date_time_filter_value(v), "{v}");
}

fn rejects(v: &str) {
    assert!(!is_valid_date_time_filter_value(v), "{v}");
}

// ts:51 ported
#[test]
fn contact_filter_value_format_helpers_accepts_valid_datetime_value_2026_05_19() {
    accepts("2026-05-19");
}
// ts:51 ported
#[test]
fn contact_filter_value_format_helpers_accepts_valid_datetime_value_2026_05_19_10_30() {
    accepts("2026-05-19 10:30");
}
// ts:51 ported
#[test]
fn contact_filter_value_format_helpers_accepts_valid_datetime_value_2026_05_19t10_30_00z() {
    accepts("2026-05-19T10:30:00Z");
}
// ts:51 ported
#[test]
fn contact_filter_value_format_helpers_accepts_valid_datetime_value_with_offset_plus_07_00() {
    accepts("2026-05-19T10:30:00+07:00");
}
// ts:51 ported
#[test]
fn contact_filter_value_format_helpers_accepts_valid_datetime_value_2024_02_29() {
    accepts("2024-02-29");
}
// ts:61 ported
#[test]
fn contact_filter_value_format_helpers_rejects_invalid_datetime_value_empty() {
    rejects("");
}
// ts:61 ported
#[test]
fn contact_filter_value_format_helpers_rejects_invalid_datetime_value_junk() {
    rejects("junk");
}
// ts:61 ported
#[test]
fn contact_filter_value_format_helpers_rejects_invalid_datetime_value_2026_13_40() {
    rejects("2026-13-40");
}
// ts:61 ported
#[test]
fn contact_filter_value_format_helpers_rejects_invalid_datetime_value_2026_05_19_25_99() {
    rejects("2026-05-19 25:99");
}
// ts:61 ported
#[test]
fn contact_filter_value_format_helpers_rejects_invalid_datetime_value_2026_02_30() {
    rejects("2026-02-30");
}
// ts:61 ported
#[test]
fn contact_filter_value_format_helpers_rejects_invalid_datetime_value_2026_02_29() {
    rejects("2026-02-29");
}
// ts:61 ported
#[test]
fn contact_filter_value_format_helpers_rejects_invalid_datetime_value_2026_04_31() {
    rejects("2026-04-31");
}

// ---------------------------------------------------------------------------
// contact filter permission helpers
// ---------------------------------------------------------------------------

// ts:77 ported
#[test]
fn contact_filter_permission_helpers_drops_only_email_phone_contact_filter_fields_when_email_and_phone_are_denied()
 {
    let f = crit(json!({
        "operator": "and",
        "conditions": [
            { "field": "email", "operator": "eq", "value": "ada@example.com" },
            { "field": "phone", "operator": "eq", "value": "+84912345678" },
            { "field": "hasContactInfo", "operator": "in", "value": ["email"] },
            { "field": "emailWasVerified", "operator": "eq", "value": "true" },
            { "field": "optedInForEmail", "operator": "eq", "value": "true" },
            { "field": "existingContact", "operator": "eq", "value": "true" },
            { "field": "fullName", "operator": "contains", "value": "Ada" },
        ],
    }));
    assert_eq!(
        prune_email_phone_filter_conditions(Some(&f), false),
        Some(crit(json!({
            "operator": "and",
            "conditions": [{ "field": "fullName", "operator": "contains", "value": "Ada" }],
        })))
    );
}

// ts:97 ported
#[test]
fn contact_filter_permission_helpers_keeps_all_contact_filter_fields_when_email_and_phone_are_allowed()
 {
    let f = crit(json!({
        "operator": "or",
        "conditions": [
            { "field": "email", "operator": "eq", "value": "ada@example.com" },
            { "field": "fullName", "operator": "contains", "value": "Ada" },
        ],
    }));
    assert_eq!(prune_email_phone_filter_conditions(Some(&f), true), Some(f));
}

// ts:111 ported
#[test]
fn contact_filter_permission_helpers_normalizes_an_emptied_filter_back_to_and() {
    let f = crit(json!({
        "operator": "or",
        "conditions": [{ "field": "email", "operator": "eq", "value": "a@b.co" }],
    }));
    assert_eq!(
        prune_email_phone_filter_conditions(Some(&f), false),
        Some(crit(json!({ "operator": "and", "conditions": [] })))
    );
}

// ts:123 ported
#[test]
fn contact_filter_permission_helpers_preserves_the_filter_timezone_while_pruning_email_phone_fields()
 {
    let f = crit(json!({
        "operator": "and",
        "timezone": "Asia/Ho_Chi_Minh",
        "conditions": [
            { "field": "email", "operator": "eq", "value": "ada@example.com" },
            { "field": "customField", "operator": "isBetween", "value": ["2026-07-01", "2026-07-31"] },
        ],
    }));
    assert_eq!(
        prune_email_phone_filter_conditions(Some(&f), false),
        Some(crit(json!({
            "operator": "and",
            "timezone": "Asia/Ho_Chi_Minh",
            "conditions": [
                { "field": "customField", "operator": "isBetween", "value": ["2026-07-01", "2026-07-31"] },
            ],
        })))
    );
}

// ts:153 ported
#[test]
fn contact_filter_permission_helpers_preserves_the_filter_timezone_even_when_every_condition_is_pruned()
 {
    let f = crit(json!({
        "operator": "or",
        "timezone": "America/New_York",
        "conditions": [{ "field": "email", "operator": "eq", "value": "a@b.co" }],
    }));
    assert_eq!(
        prune_email_phone_filter_conditions(Some(&f), false),
        Some(crit(
            json!({ "operator": "and", "timezone": "America/New_York", "conditions": [] })
        ))
    );
}

// ---------------------------------------------------------------------------
// applyContactFilter
// ---------------------------------------------------------------------------

// ts:168 ported
#[test]
fn apply_contact_filter_reports_whether_a_non_empty_filter_compiles_to_an_effective_predicate() {
    let unknown_only = crit(and1(
        json!({ "field": "unknownField", "operator": "eq", "value": "x" }),
    ));
    let valid = crit(and1(
        json!({ "field": "fullName", "operator": "contains", "value": "Ada" }),
    ));
    assert!(
        apply(and1(
            json!({ "field": "unknownField", "operator": "eq", "value": "x" })
        ))
        .is_none()
    );
    assert!(!contact_filter_has_predicate(&unknown_only, None));
    assert!(contact_filter_has_predicate(&valid, None));
}

// ts:195 ported
#[test]
fn apply_contact_filter_threads_workspace_id_through_so_a_bot_field_only_criteria_reports_a_real_predicate()
 {
    let f = crit(and1(json!({
        "field": "botField",
        "botFieldId": "bf-1",
        "valueType": "text",
        "operator": "isNotEmpty",
    })));
    assert!(!contact_filter_has_predicate(&f, None));
    assert!(contact_filter_has_predicate(&f, Some(1)));
}

// ts:216 adapted
#[test]
fn apply_contact_filter_maps_inbox_filters_to_an_exists_contact_inbox_inbox_id_subquery() {
    let q = one(json!({ "field": "inbox", "operator": "in", "value": ["123", "456"] }));
    has(&q, "EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(&q, "\"ContactInbox\".\"inboxId\" IN (");
    has_param(&q, int(123));
    has_param(&q, int(456));
}

// ts:236 adapted
#[test]
fn apply_contact_filter_renders_multiple_and_conditions_on_contact_inboxes_as_exists_subqueries() {
    let q = render(apply(json!({
        "operator": "and",
        "conditions": [
            { "field": "currentChannel", "operator": "in", "value": ["messenger"] },
            { "field": "inbox", "operator": "in", "value": ["123"] },
        ],
    })));
    has(&q, "\"ContactInbox\".\"channel\" IN (");
    has(&q, "\"ContactInbox\".\"inboxId\" IN (");
    assert_eq!(count(&q, "EXISTS"), 2);
}

// ts:260 adapted
#[test]
fn apply_contact_filter_maps_tag_filters_to_an_exists_contact_to_tag_subquery() {
    let q = one(json!({ "field": "tags", "operator": "eq", "value": ["tag-1"] }));
    has(&q, "EXISTS (SELECT 1 FROM \"ContactToTag\"");
    has(&q, "\"ContactToTag\".\"tagId\" IN (");
    has_param(&q, text("tag-1"));
}

// ts:279 adapted
#[test]
fn apply_contact_filter_maps_a_custom_field_condition_to_an_exists_raw_filter() {
    let q = one(json!({
        "field": "customField",
        "customFieldId": "cf-1",
        "valueType": "text",
        "operator": "eq",
        "value": "vip",
    }));
    has(&q, "EXISTS (SELECT 1 FROM \"ContactCustomField\"");
}

// ts:298 ported
#[test]
fn apply_contact_filter_ignores_a_custom_field_condition_without_a_custom_field_id() {
    let w = apply(and1(json!({
        "field": "customField",
        "valueType": "text",
        "operator": "eq",
        "value": "vip",
    })));
    assert!(w.is_none());
}

// ts:314 adapted
#[test]
fn apply_contact_filter_supports_text_search_operators_for_number_custom_fields() {
    let q = one(json!({
        "field": "customField",
        "customFieldId": "cf-1",
        "valueType": "number",
        "operator": "contains",
        "value": "12",
    }));
    has(&q, "EXISTS (SELECT 1 FROM \"ContactCustomField\"");
    has_param(&q, text("%12%"));
}

// ts:333 adapted
#[test]
fn apply_contact_filter_renders_static_starts_with_filters_with_supported_sql() {
    let q = one(json!({ "field": "fullName", "operator": "startsWith", "value": "Al" }));
    has(&q, "ilike(\"Contact\".\"fullName\", ?)");
    has_param(&q, text("Al%"));
}

fn static_text_operator(operator: &str, param: &str) {
    let q = one(json!({ "field": "email", "operator": operator, "value": "Al" }));
    has(&q, "\"Contact\".\"email\"");
    has(&q, "ilike(");
    has_param(&q, text(param));
}

// ts:351 adapted
#[test]
fn apply_contact_filter_renders_static_text_operator_starts_with_as_supported_sql() {
    static_text_operator("startsWith", "Al%");
}
// ts:351 adapted
#[test]
fn apply_contact_filter_renders_static_text_operator_ends_with_as_supported_sql() {
    static_text_operator("endsWith", "%Al");
}
// ts:351 adapted
#[test]
fn apply_contact_filter_renders_static_text_operator_contains_as_supported_sql() {
    static_text_operator("contains", "%Al%");
}

// ts:374 ported
#[test]
fn apply_contact_filter_escapes_like_wildcards_for_static_text_filters() {
    let q = one(json!({ "field": "email", "operator": "contains", "value": "100%_ready\\ok" }));
    has_param(&q, text("%100\\%\\_ready\\\\ok%"));
}

// ts:391 adapted
#[test]
fn apply_contact_filter_renders_static_free_text_eq_as_wildcard_free_case_insensitive_sql() {
    let q = one(json!({ "field": "fullName", "operator": "eq", "value": "JOHN%_DOE\\" }));
    has(&q, "ilike(\"Contact\".\"fullName\", ?)");
    has_param(&q, text("JOHN\\%\\_DOE\\\\"));
    lacks_param(&q, text("%JOHN\\%\\_DOE\\\\%"));
}

// ts:410 adapted
#[test]
fn apply_contact_filter_renders_static_free_text_ne_as_not_ilike_plus_null_fallback() {
    let q = one(json!({ "field": "email", "operator": "ne", "value": "John@Example.com" }));
    has(&q, "NOT ilike(\"Contact\".\"email\", ?)");
    has(&q, "\"Contact\".\"email\" IS NULL");
    has_param(&q, text("John@Example.com"));
}

// ts:436 adapted
#[test]
fn apply_contact_filter_renders_static_free_text_eq_on_email_phone_as_case_insensitive_column_equality()
 {
    let email = one(json!({ "field": "email", "operator": "eq", "value": "Ada@Example.com" }));
    has(&email, "ilike(\"Contact\".\"email\", ?)");
    has_param(&email, text("Ada@Example.com"));
    let phone = one(json!({ "field": "phone", "operator": "eq", "value": "+15551234567" }));
    has(&phone, "ilike(\"Contact\".\"phoneNumber\", ?)");
    has_param(&phone, text("+15551234567"));
}

// ts:470 adapted
#[test]
fn apply_contact_filter_keeps_enum_and_select_equality_case_sensitive() {
    let gender = one(json!({ "field": "gender", "operator": "eq", "value": "male" }));
    let country = one(json!({ "field": "country", "operator": "eq", "value": "VN" }));
    has(&gender, "\"Contact\".\"gender\" =");
    lacks(&gender, "ilike");
    has(&country, "\"Contact\".\"country\" =");
    lacks(&country, "ilike");
}

// ts:502 adapted
#[test]
fn apply_contact_filter_maps_dropdown_eq_ne_array_values_to_exists_not_exists_in_subqueries() {
    let channel = one(json!({
        "field": "currentChannel",
        "operator": "eq",
        "value": ["messenger", "whatsapp"],
    }));
    has(&channel, "\"ContactInbox\".\"channel\" IN (");
    let tag = one(json!({ "field": "tags", "operator": "ne", "value": ["tag-1"] }));
    has(&tag, "NOT EXISTS (SELECT 1 FROM \"ContactToTag\"");
    has(&tag, "\"ContactToTag\".\"tagId\" IN (");
    lacks(&tag, "\"ContactToTag\".\"tagId\" NOT IN");
}

// ts:536 adapted
#[test]
fn apply_contact_filter_renders_current_channel_is_empty_as_not_exists_contact_inbox() {
    let q = one(json!({ "field": "currentChannel", "operator": "isEmpty" }));
    has(&q, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
}

// ts:549 adapted
#[test]
fn apply_contact_filter_renders_source_filters_as_an_exists_contact_inbox_source_subquery() {
    let q = one(json!({ "field": "source", "operator": "in", "value": ["direct"] }));
    has(&q, "\"ContactInbox\".\"source\" IN (");
}

// ts:566 adapted
#[test]
fn apply_contact_filter_renders_interacted_in_last24h_as_an_exists_contact_inbox_recency_subquery()
{
    let positive =
        one(json!({ "field": "interactedInLast24h", "operator": "eq", "value": "true" }));
    has(&positive, "\"ContactInbox\".\"lastIncomingMessageAt\" >= ?");
    has_param(&positive, int(NOW - 86_400_000));
    let negated =
        one(json!({ "field": "interactedInLast24h", "operator": "eq", "value": "false" }));
    has(&negated, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
}

// ts:598 adapted
#[test]
fn apply_contact_filter_renders_tags_is_empty_as_not_exists_contact_to_tag() {
    let q = one(json!({ "field": "tags", "operator": "isEmpty" }));
    has(&q, "NOT EXISTS (SELECT 1 FROM \"ContactToTag\"");
}

// ts:609 adapted
#[test]
fn apply_contact_filter_renders_conversation_based_conditions_as_exists_conversation_subqueries() {
    let archived = one(json!({ "field": "archived", "operator": "eq", "value": "true" }));
    has(&archived, "EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&archived, "\"Conversation\".\"archivedAt\" IS NOT NULL");
    let follow_up = one(json!({ "field": "followUp", "operator": "eq", "value": "true" }));
    has(&follow_up, "\"Conversation\".\"followed\" =");
    let transferred = one(json!({
        "field": "conversationTransferredToHuman",
        "operator": "eq",
        "value": "true",
    }));
    has(&transferred, "\"Conversation\".\"botEnabled\" =");
    has_param(&transferred, int(NOW));
}

// ts:646 ported
#[test]
fn apply_contact_filter_parses_conversation_assignee_sentinel_and_prefixed_ids() {
    assert_eq!(
        parse_conversation_assignee_values(&["unassigned", "u_123", "t_456", "u_not-a-number"]),
        ConversationAssigneeSelection {
            has_unassigned: true,
            user_ids: vec!["123".to_owned()],
            inbox_team_ids: vec!["456".to_owned()],
        }
    );
}

// ts:661 adapted
#[test]
fn apply_contact_filter_renders_conversation_assigned_with_user_team_and_unassigned_branches() {
    let q = one(json!({
        "field": "conversationAssigned",
        "operator": "in",
        "value": ["u_123", "t_456", "unassigned"],
    }));
    has(&q, "EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&q, "\"Conversation\".\"assignedUserId\" IN (");
    has(&q, "\"Conversation\".\"assignedInboxTeamId\" IN (");
    has(&q, "\"Conversation\".\"assignedUserId\" IS NULL");
    has(&q, "\"Conversation\".\"assignedInboxTeamId\" IS NULL");
    assert_eq!(q.params, vec![int(123), int(456)]);
}

// ts:683 adapted
#[test]
fn apply_contact_filter_renders_conversation_assigned_negative_and_empty_branches_as_not_exists() {
    let negative =
        one(json!({ "field": "conversationAssigned", "operator": "notIn", "value": ["u_123"] }));
    has(&negative, "NOT EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&negative, "\"Conversation\".\"assignedUserId\" IN (");
    let empty = one(json!({ "field": "conversationAssigned", "operator": "isEmpty" }));
    has(&empty, "NOT EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&empty, "\"Conversation\".\"assignedUserId\" IS NOT NULL");
    has(
        &empty,
        "\"Conversation\".\"assignedInboxTeamId\" IS NOT NULL",
    );
}

// ts:717 adapted
#[test]
fn apply_contact_filter_renders_unread_and_unreplied_as_boolean_exists_predicates() {
    let unread = one(json!({ "field": "unread", "operator": "eq", "value": "true" }));
    has(&unread, "EXISTS (SELECT 1 FROM \"Conversation\"");
    has(&unread, "\"Conversation\".\"lastActivityAt\" IS NOT NULL");
    has(&unread, "\"Conversation\".\"agentLastReadAt\" IS NULL");
    has(
        &unread,
        "\"Conversation\".\"lastActivityAt\" > \"Conversation\".\"agentLastReadAt\"",
    );
    let unreplied = one(json!({ "field": "unreplied", "operator": "eq", "value": "true" }));
    has(&unreplied, "EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(
        &unreplied,
        "\"ContactInbox\".\"lastIncomingMessageAt\" IS NOT NULL",
    );
    has(
        &unreplied,
        "\"ContactInbox\".\"lastOutboundMessageAt\" IS NULL",
    );
    has(
        &unreplied,
        "\"ContactInbox\".\"lastIncomingMessageAt\" > \"ContactInbox\".\"lastOutboundMessageAt\"",
    );
}

// ts:757 adapted
#[test]
fn apply_contact_filter_renders_existing_contact_as_email_or_phone_boolean_predicate() {
    let positive = one(json!({ "field": "existingContact", "operator": "eq", "value": "true" }));
    has(&positive, "\"Contact\".\"email\" IS NOT NULL");
    has(&positive, "\"Contact\".\"phoneNumber\" IS NOT NULL");
    lacks(&positive, "NOT (");
    let negative = one(json!({ "field": "existingContact", "operator": "eq", "value": "false" }));
    has(&negative, "NOT (");
    has(&negative, "\"Contact\".\"email\" IS NOT NULL");
    has(&negative, "\"Contact\".\"phoneNumber\" IS NOT NULL");
}

// ts:791 adapted
#[test]
fn apply_contact_filter_renders_has_contact_info_presence_predicate_for_the_selected_info_types() {
    let phone_only =
        one(json!({ "field": "hasContactInfo", "operator": "in", "value": ["phone"] }));
    has(&phone_only, "\"Contact\".\"phoneNumber\" IS NOT NULL");
    lacks(&phone_only, "\"Contact\".\"email\"");
    let both =
        one(json!({ "field": "hasContactInfo", "operator": "in", "value": ["phone", "email"] }));
    has(&both, "\"Contact\".\"phoneNumber\" IS NOT NULL");
    has(&both, "\"Contact\".\"email\" IS NOT NULL");
    has(&both, " OR ");
}

// ts:824 adapted
#[test]
fn apply_contact_filter_renders_has_contact_info_phone_and_email_as_a_phone_and_email_presence_predicate()
 {
    let has_both =
        one(json!({ "field": "hasContactInfo", "operator": "in", "value": ["phoneAndEmail"] }));
    has(&has_both, "\"Contact\".\"phoneNumber\" IS NOT NULL");
    has(&has_both, "\"Contact\".\"email\" IS NOT NULL");
    has(&has_both, " AND ");
    lacks(&has_both, " OR ");
    let lacks_both = one(json!({
        "field": "hasContactInfo",
        "operator": "notIn",
        "value": ["phoneAndEmail"],
    }));
    has(&lacks_both, "NOT (");
    has(&lacks_both, "\"Contact\".\"phoneNumber\" IS NOT NULL");
    has(&lacks_both, "\"Contact\".\"email\" IS NOT NULL");
    has(&lacks_both, " AND ");
}

// ts:860 adapted
#[test]
fn apply_contact_filter_renders_null_safe_negation_for_has_contact_info_not_in_and_is_empty() {
    let lacks_email =
        one(json!({ "field": "hasContactInfo", "operator": "notIn", "value": ["email"] }));
    has(&lacks_email, "NOT (");
    has(&lacks_email, "\"Contact\".\"email\" IS NOT NULL");
    lacks(&lacks_email, "\"Contact\".\"phoneNumber\"");
    let lacks_both = one(json!({ "field": "hasContactInfo", "operator": "isEmpty" }));
    has(&lacks_both, "NOT (");
    has(&lacks_both, "\"Contact\".\"email\" IS NOT NULL");
    has(&lacks_both, "\"Contact\".\"phoneNumber\" IS NOT NULL");
}

// ts:893 ported
#[test]
fn apply_contact_filter_ignores_has_contact_info_conditions_with_unknown_info_types() {
    assert!(
        apply(and1(
            json!({ "field": "hasContactInfo", "operator": "in", "value": ["whatsapp"] })
        ))
        .is_none()
    );
}

// ts:908 adapted
#[test]
fn apply_contact_filter_renders_continent_country_code_expansion_and_unknown_sentinel() {
    let q = one(json!({ "field": "continent", "operator": "eq", "value": ["AS", "unknown"] }));
    has(&q, "\"Contact\".\"country\" IN (");
    has(&q, "\"Contact\".\"country\" IS NULL");
    has(&q, "\"Contact\".\"country\" =");
    has(&q, "NOT (\"Contact\".\"country\" IN (");
    has_param(&q, text("VN"));
}

// ts:929 adapted
#[test]
fn apply_contact_filter_renders_last_comment_text_filters_through_contact_inbox_and_message() {
    let contains =
        one(json!({ "field": "lastComment", "operator": "contains", "value": "Need_%help\\" }));
    let equal =
        one(json!({ "field": "lastComment", "operator": "eq", "value": "Exact_%comment\\" }));
    has(&contains, "EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(&contains, "FROM \"Message\"");
    has(&contains, "\"ContactInbox\".\"lastCommentMessageId\"");
    has(&contains, "CASE");
    has(&contains, "^[0-9]+$");
    has(&contains, "AS INTEGER)");
    has(&contains, "ilike(\"Message\".\"text\", ?)");
    has_param(&contains, text("%Need\\_\\%help\\\\%"));
    has_param(&equal, text("Exact\\_\\%comment\\\\"));
}

// ts:966 adapted
#[test]
fn apply_contact_filter_renders_last_comment_negative_and_empty_operators_with_expected_exists_semantics()
 {
    let not_contains =
        one(json!({ "field": "lastComment", "operator": "notContains", "value": "spam" }));
    let not_equal = one(json!({ "field": "lastComment", "operator": "ne", "value": "spam" }));
    let empty = one(json!({ "field": "lastComment", "operator": "isEmpty" }));
    let not_empty = one(json!({ "field": "lastComment", "operator": "isNotEmpty" }));
    has(&not_contains, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(&not_contains, "ilike(\"Message\".\"text\", ?)");
    has(&not_equal, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(&not_equal, "ilike(\"Message\".\"text\", ?)");
    has(&empty, "NOT EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(
        &empty,
        "\"ContactInbox\".\"lastCommentMessageId\" IS NOT NULL",
    );
    has(&not_empty, "EXISTS (SELECT 1 FROM \"ContactInbox\"");
    lacks(&not_empty, "NOT EXISTS");
}

// ts:1023 ported
#[test]
fn apply_contact_filter_ignores_last_comment_value_operators_with_empty_values() {
    assert!(
        apply(and1(
            json!({ "field": "lastComment", "operator": "contains", "value": "" })
        ))
        .is_none()
    );
}

// ts:1038 adapted
#[test]
fn apply_contact_filter_renders_last_sent_as_latest_outbound_contact_inbox_datetime() {
    let q = one(json!({ "field": "lastSent", "operator": "eq", "value": "2026-05-19T10:00:00Z" }));
    has(&q, "MAX(\"ContactInbox\".\"lastOutboundMessageAt\")");
    lacks(&q, "date_trunc");
    has_param(&q, at("2026-05-19T10:00:00.000Z"));
    has_param(&q, at("2026-05-19T10:00:01.000Z"));
}

// ts:1063 adapted
#[test]
fn apply_contact_filter_maps_minutes_ago_comparison_directions_to_timestamp_boundaries() {
    let at_least =
        one(json!({ "field": "contactCreatedDateMinutesAgo", "operator": "gte", "value": "30" }));
    has(&at_least, "\"Contact\".\"createdAt\" <= ?");
    assert_eq!(at_least.params, vec![int(NOW - 30 * MIN)]);
    let less_than = one(json!({ "field": "lastSeenMinutesAgo", "operator": "lt", "value": "15" }));
    has(&less_than, "MAX(\"ContactInbox\".\"contactLastReadAt\")");
    has(&less_than, "\"latestInteraction\".\"latest\" > ?");
    lacks(&less_than, "\"Contact\".\"lastReadAt\"");
    assert_eq!(less_than.params, vec![int(NOW - 15 * MIN)]);
}

// ts:1101 adapted
#[test]
fn apply_contact_filter_renders_minutes_ago_between_with_inverted_newer_and_older_bounds() {
    let q = one(json!({
        "field": "contactCreatedDateMinutesAgo",
        "operator": "isBetween",
        "value": ["5", "30"],
    }));
    has(&q, "\"Contact\".\"createdAt\" >= ?");
    has(&q, "\"Contact\".\"createdAt\" <= ?");
    assert_eq!(q.params, vec![int(NOW - 30 * MIN), int(NOW - 5 * MIN)]);
}

// ts:1124 adapted
#[test]
fn apply_contact_filter_renders_latest_interaction_minutes_ago_against_max_incoming_message() {
    let q = one(json!({ "field": "lastInteractionMinutesAgo", "operator": "gt", "value": "60" }));
    has(&q, "MAX(\"ContactInbox\".\"lastIncomingMessageAt\")");
    has(&q, "\"latestInteraction\".\"latest\" < ?");
    assert_eq!(q.params, vec![int(NOW - 60 * MIN)]);
}

// ts:1145 adapted
#[test]
fn apply_contact_filter_renders_consecutive_ai_failures_as_latest_numeric_aggregate() {
    let q = one(json!({ "field": "consecutiveAiFailures", "operator": "gt", "value": "2" }));
    has(&q, "MAX(\"ContactInbox\".\"consecutiveFailedReply\")");
    has(&q, "\"latestInteraction\".\"latest\" >");
    assert_eq!(q.params, vec![int(2)]);
}

// ts:1164 adapted
#[test]
fn apply_contact_filter_maps_boolean_field_operators_to_boolean_timestamp_predicates() {
    same(
        &first_and("emailWasVerified", "eq", Some(json!("true"))),
        "(\"Contact\".\"emailVerified\" = ?)",
        vec![int(1)],
    );
    same(
        &first_and("subscribedToBroadcast", "eq", Some(json!("false"))),
        "(\"Contact\".\"broadcastSubscribedAt\" IS NULL)",
        vec![],
    );
}

// ts:1196 adapted
#[test]
fn apply_contact_filter_renders_static_datetime_equality_at_the_typed_precision() {
    let q = one(
        json!({ "field": "contactCreatedAt", "operator": "eq", "value": "2026-05-19T10:00:00Z" }),
    );
    has(&q, "\"Contact\".\"createdAt\" >=");
    has(&q, "\"Contact\".\"createdAt\" <");
    lacks(&q, "date_trunc");
    assert_eq!(
        q.params,
        vec![
            at("2026-05-19T10:00:00.000Z"),
            at("2026-05-19T10:00:01.000Z")
        ]
    );
}

// ts:1221 adapted
#[test]
fn apply_contact_filter_renders_a_static_date_only_equality_as_a_full_day_range() {
    let q = one(json!({ "field": "contactCreatedAt", "operator": "eq", "value": "2026-05-19" }));
    assert_eq!(
        q.params,
        vec![
            at("2026-05-19T00:00:00.000Z"),
            at("2026-05-20T00:00:00.000Z")
        ]
    );
}

// ts:1242 adapted
#[test]
fn apply_contact_filter_renders_static_date_intervals_with_supported_sql() {
    let q = one(json!({
        "field": "lastSeen",
        "operator": "isBetween",
        "value": ["2026-05-01T00:00:00Z", "2026-05-31T23:59:59Z"],
    }));
    has(&q, "MAX(\"ContactInbox\".\"contactLastReadAt\")");
    lacks(&q, "\"Contact\".\"lastReadAt\"");
    lacks(&q, "<=");
    assert_eq!(
        q.params,
        vec![
            at("2026-05-01T00:00:00.000Z"),
            at("2026-06-01T00:00:00.000Z")
        ]
    );
}

// ts:1270 adapted
#[test]
fn apply_contact_filter_extends_an_is_between_to_bound_to_the_end_of_the_typed_precision_unit() {
    let q = render(apply(json!({
        "operator": "and",
        "timezone": "Asia/Ho_Chi_Minh",
        "conditions": [{
            "field": "lastSeen",
            "operator": "isBetween",
            "value": ["2026-07-01 09:00", "2026-07-01 09:00"],
        }],
    })));
    assert_eq!(
        q.params,
        vec![
            at("2026-07-01T02:00:00.000Z"),
            at("2026-07-01T02:01:00.000Z")
        ]
    );
}

// ts:1293 adapted
#[test]
fn apply_contact_filter_renders_static_date_not_between_with_supported_sql() {
    let q = one(json!({
        "field": "lastSeen",
        "operator": "notBetween",
        "value": ["2026-05-01T00:00:00Z", "2026-05-31T23:59:59Z"],
    }));
    has(&q, "MAX(\"ContactInbox\".\"contactLastReadAt\")");
    lacks(&q, "\"Contact\".\"lastReadAt\"");
    has(&q, "IS NULL");
    assert_eq!(
        q.params,
        vec![
            at("2026-05-01T00:00:00.000Z"),
            at("2026-06-01T00:00:00.000Z")
        ]
    );
}

// ts:1315 ported
#[test]
fn apply_contact_filter_ignores_static_date_intervals_with_invalid_values() {
    assert!(
        apply(and1(json!({
            "field": "lastSeen",
            "operator": "isBetween",
            "value": ["not-a-date", "2026-05-31T23:59:59Z"],
        })))
        .is_none()
    );
}

fn in_zone(timezone: &str, c: serde_json::Value) -> chatbotx_db::repo::sql::Sql {
    render(apply(
        json!({ "operator": "and", "timezone": timezone, "conditions": [c] }),
    ))
}

// ts:1330 adapted
#[test]
fn apply_contact_filter_interprets_a_naive_date_equality_in_the_supplied_timezone_utc_7() {
    let q = in_zone(
        "Asia/Ho_Chi_Minh",
        json!({ "field": "contactCreatedAt", "operator": "eq", "value": "2026-07-20" }),
    );
    lacks(&q, "date_trunc");
    assert_eq!(
        q.params,
        vec![
            at("2026-07-19T17:00:00.000Z"),
            at("2026-07-20T17:00:00.000Z")
        ]
    );
}

// ts:1353 adapted
#[test]
fn apply_contact_filter_interprets_a_naive_datetime_gt_in_the_supplied_timezone_utc_7() {
    let q = in_zone(
        "Asia/Ho_Chi_Minh",
        json!({ "field": "lastSeen", "operator": "gt", "value": "2026-07-20 14:30" }),
    );
    has(&q, "MAX(\"ContactInbox\".\"contactLastReadAt\")");
    lacks(&q, "\"Contact\".\"lastReadAt\"");
    assert_eq!(q.params, vec![at("2026-07-20T07:31:00.000Z")]);
}

// ts:1375 adapted
#[test]
fn apply_contact_filter_interprets_a_naive_datetime_range_in_the_supplied_timezone_utc_7() {
    let q = in_zone(
        "Asia/Ho_Chi_Minh",
        json!({
            "field": "lastInteraction",
            "operator": "isBetween",
            "value": ["2026-07-01 00:00", "2026-07-31 23:59"],
        }),
    );
    has(&q, "MAX(\"ContactInbox\".\"lastIncomingMessageAt\")");
    has_param(&q, at("2026-06-30T17:00:00.000Z"));
    has_param(&q, at("2026-07-31T17:00:00.000Z"));
}

// ts:1401 adapted
#[test]
fn apply_contact_filter_defaults_date_filters_to_utc_when_no_timezone_is_supplied() {
    let q = one(json!({ "field": "contactCreatedAt", "operator": "eq", "value": "2026-07-20" }));
    assert_eq!(
        q.params,
        vec![
            at("2026-07-20T00:00:00.000Z"),
            at("2026-07-21T00:00:00.000Z")
        ]
    );
}

// ts:1421 adapted
#[test]
fn apply_contact_filter_uses_an_explicit_offset_in_the_value_over_the_supplied_timezone() {
    let q = in_zone(
        "Asia/Ho_Chi_Minh",
        json!({ "field": "lastSeen", "operator": "gt", "value": "2026-07-20T14:30:00+00:00" }),
    );
    assert_eq!(q.params, vec![at("2026-07-20T14:30:01.000Z")]);
}

// ts:1441 adapted
#[test]
fn apply_contact_filter_falls_back_to_utc_for_an_unrecognized_timezone() {
    let q = in_zone(
        "Not/AZone",
        json!({ "field": "contactCreatedAt", "operator": "eq", "value": "2026-07-20" }),
    );
    assert_eq!(
        q.params,
        vec![
            at("2026-07-20T00:00:00.000Z"),
            at("2026-07-21T00:00:00.000Z")
        ]
    );
}

// ts:1462 adapted
#[test]
fn apply_contact_filter_applies_the_criteria_timezone_to_custom_field_dates() {
    let q = in_zone(
        "Asia/Ho_Chi_Minh",
        json!({
            "field": "customField",
            "customFieldId": "cf-1",
            "valueType": "datetime",
            "operator": "eq",
            "value": "2026-07-20",
        }),
    );
    lacks(&q, "date_trunc");
    has_param(&q, at("2026-07-19T17:00:00.000Z"));
    has_param(&q, at("2026-07-20T17:00:00.000Z"));
}

// ts:1485 adapted
#[test]
fn apply_contact_filter_compares_a_custom_field_date_with_time_by_wall_clock_ignoring_the_criteria_timezone()
 {
    let q = in_zone(
        "Asia/Ho_Chi_Minh",
        json!({
            "field": "customField",
            "customFieldId": "cf-1",
            "valueType": "datetime",
            "customFieldType": "date",
            "operator": "eq",
            "value": "2026-07-20 09:30",
        }),
    );
    has_param(&q, text("2026-07-20T09:30:00"));
    has_param(&q, text("2026-07-20T09:31:00"));
    lacks_param(&q, at("2026-07-20T02:30:00.000Z"));
}

fn guarded_datetime_eq(timezone: &str, value: &str, start: &str, end: &str) {
    let q = in_zone(
        timezone,
        json!({
            "field": "customField",
            "customFieldId": "cf-1",
            "valueType": "datetime",
            "operator": "eq",
            "value": value,
        }),
    );
    has(&q, "CASE WHEN");
    lacks(&q, "date_trunc");
    has_param(&q, at(start));
    has_param(&q, at(end));
}

// ts:1514 adapted
#[test]
fn apply_contact_filter_guards_a_datetime_custom_field_cast_and_matches_equality_by_the_typed_precision_window_day_precision_when_only_a_date_is_typed()
 {
    guarded_datetime_eq(
        "UTC",
        "2026-05-19",
        "2026-05-19T00:00:00.000Z",
        "2026-05-20T00:00:00.000Z",
    );
}
// ts:1514 adapted
#[test]
fn apply_contact_filter_guards_a_datetime_custom_field_cast_and_matches_equality_by_the_typed_precision_window_minute_precision_when_a_time_is_typed()
 {
    guarded_datetime_eq(
        "UTC",
        "2026-05-19 10:00",
        "2026-05-19T10:00:00.000Z",
        "2026-05-19T10:01:00.000Z",
    );
}
// ts:1514 adapted
#[test]
fn apply_contact_filter_guards_a_datetime_custom_field_cast_and_matches_equality_by_the_typed_precision_window_second_precision_when_seconds_are_typed()
 {
    guarded_datetime_eq(
        "UTC",
        "2026-05-19T10:00:30Z",
        "2026-05-19T10:00:30.000Z",
        "2026-05-19T10:00:31.000Z",
    );
}
// ts:1514 adapted
#[test]
fn apply_contact_filter_guards_a_datetime_custom_field_cast_and_matches_equality_by_the_typed_precision_window_minute_precision_anchored_to_the_criteria_timezone()
 {
    guarded_datetime_eq(
        "Asia/Ho_Chi_Minh",
        "2026-05-19 10:00",
        "2026-05-19T03:00:00.000Z",
        "2026-05-19T03:01:00.000Z",
    );
}

// ts:1574 adapted
#[test]
fn apply_contact_filter_renders_numeric_custom_field_ranges_with_numeric_guard() {
    let q = one(json!({
        "field": "customField",
        "customFieldId": "cf-1",
        "valueType": "number",
        "operator": "isBetween",
        "value": ["10", "20"],
    }));
    has(&q, "EXISTS");
    has(&q, "AS REAL)");
    has(&q, "regexp(");
    has(&q, ">=");
    has(&q, "<=");
    assert_eq!(q.params, vec![text("cf-1"), real(10.0), real(20.0)]);
}

// ts:1598 adapted
#[test]
fn apply_contact_filter_renders_datetime_custom_field_ranges_with_guarded_timestamptz_casts() {
    let q = one(json!({
        "field": "customField",
        "customFieldId": "cf-1",
        "valueType": "datetime",
        "operator": "isBetween",
        "value": ["2026-05-01T00:00:00Z", "2026-05-31T23:59:59Z"],
    }));
    has(&q, "CASE WHEN");
    has(&q, ">=");
    has(&q, "<");
    has_param(&q, at("2026-05-01T00:00:00.000Z"));
    has_param(&q, at("2026-06-01T00:00:00.000Z"));
}

// ts:1620 ported
#[test]
fn apply_contact_filter_ignores_datetime_custom_field_conditions_with_invalid_input() {
    assert!(
        apply(and1(json!({
            "field": "customField",
            "customFieldId": "cf-1",
            "valueType": "datetime",
            "operator": "eq",
            "value": "not-a-date",
        })))
        .is_none()
    );
}

// ts:1637 ported
#[test]
fn apply_contact_filter_ignores_unsupported_custom_field_operator_type_combinations() {
    assert!(
        apply(and1(json!({
            "field": "customField",
            "customFieldId": "cf-1",
            "valueType": "datetime",
            "operator": "contains",
            "value": "2026",
        })))
        .is_none()
    );
}

// ts:1654 adapted
#[test]
fn apply_contact_filter_ands_keyword_search_with_or_contact_filter_without_overwriting_either_or() {
    let q = build_contact_where(&ContactWhereInput {
        workspace_id: 1,
        keyword: Some("Acme".to_owned()),
        contact_filter: Some(crit(json!({
            "operator": "or",
            "conditions": [{ "field": "fullName", "operator": "contains", "value": "bob" }],
        }))),
        now: NOW,
    });
    prepares(&q);
    has(&q, "\"Contact\".\"workspaceId\" = ?");
    has(&q, "ilike(\"Contact\".\"firstName\", ?)");
    has(&q, "ilike(\"Contact\".\"lastName\", ?)");
    has(&q, "EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(&q, "\"ContactInbox\".\"sourceId\" = ?");
    has(&q, "ilike(\"Contact\".\"fullName\", ?)");
    lacks(&q, "ilike(\"Contact\".\"email\"");
    lacks(&q, "ilike(\"Contact\".\"phoneNumber\"");
    assert_eq!(
        q.params,
        vec![
            int(1),
            text("%acme%"),
            text("%acme%"),
            text("Acme"),
            text("%bob%")
        ]
    );
}

// ts:1685 adapted
#[test]
fn apply_contact_filter_builds_contact_inbox_audience_sql_from_a_contact_rooted_filter() {
    let q = build_contact_inbox_contact_filter_sql(
        "\"ContactInbox\".\"contactId\"",
        1,
        &crit(and1(
            json!({ "field": "fullName", "operator": "contains", "value": "Ada" }),
        )),
        NOW,
    );
    has(&q, "\"ContactInbox\".\"contactId\" IN (");
    has(&q, "SELECT \"Contact\".\"id\" FROM \"Contact\"");
    has(&q, "\"Contact\".\"workspaceId\" = ?");
    has(&q, "ilike(\"Contact\".\"fullName\", ?)");
    assert_eq!(q.params, vec![int(1), text("%Ada%")]);
}

// ts:1710 ported
#[test]
fn apply_contact_filter_preserves_empty_audience_filters_as_true() {
    let q = build_contact_inbox_contact_filter_sql(
        "\"ContactInbox\".\"contactId\"",
        1,
        &crit(json!({ "operator": "and", "conditions": [] })),
        NOW,
    );
    assert_eq!(q.text, "TRUE");
    assert!(q.params.is_empty());
}

// ts:1726 ported
#[test]
fn apply_contact_filter_turns_all_unknown_audience_filters_into_false() {
    let q = build_contact_inbox_contact_filter_sql(
        "\"ContactInbox\".\"contactId\"",
        1,
        &crit(and1(
            json!({ "field": "deletedCustomField", "operator": "eq", "value": "x" }),
        )),
        NOW,
    );
    assert_eq!(q.text, "FALSE");
    assert!(q.params.is_empty());
}

// ---------------------------------------------------------------------------
// applyContactFilter — direct column fields (TS `firstAnd` object shapes -> exact SQLite
// text and binds of the one-condition AND)
// ---------------------------------------------------------------------------

macro_rules! rows {
    ($( $name:ident => $body:expr; )*) => { $( #[test] fn $name() { $body; } )* };
}

fn col_eq(field: &str, column: &str) {
    same(
        &first_and(field, "eq", Some(json!("x"))),
        &format!("(\"Contact\".\"{column}\" = ?)"),
        vec![text("x")],
    );
}

fn col_ci_eq(field: &str, column: &str) {
    let q = one(json!({ "field": field, "operator": "eq", "value": "x" }));
    has(&q, &format!("ilike(\"Contact\".\"{column}\", ?)"));
    has_param(&q, text("x"));
}

fn full_name(
    operator: &str,
    value: serde_json::Value,
    sql: &str,
    params: Vec<chatbotx_db::rusqlite::types::Value>,
) {
    same(&first_and("fullName", operator, Some(value)), sql, params);
}

fn negation_nulls(field: &str, column: &str) {
    same(
        &first_and(field, "ne", Some(json!("x"))),
        &format!("((\"Contact\".\"{column}\" <> ?) OR (\"Contact\".\"{column}\" IS NULL))"),
        vec![text("x")],
    );
    same(
        &first_and(field, "notContains", Some(json!("x"))),
        &format!(
            "((NOT ilike(\"Contact\".\"{column}\", ?)) OR (\"Contact\".\"{column}\" IS NULL))"
        ),
        vec![text("%x%")],
    );
}

fn ci_ne_nulls(field: &str, column: &str) {
    let q = one(json!({ "field": field, "operator": "ne", "value": "x" }));
    has(&q, &format!("NOT ilike(\"Contact\".\"{column}\", ?)"));
    has(&q, &format!("\"Contact\".\"{column}\" IS NULL"));
}

fn not_contains_nulls(field: &str, column: &str) {
    same(
        &first_and(field, "notContains", Some(json!("x"))),
        &format!(
            "((NOT ilike(\"Contact\".\"{column}\", ?)) OR (\"Contact\".\"{column}\" IS NULL))"
        ),
        vec![text("%x%")],
    );
}

fn bool_column(field: &str, column: &str) {
    same(
        &first_and(field, "eq", Some(json!("true"))),
        &format!("(\"Contact\".\"{column}\" = ?)"),
        vec![int(1)],
    );
    same(
        &first_and(field, "eq", Some(json!("false"))),
        &format!("(\"Contact\".\"{column}\" = ?)"),
        vec![int(0)],
    );
    same(
        &first_and(field, "isEmpty", None),
        &format!("(\"Contact\".\"{column}\" IS NULL)"),
        vec![],
    );
}

fn bool_from_timestamp(field: &str, column: &str) {
    let not_null = format!("(\"Contact\".\"{column}\" IS NOT NULL)");
    let null = format!("(\"Contact\".\"{column}\" IS NULL)");
    same(
        &first_and(field, "eq", Some(json!("true"))),
        &not_null,
        vec![],
    );
    same(&first_and(field, "eq", Some(json!("false"))), &null, vec![]);
    same(&first_and(field, "isEmpty", None), &null, vec![]);
    same(&first_and(field, "isNotEmpty", None), &not_null, vec![]);
}

fn timestamptz_comparison(operator: &str) {
    let q = one(
        json!({ "field": "contactCreatedAt", "operator": operator, "value": "2026-05-19T10:00:00Z" }),
    );
    has(&q, "\"Contact\".\"createdAt\"");
    assert_eq!(q.params.len(), 1);
}

rows! {
    // ts:1766 adapted
    apply_contact_filter_direct_column_fields_maps_gender_eq_to_the_gender_column => col_eq("gender", "gender");
    // ts:1766 adapted
    apply_contact_filter_direct_column_fields_maps_country_eq_to_the_country_column => col_eq("country", "country");
    // ts:1766 adapted
    apply_contact_filter_direct_column_fields_maps_locale_eq_to_the_locale_column => col_eq("locale", "locale");
    // ts:1766 adapted
    apply_contact_filter_direct_column_fields_maps_timezone_eq_to_the_timezone_column => col_eq("timezone", "timezone");
    // ts:1777 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_eq_to_case_insensitive_full_name_sql => col_ci_eq("fullName", "fullName");
    // ts:1777 adapted
    apply_contact_filter_direct_column_fields_maps_email_eq_to_case_insensitive_email_sql => col_ci_eq("email", "email");
    // ts:1777 adapted
    apply_contact_filter_direct_column_fields_maps_phone_eq_to_case_insensitive_phone_number_sql => col_ci_eq("phone", "phoneNumber");
    // ts:1795 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_operator_eq => full_name("eq", json!(["a", "b"]), "(\"Contact\".\"fullName\" IN (?, ?))", vec![text("a"), text("b")]);
    // ts:1795 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_operator_ne => full_name("ne", json!(["a"]), "((\"Contact\".\"fullName\" NOT IN (?)) OR (\"Contact\".\"fullName\" IS NULL))", vec![text("a")]);
    // ts:1795 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_operator_in => full_name("in", json!(["a", "b"]), "(\"Contact\".\"fullName\" IN (?, ?))", vec![text("a"), text("b")]);
    // ts:1795 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_operator_not_in => full_name("notIn", json!(["a"]), "((\"Contact\".\"fullName\" NOT IN (?)) OR (\"Contact\".\"fullName\" IS NULL))", vec![text("a")]);
    // ts:1795 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_operator_contains => full_name("contains", json!("ad"), "(ilike(\"Contact\".\"fullName\", ?))", vec![text("%ad%")]);
    // ts:1795 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_operator_not_contains => full_name("notContains", json!("ad"), "((NOT ilike(\"Contact\".\"fullName\", ?)) OR (\"Contact\".\"fullName\" IS NULL))", vec![text("%ad%")]);
    // ts:1795 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_operator_gt => full_name("gt", json!("M"), "(\"Contact\".\"fullName\" > ?)", vec![text("M")]);
    // ts:1795 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_operator_gte => full_name("gte", json!("M"), "(\"Contact\".\"fullName\" >= ?)", vec![text("M")]);
    // ts:1795 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_operator_lt => full_name("lt", json!("M"), "(\"Contact\".\"fullName\" < ?)", vec![text("M")]);
    // ts:1795 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_operator_lte => full_name("lte", json!("M"), "(\"Contact\".\"fullName\" <= ?)", vec![text("M")]);
    // ts:1831 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_operator_eq_as_case_insensitive_sql => {
        let q = one(json!({ "field": "fullName", "operator": "eq", "value": "Ada" }));
        has(&q, "ilike(\"Contact\".\"fullName\", ?)");
        has_param(&q, text("Ada"));
    };
    // ts:1831 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_operator_ne_as_case_insensitive_sql => {
        let q = one(json!({ "field": "fullName", "operator": "ne", "value": "Ada" }));
        has(&q, "NOT ilike(\"Contact\".\"fullName\", ?)");
        has_param(&q, text("Ada"));
    };
    // ts:1846 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_is_empty_to_include_empty_strings => same(&first_and("fullName", "isEmpty", None), "((\"Contact\".\"fullName\" IS NULL) OR (\"Contact\".\"fullName\" = ''))", vec![]);
    // ts:1846 adapted
    apply_contact_filter_direct_column_fields_maps_full_name_is_not_empty_to_include_empty_strings => same(&first_and("fullName", "isNotEmpty", None), "((\"Contact\".\"fullName\" IS NOT NULL) AND (\"Contact\".\"fullName\" <> ''))", vec![]);
    // ts:1861 adapted
    apply_contact_filter_direct_column_fields_includes_null_rows_for_gender_negation_operators => negation_nulls("gender", "gender");
    // ts:1861 adapted
    apply_contact_filter_direct_column_fields_includes_null_rows_for_country_negation_operators => negation_nulls("country", "country");
    // ts:1861 adapted
    apply_contact_filter_direct_column_fields_includes_null_rows_for_locale_negation_operators => negation_nulls("locale", "locale");
    // ts:1861 adapted
    apply_contact_filter_direct_column_fields_includes_null_rows_for_timezone_negation_operators => negation_nulls("timezone", "timezone");
    // ts:1875 adapted
    apply_contact_filter_direct_column_fields_includes_null_rows_for_full_name_case_insensitive_ne => ci_ne_nulls("fullName", "fullName");
    // ts:1875 adapted
    apply_contact_filter_direct_column_fields_includes_null_rows_for_email_case_insensitive_ne => ci_ne_nulls("email", "email");
    // ts:1875 adapted
    apply_contact_filter_direct_column_fields_includes_null_rows_for_phone_case_insensitive_ne => ci_ne_nulls("phone", "phoneNumber");
    // ts:1895 adapted
    apply_contact_filter_direct_column_fields_includes_null_rows_for_full_name_not_contains => not_contains_nulls("fullName", "fullName");
    // ts:1895 adapted
    apply_contact_filter_direct_column_fields_includes_null_rows_for_email_not_contains => not_contains_nulls("email", "email");
    // ts:1895 adapted
    apply_contact_filter_direct_column_fields_includes_null_rows_for_phone_not_contains => not_contains_nulls("phone", "phoneNumber");
    // ts:1905 adapted
    apply_contact_filter_direct_column_fields_does_not_compare_the_gender_enum_to_an_empty_string => {
        same(&first_and("gender", "isEmpty", None), "(\"Contact\".\"gender\" IS NULL)", vec![]);
        same(&first_and("gender", "isNotEmpty", None), "(\"Contact\".\"gender\" IS NOT NULL)", vec![]);
    };
    // ts:1914 adapted
    apply_contact_filter_direct_column_fields_renders_starts_with_ends_with_as_anchored_ilike => {
        let starts = one(json!({ "field": "fullName", "operator": "startsWith", "value": "Ad" }));
        has(&starts, "ilike(\"Contact\".\"fullName\", ?)");
        has_param(&starts, text("Ad%"));
        let ends = one(json!({ "field": "email", "operator": "endsWith", "value": "@acme.com" }));
        has_param(&ends, text("%@acme.com"));
    };
    // ts:1945 ported
    apply_contact_filter_direct_column_fields_drops_is_between_not_between_for_non_date_columns => assert!(first_and("fullName", "isBetween", Some(json!(["a", "b"]))).is_none());
    // ts:1962 adapted
    apply_contact_filter_boolean_columns_maps_email_was_verified_eq_true_false_is_empty_to_email_verified => bool_column("emailWasVerified", "emailVerified");
    // ts:1962 adapted
    apply_contact_filter_boolean_columns_maps_opted_in_for_email_eq_true_false_is_empty_to_email_opt_in => bool_column("optedInForEmail", "emailOptIn");
    // ts:1979 adapted
    apply_contact_filter_boolean_from_timestamp_columns_maps_subscribed_to_broadcast_to_broadcast_subscribed_at_null_checks => bool_from_timestamp("subscribedToBroadcast", "broadcastSubscribedAt");
    // ts:1979 adapted
    apply_contact_filter_boolean_from_timestamp_columns_maps_blocked_to_blocked_at_null_checks => bool_from_timestamp("blocked", "blockedAt");
    // ts:1999 adapted
    apply_contact_filter_date_columns_maps_contact_created_at_is_empty_is_not_empty_to_created_at_null_checks => {
        same(&first_and("contactCreatedAt", "isEmpty", None), "(\"Contact\".\"createdAt\" IS NULL)", vec![]);
        same(&first_and("contactCreatedAt", "isNotEmpty", None), "(\"Contact\".\"createdAt\" IS NOT NULL)", vec![]);
    };
    // ts:2010 adapted
    apply_contact_filter_date_columns_maps_last_seen_is_empty_is_not_empty_to_a_contact_inbox_aggregate_null_check => {
        let empty = one(json!({ "field": "lastSeen", "operator": "isEmpty" }));
        has(&empty, "MAX(\"ContactInbox\".\"contactLastReadAt\")");
        has(&empty, "IS NULL");
        lacks(&empty, "\"Contact\".\"lastReadAt\"");
        let not_empty = one(json!({ "field": "lastSeen", "operator": "isNotEmpty" }));
        has(&not_empty, "MAX(\"ContactInbox\".\"contactLastReadAt\")");
        has(&not_empty, "IS NOT NULL");
        lacks(&not_empty, "\"Contact\".\"lastReadAt\"");
    };
    // ts:2036 adapted
    apply_contact_filter_date_columns_renders_ne_as_an_outside_the_precision_window_range => {
        let q = one(json!({ "field": "contactCreatedAt", "operator": "ne", "value": "2026-05-19T10:00:00Z" }));
        has(&q, "\"Contact\".\"createdAt\" <");
        has(&q, "\"Contact\".\"createdAt\" >=");
        has(&q, "\"Contact\".\"createdAt\" IS NULL");
        lacks(&q, "date_trunc");
        assert_eq!(q.params, vec![at("2026-05-19T10:00:00.000Z"), at("2026-05-19T10:00:01.000Z")]);
    };
    // ts:2061 adapted
    apply_contact_filter_date_columns_renders_gt_as_a_timestamptz_comparison => timestamptz_comparison("gt");
    // ts:2061 adapted
    apply_contact_filter_date_columns_renders_gte_as_a_timestamptz_comparison => timestamptz_comparison("gte");
    // ts:2061 adapted
    apply_contact_filter_date_columns_renders_lt_as_a_timestamptz_comparison => timestamptz_comparison("lt");
    // ts:2061 adapted
    apply_contact_filter_date_columns_renders_lte_as_a_timestamptz_comparison => timestamptz_comparison("lte");
    // ts:2083 ported
    apply_contact_filter_date_columns_drops_invalid_single_date_values => assert!(first_and("lastSeen", "gt", Some(json!("not-a-date"))).is_none());
}
