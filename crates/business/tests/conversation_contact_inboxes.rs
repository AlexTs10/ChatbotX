//! Port of `packages/business/__tests__/conversation-find-by-contact-with-inboxes.test.ts`
//! and `conversation-resolve-contact-inbox-for-send.test.ts` (P3.6-D3, slice
//! contact messages). Adapted: the spied `findFirst` / service calls become
//! real `Conversation` / `ContactInbox` rows; "orders DM first ..." is the
//! returned row of a seeded set that only that order picks.

mod common;

use chatbotx_business::conversation::{
    find_by_contact_with_inboxes, find_recent_contact_inbox, resolve_contact_inbox_for_send,
};
use chatbotx_db::rusqlite::{Connection, params};
use common::{OTHER_WS, WS, setup, two_workspaces};

/// Contact 1 on inboxes 5 (contact inbox 50) and 6 (contact inbox 60).
fn base(tx: &Connection) -> Result<(), chatbotx_db::DbError> {
    common::inbox(tx, 5, WS, "webchat")?;
    common::inbox(tx, 6, WS, "messenger")?;
    common::contact(tx, 1, WS)?;
    common::contact_inbox(tx, 50, 1, 5, "webchat", "guest-1")?;
    common::contact_inbox(tx, 60, 1, 6, "messenger", "psid-1")?;
    Ok(())
}

fn conversation(
    tx: &Connection,
    id: i64,
    source_id: Option<&str>,
    last_activity_at: Option<i64>,
) -> Result<(), chatbotx_db::DbError> {
    tx.execute(
        r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "sourceId", "lastActivityAt") VALUES (?1, ?2, 1, ?3, ?4)"#,
        params![id, WS, source_id, last_activity_at],
    )?;
    Ok(())
}

fn last_message(tx: &Connection, id: i64, at: i64) -> Result<(), chatbotx_db::DbError> {
    tx.execute(
        r#"UPDATE "ContactInbox" SET "lastMessageAt" = ?2 WHERE "id" = ?1"#,
        params![id, at],
    )?;
    Ok(())
}

// describe("conversationService.findByContactWithInboxes")

#[tokio::test]
async fn orders_dm_first_then_activity_with_nulls_last_and_id() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        base(tx)?;
        // One DM per contact (partial unique index): the activity order is
        // seen among comment threads, the DM priority once the DM exists.
        conversation(tx, 70, Some("post-a"), Some(1_000))?;
        conversation(tx, 71, Some("post-b"), None)?;
        conversation(tx, 72, Some("post-c"), Some(1_000))?;
        conversation(tx, 73, Some("post-d"), Some(500))?;
        Ok(())
    })
    .await;
    let found = find_by_contact_with_inboxes(&db, WS, 1)
        .await
        .expect("find")
        .expect("conversation");
    assert_eq!(found.conversation.id, 72);
    setup(&db, |tx| conversation(tx, 74, None, None)).await;
    let found = find_by_contact_with_inboxes(&db, WS, 1)
        .await
        .expect("find")
        .expect("conversation");
    assert_eq!(found.conversation.id, 74);
}

#[tokio::test]
async fn uses_one_ordered_lookup_and_returns_the_dm_thread() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        base(tx)?;
        conversation(tx, 70, Some("page_post"), Some(9_000))?;
        conversation(tx, 71, None, None)?;
        Ok(())
    })
    .await;
    let found = find_by_contact_with_inboxes(&db, WS, 1)
        .await
        .expect("find")
        .expect("conversation");
    assert_eq!(found.conversation.id, 71);
    let mut inboxes: Vec<i64> = found.contact_inboxes.iter().map(|ci| ci.id).collect();
    inboxes.sort_unstable();
    assert_eq!(inboxes, vec![50, 60]);
    assert_eq!(
        find_by_contact_with_inboxes(&db, OTHER_WS, 1)
            .await
            .expect("find"),
        None
    );
}

#[tokio::test]
async fn returns_a_comment_conversation_when_there_is_no_dm_thread() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        base(tx)?;
        conversation(tx, 70, Some("page-1_post-1"), None)?;
        Ok(())
    })
    .await;
    let found = find_by_contact_with_inboxes(&db, WS, 1)
        .await
        .expect("find")
        .expect("conversation");
    assert_eq!(found.conversation.id, 70);
}

// describe("conversationService.resolveContactInboxForSend")

#[tokio::test]
async fn resolves_the_contact_inbox_matching_an_explicit_inbox_id() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        base(tx)?;
        conversation(tx, 70, None, None)?;
        last_message(tx, 50, 9_000)?;
        Ok(())
    })
    .await;
    let (conversation, contact_inbox) = resolve_contact_inbox_for_send(&db, WS, 1, Some(6))
        .await
        .expect("resolve");
    assert_eq!(contact_inbox.id, 60);
    assert_eq!(conversation.conversation.id, 70);
}

#[tokio::test]
async fn resolves_the_most_recently_active_contact_inbox_when_inbox_id_is_omitted() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        base(tx)?;
        conversation(tx, 70, None, None)?;
        last_message(tx, 50, 1_000)?;
        last_message(tx, 60, 2_000)?;
        Ok(())
    })
    .await;
    let (_, contact_inbox) = resolve_contact_inbox_for_send(&db, WS, 1, None)
        .await
        .expect("resolve");
    assert_eq!(contact_inbox.id, 60);
    assert_eq!(
        find_recent_contact_inbox(&db, WS, 1)
            .await
            .expect("recent")
            .map(|ci| ci.id),
        Some(60)
    );
}

#[tokio::test]
async fn takes_the_conversation_find_by_contact_with_inboxes_settles_on_passing_no_opt_in() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        base(tx)?;
        conversation(tx, 70, Some("page_post"), Some(9_000))?;
        conversation(tx, 71, None, None)?;
        Ok(())
    })
    .await;
    let (conversation, _) = resolve_contact_inbox_for_send(&db, WS, 1, None)
        .await
        .expect("resolve");
    assert_eq!(conversation.conversation.id, 71);
}

#[tokio::test]
async fn still_resolves_a_comment_conversation_when_the_contact_has_no_dm_thread() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        base(tx)?;
        conversation(tx, 70, Some("page-1_post-1"), None)?;
        last_message(tx, 50, 5_000)?;
        Ok(())
    })
    .await;
    let (conversation, contact_inbox) = resolve_contact_inbox_for_send(&db, WS, 1, None)
        .await
        .expect("resolve");
    assert_eq!(conversation.conversation.id, 70);
    assert_eq!(contact_inbox.id, 50);
}

#[tokio::test]
async fn it_404s_when_no_conversation_exists_for_the_contact() {
    let db = two_workspaces().await;
    setup(&db, base).await;
    let error = resolve_contact_inbox_for_send(&db, WS, 1, None)
        .await
        .expect_err("no conversation");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Conversation not found");
}

#[tokio::test]
async fn it_404s_when_the_conversation_exists_but_no_contact_inbox_matches_the_given_inbox_id() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        base(tx)?;
        conversation(tx, 70, None, None)?;
        Ok(())
    })
    .await;
    let error = resolve_contact_inbox_for_send(&db, WS, 1, Some(999))
        .await
        .expect_err("no inbox");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Conversation not found");
}

#[tokio::test]
async fn it_404s_when_the_contact_has_no_contact_inbox_at_all() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        common::contact(tx, 1, WS)?;
        conversation(tx, 70, None, None)?;
        Ok(())
    })
    .await;
    let error = resolve_contact_inbox_for_send(&db, WS, 1, None)
        .await
        .expect_err("no contact inbox");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.message(), "Conversation not found");
}
