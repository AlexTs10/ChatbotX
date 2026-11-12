//! Port of `packages/database/__tests__/whatsapp-call-list-for-workspace.test.ts` (P3.3).
//! The TS test renders the Drizzle WHERE / ORDER BY / LIMIT of a mocked chain; each port
//! asserts the rows that SQL selects from a migrated SQLite file.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo6.rs"]
mod repo6;

use chatbotx_db::repo::whatsapp_call::*;
use chatbotx_db::repo::{self, iso, parse_iso};
use repo_common::{ok, setup};
use repo6::*;

const MIN: i64 = 60_000;
const OTHER_CONV: i64 = 3202;

/// Calls 1 (answered by 1002, T0), 2 (initiated by 1002, T0+1m, unassigned conversation) and
/// 3 (answered by 1003, T0+2m); `CALL_CONV` is assigned to 1002.
async fn scoped_db() -> common::TestDb {
    let (db, s) = call_db().await;
    setup(&db, move |tx| {
        put_user(tx, 1002)?;
        put_user(tx, 1003)?;
        common::insert_conversation(tx, s, OTHER_CONV, Some("thread-2"))?;
        tx.execute(
            "UPDATE \"Conversation\" SET \"assignedUserId\" = 1002 WHERE \"id\" = ?1",
            [CALL_CONV],
        )?;
        put_call(tx, 1, T0, "\"answeredByUserId\" = 1002")?;
        put_call(
            tx,
            2,
            T0 + MIN,
            &format!("\"initiatedByUserId\" = 1002, \"conversationId\" = {OTHER_CONV}"),
        )?;
        put_call(tx, 3, T0 + 2 * MIN, "\"answeredByUserId\" = 1003")
    })
    .await;
    db
}

fn input(scope: WhatsappCallHistoryScope) -> ListForWorkspaceInput {
    ListForWorkspaceInput {
        workspace_id: 2001,
        scope,
        filters: WhatsappCallListFilters::default(),
        cursor: None,
        limit: 25,
    }
}

async fn ids(db: &chatbotx_db::Db, input: ListForWorkspaceInput) -> Vec<i64> {
    ok(repo::read(db, move |c| list_for_workspace(c, &input)).await)
        .iter()
        .map(|r| r.call.id)
        .collect()
}

fn own(user_id: i64, assigned_only: bool) -> WhatsappCallHistoryScope {
    WhatsappCallHistoryScope::Own {
        user_id,
        assigned_only,
    }
}

/// Port of: packages/database/__tests__/whatsapp-call-list-for-workspace.test.ts:77 "whatsappCallRepository.listForWorkspace — where-builder > allCalls scope adds no own-call restriction"
#[tokio::test]
async fn whatsapp_call_repository_list_for_workspace_where_builder_all_calls_scope_adds_no_own_call_restriction()
 {
    let db = scoped_db().await;
    assert_eq!(
        ids(&db, input(WhatsappCallHistoryScope::AllCalls)).await,
        vec![3, 2, 1]
    );
    let other_ws = ListForWorkspaceInput {
        workspace_id: 2002,
        ..input(WhatsappCallHistoryScope::AllCalls)
    };
    assert!(ids(&db, other_ws).await.is_empty());
}

/// Port of: packages/database/__tests__/whatsapp-call-list-for-workspace.test.ts:92 "whatsappCallRepository.listForWorkspace — where-builder > own-call scope (contacts) restricts to answered-or-initiated by the member, no conversation restriction"
#[tokio::test]
async fn whatsapp_call_repository_list_for_workspace_where_builder_own_call_scope_contacts_restricts_to_answered_or_initiated_by_the_member_no_conversation_restriction()
 {
    let db = scoped_db().await;
    assert_eq!(ids(&db, input(own(1002, false))).await, vec![2, 1]);
}

/// Port of: packages/database/__tests__/whatsapp-call-list-for-workspace.test.ts:110 "whatsappCallRepository.listForWorkspace — where-builder > assignedOnly scope (onlyAssignedContacts) ALSO restricts by conversation.assignedUserId"
#[tokio::test]
async fn whatsapp_call_repository_list_for_workspace_where_builder_assigned_only_scope_only_assigned_contacts_also_restricts_by_conversation_assigned_user_id()
 {
    let db = scoped_db().await;
    assert_eq!(ids(&db, input(own(1002, true))).await, vec![1]);
}

/// Port of: packages/database/__tests__/whatsapp-call-list-for-workspace.test.ts:126 "whatsappCallRepository.listForWorkspace — where-builder > agentUserId filter is applied only when scope.allCalls is true"
#[tokio::test]
async fn whatsapp_call_repository_list_for_workspace_where_builder_agent_user_id_filter_is_applied_only_when_scope_all_calls_is_true()
 {
    let db = scoped_db().await;
    let filters = WhatsappCallListFilters {
        agent_user_id: Some(1003),
        ..WhatsappCallListFilters::default()
    };
    let admin = ListForWorkspaceInput {
        filters: filters.clone(),
        ..input(WhatsappCallHistoryScope::AllCalls)
    };
    let member = ListForWorkspaceInput {
        filters,
        ..input(own(1002, false))
    };
    assert_eq!(ids(&db, admin).await, vec![3]);
    assert_eq!(ids(&db, member).await, vec![2, 1]);
}

