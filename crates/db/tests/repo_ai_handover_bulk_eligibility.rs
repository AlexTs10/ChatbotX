//! Port of `packages/database/__tests__/ai-handover-bulk-eligibility.test.ts` (P3.2b).
//! The rendered Postgres SQL / params become the SQLite SQL text and binds (adapted;
//! instants bind as Unix ms, not ISO strings); the mocked-Drizzle repository cases run on
//! real rows of a migrated temp file (adapted).

mod common;
#[path = "common/repo12.rs"]
mod repo12;

use chatbotx_db::repo::contact_inbox::{self as ci, BulkAiPageInput};
use chatbotx_db::repo::queries::ai_handover_bulk_eligibility::{
    AiHandoverBulkAction as Action, bulk_eligibility_conditions, bulk_eligibility_where,
};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::rusqlite::types::Value;
use repo12::{
    NOW, REQUESTED_AT, SINCE, input, input_for, put_bulk_thread, put_conversation, put_inbox_team,
    thread,
};

/// `render`: `and(...bulkEligibilityConditions(input))` as SQL text and binds.
fn render(action: Action) -> Sql {
    Sql::and(
        bulk_eligibility_conditions(&input_for(action))
            .into_iter()
            .map(Some),
    )
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

// --- bulkEligibilityConditions

#[test]
fn bulk_eligibility_conditions_disable_targets_only_threads_the_ai_agent_holds_right_now() {
    let q = render(Action::Disable);
    assert!(q.text.contains("\"threadControlState\" = ?"), "{}", q.text);
    assert!(q.text.contains("\"threadOwnerRole\" = ?"), "{}", q.text);
    assert!(q.params.contains(&text("standby")));
    assert!(q.params.contains(&text("ai_agent")));
    // The resolved (expiry / 24h idle) form of standby, not the raw column.
    assert!(
        q.text.contains("\"threadOwnerExpiresAt\" IS NOT NULL"),
        "{}",
        q.text
    );
    // SQLite has no GREATEST; the NULL-ignoring `max` form stands in (adapted).
    assert!(q.text.contains("max("), "{}", q.text);
    assert!(!q.text.contains("NOT ("), "{}", q.text);
}

#[test]
fn bulk_eligibility_conditions_enable_excludes_held_standby_and_is_never_limited_to_the_ai_role() {
    let q = render(Action::Enable);
    assert!(q.text.contains("NOT COALESCE("), "{}", q.text);
    assert!(!q.params.contains(&text("ai_agent")));
}

fn assert_request_cut_off(action: Action) {
    let q = render(action);
    assert!(
        q.text.contains("\"threadControlUpdatedAt\" IS NULL"),
        "{}",
        q.text
    );
    assert!(
        q.text.contains("\"threadControlUpdatedAt\" <= ?"),
        "{}",
        q.text
    );
    // Column comparisons bind Unix ms (the TS: ISO strings).
    assert!(q.params.contains(&Value::Integer(REQUESTED_AT)));
}

#[test]
fn bulk_eligibility_conditions_enable_respects_the_request_cut_off_and_accepts_never_observed_threads()
 {
    assert_request_cut_off(Action::Enable);
}

#[test]
fn bulk_eligibility_conditions_disable_respects_the_request_cut_off_and_accepts_never_observed_threads()
 {
    assert_request_cut_off(Action::Disable);
}

fn assert_address_and_window(action: Action) {
    let q = render(action);
    assert!(q.text.contains("\"sourceId\" IS NOT NULL"), "{}", q.text);
    assert!(
        q.text.contains("\"lastIncomingMessageAt\" >= ?"),
        "{}",
        q.text
    );
    assert!(q.params.contains(&Value::Integer(SINCE)));
}

#[test]
fn bulk_eligibility_conditions_enable_requires_an_address_and_activity_inside_the_window() {
    assert_address_and_window(Action::Enable);
}

#[test]
fn bulk_eligibility_conditions_disable_requires_an_address_and_activity_inside_the_window() {
    assert_address_and_window(Action::Disable);
}

#[test]
fn bulk_eligibility_conditions_the_idle_boundary_is_computed_from_now_not_the_database_clock() {
    let q = render(Action::Disable);
    assert!(
        q.params
            .contains(&Value::Integer(NOW - 24 * 60 * 60 * 1000))
    );
    assert!(!q.text.contains("now_ms()"), "{}", q.text);
}

// --- bulkEligibilityWhere

#[test]
fn bulk_eligibility_where_scopes_to_the_inbox_and_resumes_after_the_cursor() {
    let with_cursor = bulk_eligibility_where(3101, &input(), Some(9009));
    let fresh = bulk_eligibility_where(3101, &input(), None);
    assert!(with_cursor.params.contains(&Value::Integer(3101)));
    assert!(with_cursor.params.contains(&Value::Integer(9009)));
    assert!(
        with_cursor.text.contains("\"ContactInbox\".\"id\" > ?"),
        "{}",
        with_cursor.text
    );
    assert!(
        !fresh.text.contains("\"ContactInbox\".\"id\" > ?"),
        "{}",
        fresh.text
    );
}

// --- contactInboxRepository bulk queries

#[tokio::test]
async fn contact_inbox_repository_bulk_queries_a_page_joins_the_direct_message_conversation_of_the_workspace_and_is_id_ordered()
 {
    let db = common::fresh_db().await;
    let s = common::seed(&db).await;
    let other_ws = 2002;
    common::seed_workspace(&db, 1002, other_ws).await;
    let (ws, inbox) = (s.workspace_id, s.inbox_id);
    put_inbox_team(&db, 61, ws).await;
    // Three eligible threads (disable: held by the AI agent), inserted out of id order.
    let held = repo12::Thread {
        state: Some("standby"),
        role: Some("ai_agent"),
        updated_at: Some(repo12::hours_ago(2)),
        ..thread()
    };
    for (ci_id, contact_id) in [(9003, 8003), (9001, 8001), (9002, 8002), (9004, 8004)] {
        put_bulk_thread(&db, ci_id, contact_id, ws, inbox, held).await;
    }
    // 8001: the DM conversation of the workspace, assigned.
    put_conversation(&db, 7001, ws, 8001, None, Some(61), Some(1001)).await;
    // 8002: only a non-DM conversation (not eligible: no conversation is ever created).
    put_conversation(&db, 7002, ws, 8002, Some("thread-x"), None, None).await;
    // 8003: a DM conversation.
    put_conversation(&db, 7003, ws, 8003, None, None, None).await;
    // 8004: its DM conversation is in another workspace (the workspace scope).
    put_conversation(&db, 7004, other_ws, 8004, None, None, None).await;
    let page_input = BulkAiPageInput {
        eligibility: input(),
        workspace_id: ws,
        inbox_id: inbox,
        after_id: None,
        limit: 50,
    };
    let page = chatbotx_db::repo::read(&db, move |c| ci::list_bulk_ai_page(c, &page_input))
        .await
        .expect("page");
    let got: Vec<(i64, i64, Option<i64>, Option<i64>)> = page
        .iter()
        .map(|r| {
            (
                r.contact_inbox.id,
                r.conversation_id,
                r.assigned_inbox_team_id,
                r.assigned_user_id,
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![(9001, 7001, Some(61), Some(1001)), (9003, 7003, None, None)]
    );
    assert_eq!(page[0].contact_inbox.contact_id, 8001);
    // The limit is applied.
    let one = BulkAiPageInput {
        limit: 1,
        ..page_input
    };
    let page = chatbotx_db::repo::read(&db, move |c| ci::list_bulk_ai_page(c, &one))
        .await
        .expect("page");
    assert_eq!(
        page.iter().map(|r| r.contact_inbox.id).collect::<Vec<_>>(),
        vec![9001]
    );
}

#[tokio::test]
async fn contact_inbox_repository_bulk_queries_the_pre_dispatch_re_check_makes_no_query_for_an_empty_batch()
 {
    // No query is observable on SQLite; the empty batch returns nothing even on a
    // database whose `ContactInbox` table is gone (adapted from "select not called").
    let db = common::fresh_db().await;
    db.write(|tx| Ok(tx.execute_batch(r#"DROP TABLE "ContactInbox""#)?))
        .await
        .expect("drop");
    let ids = chatbotx_db::repo::read(&db, |c| {
        ci::list_still_bulk_ai_eligible(c, &input(), 2001, &[])
    })
    .await
    .expect("re-check");
    assert!(ids.is_empty());
}
