//! Port of `packages/database/__tests__/contact-repository-list-where.test.ts` (P3.2b slice
//! D1). The TS mocked `applyContactFilter` / `buildSmartKeywordWhere` and compared the
//! Drizzle object; here the real builders run and the checks are on the SQLite text and
//! binds (adapted). The mocked `{ conversation: .. }` merge becomes a separate conversation
//! EXISTS AND-ed with the filter's own conversation condition. Workspace "1" is 1, user
//! "user-1" is 7, team "team-1" is 8.

mod common;
#[path = "common/repo8.rs"]
mod repo8;

use chatbotx_db::repo::contact_list_where::{
    ListWhereInput, build_contact_list_where, resolve_contact_order_by,
};
use chatbotx_db::repo::queries::contact_filter::ContactInboxScope;
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::utils::{SortDir, SortItem};
use repo8::{NOW, count, crit, has, int, lacks, prepares, text};
use serde_json::json;

fn base() -> ListWhereInput {
    ListWhereInput {
        workspace_id: 1,
        include_email_and_phone: true,
        now: NOW,
        ..ListWhereInput::default()
    }
}

fn build(input: ListWhereInput) -> Sql {
    let q = build_contact_list_where(&input);
    prepares(&q);
    q
}

// ts:65 ported
#[test]
fn resolve_contact_order_by_falls_back_to_created_at_desc_when_sort_is_missing() {
    assert_eq!(
        resolve_contact_order_by(None),
        vec![("createdAt".to_owned(), SortDir::Desc)]
    );
}

// ts:69 ported
#[test]
fn resolve_contact_order_by_falls_back_to_created_at_desc_when_sort_is_an_empty_array() {
    assert_eq!(
        resolve_contact_order_by(Some(&[])),
        vec![("createdAt".to_owned(), SortDir::Desc)]
    );
}

// ts:75 ported
#[test]
fn resolve_contact_order_by_uses_the_explicit_sort_when_a_valid_column_is_provided() {
    let sort = [SortItem {
        id: "fullName".to_owned(),
        desc: false,
    }];
    assert_eq!(
        resolve_contact_order_by(Some(&sort)),
        vec![("fullName".to_owned(), SortDir::Asc)]
    );
}

// ts:85 adapted
#[test]
fn build_contact_list_where_adds_the_assigned_user_conversation_scope() {
    let q = build(ListWhereInput {
        restrict_to_assigned_user_id: Some(7),
        ..base()
    });
    has(
        &q,
        "EXISTS (SELECT 1 FROM \"Conversation\" WHERE \"Conversation\".\"contactId\" = \"Contact\".\"id\"",
    );
    has(&q, "\"Conversation\".\"assignedUserId\" = ?");
    lacks(&q, "assignedInboxTeamId");
    assert_eq!(q.params, vec![int(1), int(7)]);
}

// ts:96 adapted
#[test]
fn build_contact_list_where_drops_email_and_phone_keyword_clauses_when_email_and_phone_is_denied() {
    let q = build(ListWhereInput {
        keyword: Some("Alice".to_owned()),
        include_email_and_phone: false,
        ..base()
    });
    has(&q, "ilike(\"Contact\".\"firstName\", ?)");
    has(&q, "ilike(\"Contact\".\"lastName\", ?)");
    lacks(&q, "\"Contact\".\"email\"");
    lacks(&q, "\"Contact\".\"phoneNumber\"");
}

// ts:109 adapted
#[test]
fn build_contact_list_where_adds_the_assigned_team_conversation_scope() {
    let q = build(ListWhereInput {
        restrict_to_team_ids: vec![8],
        ..base()
    });
    has(&q, "\"Conversation\".\"assignedInboxTeamId\" IN (?)");
    lacks(&q, "\"Conversation\".\"assignedUserId\"");
    assert_eq!(q.params, vec![int(1), int(8)]);
}

// ts:120 adapted
#[test]
fn build_contact_list_where_preserves_existing_conversation_filters_when_adding_an_assigned_user_scope()
 {
    let q = build(ListWhereInput {
        contact_filter: Some(crit(json!({
            "operator": "and",
            "conditions": [{ "field": "archived", "operator": "eq", "value": "true" }],
        }))),
        restrict_to_assigned_user_id: Some(7),
        ..base()
    });
    assert_eq!(count(&q, "EXISTS (SELECT 1 FROM \"Conversation\""), 2);
    has(&q, "\"Conversation\".\"archivedAt\" IS NOT NULL");
    has(&q, "\"Conversation\".\"assignedUserId\" = ?");
}

// ts:133 adapted
#[test]
fn build_contact_list_where_prunes_email_phone_contact_filter_conditions_when_email_and_phone_is_denied()
 {
    let q = build(ListWhereInput {
        include_email_and_phone: false,
        contact_filter: Some(crit(json!({
            "operator": "and",
            "conditions": [
                { "field": "email", "operator": "eq", "value": "ada@example.com" },
                { "field": "fullName", "operator": "contains", "value": "Ada" },
            ],
        }))),
        ..base()
    });
    lacks(&q, "\"Contact\".\"email\"");
    has(&q, "ilike(\"Contact\".\"fullName\", ?)");
    assert_eq!(q.params, vec![int(1), text("%Ada%")]);
}

// ts:152 adapted
#[test]
fn build_contact_list_where_ands_keyword_search_with_or_contact_filter_without_overwriting_keyword_search()
 {
    let q = build(ListWhereInput {
        keyword: Some("Alice".to_owned()),
        contact_filter: Some(crit(json!({
            "operator": "or",
            "conditions": [{ "field": "fullName", "operator": "contains", "value": "Bob" }],
        }))),
        ..base()
    });
    has(&q, "ilike(\"Contact\".\"email\", ?)");
    has(&q, "ilike(\"Contact\".\"fullName\", ?)");
    assert_eq!(
        q.params,
        vec![
            int(1),
            text("%alice%"),
            text("%alice%"),
            text("Alice"),
            text("%Bob%")
        ]
    );
}

// ts:188 adapted
#[test]
fn build_contact_list_where_adds_the_inbox_scope_as_an_extra_filter_when_one_is_given() {
    let q = build(ListWhereInput {
        inbox_scope: Some(ContactInboxScope {
            inbox_ids: Some(vec![10, 11]),
            require_recent_interaction: true,
        }),
        ..base()
    });
    has(&q, "EXISTS (SELECT 1 FROM \"ContactInbox\"");
    has(&q, "\"ContactInbox\".\"inboxId\" IN (?, ?)");
    has(&q, "\"ContactInbox\".\"lastIncomingMessageAt\" >= ?");
    assert_eq!(
        q.params,
        vec![int(1), int(10), int(11), int(NOW - 86_400_000)]
    );
}

// ts:200 adapted
#[test]
fn build_contact_list_where_ands_the_inbox_scope_with_keyword_and_contact_filter() {
    let q = build(ListWhereInput {
        keyword: Some("Alice".to_owned()),
        inbox_scope: Some(ContactInboxScope {
            inbox_ids: Some(vec![10]),
            require_recent_interaction: false,
        }),
        ..base()
    });
    has(&q, "ilike(\"Contact\".\"firstName\", ?)");
    has(&q, "\"ContactInbox\".\"inboxId\" IN (?)");
    lacks(&q, "lastIncomingMessageAt");
}
