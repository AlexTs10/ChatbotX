//! Port of `packages/database/__tests__/contact-activity-partial.test.ts`, the
//! "contact activity migration CHECK parity" block (P2.3 spec section 5.1, rows 10.1-10.2).
//! The registry block (`:46-88`) belongs to the core/partials unit.
//! Class C: the CHECK text is read from the migrated SQLite schema (`sqlite_schema.sql`),
//! not from migration files.

mod common;

use common::{column_in_list, fresh_db, named_check_text, table_sql};

/// `UNSCOPED_PARENT_REFERENCE_ID` of `partials/contact-activity`.
const UNSCOPED_PARENT_REFERENCE_ID: &str = "0";

/// One entry of `CONTACT_ACTIVITY_ALLOWED_PAIRS`.
struct Pair {
    activity_type: &'static str,
    reference_type: &'static str,
    parent_scoped: bool,
}

/// Local copy of `CONTACT_ACTIVITY_ALLOWED_PAIRS` until the core crate exports the registry
/// (spec 5.1 row 10.1). Replace with `chatbotx_core::...` when it exists.
const CONTACT_ACTIVITY_ALLOWED_PAIRS: &[Pair] = &[
    Pair { activity_type: "flowExecuted", reference_type: "flow", parent_scoped: false },
    Pair { activity_type: "flowNodeExecuted", reference_type: "flowNode", parent_scoped: true },
    Pair { activity_type: "automatedResponseMatched", reference_type: "automatedResponse", parent_scoped: false },
    Pair { activity_type: "formCompleted", reference_type: "whatsappFlow", parent_scoped: false },
    Pair { activity_type: "messageSent", reference_type: "emailTopic", parent_scoped: false },
    Pair { activity_type: "messageDelivered", reference_type: "emailTopic", parent_scoped: false },
    Pair { activity_type: "messageOpened", reference_type: "emailTopic", parent_scoped: false },
    Pair { activity_type: "messageClicked", reference_type: "emailTopic", parent_scoped: false },
    Pair { activity_type: "adClicked", reference_type: "ad", parent_scoped: false },
];

/// Local copy of `contactActivityTypes.options` (declaration order).
const CONTACT_ACTIVITY_TYPES: &[&str] = &[
    "flowExecuted",
    "flowNodeExecuted",
    "automatedResponseMatched",
    "formCompleted",
    "messageSent",
    "messageDelivered",
    "messageOpened",
    "messageClicked",
    "adClicked",
];

/// Local copy of `contactActivityReferenceTypes.options` (declaration order).
const CONTACT_ACTIVITY_REFERENCE_TYPES: &[&str] =
    &["flow", "flowNode", "automatedResponse", "whatsappFlow", "emailTopic", "ad"];

/// Port of: packages/database/__tests__/contact-activity-partial.test.ts:121 "contact activity migration CHECK parity > lists exactly the registry's allowed pairs"
#[tokio::test]
async fn check_lists_exactly_the_registry_allowed_pairs() {
    let db = fresh_db().await;
    let sql = table_sql(&db, "ContactActivity").await.expect("ContactActivity is missing");
    let check = named_check_text(&sql, "ContactActivity_allowed_pair_check")
        .unwrap_or_else(|| panic!("no ContactActivity_allowed_pair_check in: {sql}"));

    // Same count as the regex /\("activityType" = '[^)]+\)/g of the original.
    let clauses = check.matches(r#"("activityType" = '"#).count();
    assert_eq!(clauses, CONTACT_ACTIVITY_ALLOWED_PAIRS.len(), "{check}");
    for pair in CONTACT_ACTIVITY_ALLOWED_PAIRS {
        let operator = if pair.parent_scoped { "<>" } else { "=" };
        let clause = format!(
            r#"("activityType" = '{}' AND "referenceType" = '{}' AND "parentReferenceId" {operator} {UNSCOPED_PARENT_REFERENCE_ID})"#,
            pair.activity_type, pair.reference_type
        );
        assert!(check.contains(&clause), "missing clause {clause} in: {check}");
    }
}

/// Port of: packages/database/__tests__/contact-activity-partial.test.ts:135 "contact activity migration CHECK parity > creates the enums from the registry values"
///
/// pgEnum -> column CHECK `IN (...)`: the lists of `activityType` and `referenceType` contain
/// every registry value.
#[tokio::test]
async fn enum_checks_contain_the_registry_values() {
    let db = fresh_db().await;
    let sql = table_sql(&db, "ContactActivity").await.expect("ContactActivity is missing");
    let activity = column_in_list(&sql, "activityType").unwrap_or_else(|| panic!("no activityType IN list: {sql}"));
    for value in CONTACT_ACTIVITY_TYPES {
        assert!(activity.contains(&format!("'{value}'")), "activityType CHECK lacks '{value}': {activity}");
    }
    let reference =
        column_in_list(&sql, "referenceType").unwrap_or_else(|| panic!("no referenceType IN list: {sql}"));
    for value in CONTACT_ACTIVITY_REFERENCE_TYPES {
        assert!(reference.contains(&format!("'{value}'")), "referenceType CHECK lacks '{value}': {reference}");
    }
}