/// Calls 4 (outcome failed), 5 (legacy: outcome NULL, status failed), 6 (outcome canceled).
async fn outcome_db() -> common::TestDb {
    let (db, _) = call_db().await;
    setup(&db, |tx| {
        put_call(tx, 4, T0, "\"status\" = 'failed', \"outcome\" = 'failed'")?;
        put_call(tx, 5, T0 + MIN, "\"status\" = 'failed'")?;
        put_call(
            tx,
            6,
            T0 + 2 * MIN,
            "\"status\" = 'failed', \"outcome\" = 'canceled'",
        )
    })
    .await;
    db
}

fn by_outcome(outcome: &str) -> ListForWorkspaceInput {
    ListForWorkspaceInput {
        filters: WhatsappCallListFilters {
            outcome: Some(outcome.to_owned()),
            ..WhatsappCallListFilters::default()
        },
        ..input(WhatsappCallHistoryScope::AllCalls)
    }
}

/// Port of: packages/database/__tests__/whatsapp-call-list-for-workspace.test.ts:154 "whatsappCallRepository.listForWorkspace — where-builder > outcome filter matches the persisted outcome OR a legacy null-outcome row via its status fallback"
#[tokio::test]
async fn whatsapp_call_repository_list_for_workspace_where_builder_outcome_filter_matches_the_persisted_outcome_or_a_legacy_null_outcome_row_via_its_status_fallback()
 {
    let db = outcome_db().await;
    assert_eq!(ids(&db, by_outcome("failed")).await, vec![5, 4]);
}

/// Port of: packages/database/__tests__/whatsapp-call-list-for-workspace.test.ts:172 "whatsappCallRepository.listForWorkspace — where-builder > outcome=canceled has NO legacy status fallback branch (no status value means canceled)"
#[tokio::test]
async fn whatsapp_call_repository_list_for_workspace_where_builder_outcome_canceled_has_no_legacy_status_fallback_branch_no_status_value_means_canceled()
 {
    let db = outcome_db().await;
    assert_eq!(ids(&db, by_outcome("canceled")).await, vec![6]);
}

/// Calls 1 (T0), 2 and 3 (both T0 + 1m).
async fn tie_db() -> common::TestDb {
    let (db, _) = call_db().await;
    setup(&db, |tx| {
        put_call(tx, 1, T0, "")?;
        put_call(tx, 2, T0 + MIN, "")?;
        put_call(tx, 3, T0 + MIN, "")
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/whatsapp-call-list-for-workspace.test.ts:197 "whatsappCallRepository.listForWorkspace — where-builder > cursor keyset predicate (microsecond precision must survive the round-trip) > produces createdAt < cursor::timestamptz OR (createdAt = cursor::timestamptz AND id < cursor.id)"
#[tokio::test]
async fn whatsapp_call_repository_list_for_workspace_where_builder_cursor_keyset_predicate_microsecond_precision_must_survive_the_round_trip_produces_created_at_cursor_timestamptz_or_created_at_cursor_timestamptz_and_id_cursor_id()
 {
    let db = tie_db().await;
    let page = ListForWorkspaceInput {
        cursor: Some(WhatsappCallListCursor {
            created_at: iso(T0 + MIN),
            id: 3,
        }),
        ..input(WhatsappCallHistoryScope::AllCalls)
    };
    assert_eq!(ids(&db, page).await, vec![2, 1]);
}

/// Port of: packages/database/__tests__/whatsapp-call-list-for-workspace.test.ts:241 "whatsappCallRepository.listForWorkspace — where-builder > cursor keyset predicate (microsecond precision must survive the round-trip) > selects createdAt cast to ::text so the next page's cursor keeps full precision"
#[tokio::test]
async fn whatsapp_call_repository_list_for_workspace_where_builder_cursor_keyset_predicate_microsecond_precision_must_survive_the_round_trip_selects_created_at_cast_to_text_so_the_next_page_s_cursor_keeps_full_precision()
 {
    let db = tie_db().await;
    let first_page = ListForWorkspaceInput {
        limit: 1,
        ..input(WhatsappCallHistoryScope::AllCalls)
    };
    let first = ok(repo::read(&db, move |c| list_for_workspace(c, &first_page)).await);
    assert_eq!(first.len(), 1);
    let row = &first[0];
    assert_eq!(parse_iso(&row.created_at_cursor), Some(row.call.created_at));
    let next = ListForWorkspaceInput {
        limit: 1,
        cursor: Some(WhatsappCallListCursor {
            created_at: row.created_at_cursor.clone(),
            id: row.call.id,
        }),
        ..input(WhatsappCallHistoryScope::AllCalls)
    };
    assert_eq!(ids(&db, next).await, vec![2]);
}

/// Port of: packages/database/__tests__/whatsapp-call-list-for-workspace.test.ts:258 "whatsappCallRepository.listForWorkspace — where-builder > orders by createdAt desc, id desc (stable tie-break) and requests exactly `limit` rows"
#[tokio::test]
async fn whatsapp_call_repository_list_for_workspace_where_builder_orders_by_created_at_desc_id_desc_stable_tie_break_and_requests_exactly_limit_rows()
 {
    let db = tie_db().await;
    let page = ListForWorkspaceInput {
        limit: 2,
        ..input(WhatsappCallHistoryScope::AllCalls)
    };
    assert_eq!(ids(&db, page).await, vec![3, 2]);
}
