//! Port of `packages/business/__tests__/contact-list.test.ts` (P3.6-D2 slice
//! contacts list). Adapted: the spied `contactRepository` / `inboxService`
//! calls become stored contacts, inboxes and conversations; the asserted
//! `buildListWhere` input becomes the listed contact ids; the asserted
//! repository function (`listTableRows` / `listWithInboxesAndConversation` /
//! `listWithRelations`) becomes the row kind and the relations it carries;
//! the bounded-count cap case counts below the cap (100,000 rows are not
//! seeded) and checks the reported cap.

mod common;

use chatbotx_business::contact::list::{
    ContactListInclude, ContactListScope, ListContactsInput, ListOptions, ListRows, ListScope,
    ListedContact, Projection, count, list,
};
use chatbotx_business::count_mode::{BOUNDED_COUNT_CAP, CountMode, CountOutcome};
use chatbotx_db::rusqlite::params;
use common::{
    OTHER_WS, TestDb, WS, contact, contact_inbox, contact_with, custom_field, inbox, setup, tag,
    tag_link, two_workspaces,
};

const USER: i64 = 1001;
const WHATSAPP: i64 = 3001;
const MESSENGER: i64 = 3002;
const HOUR: i64 = 3_600_000;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// 101 Ada (whatsapp, recent inbound, conversation assigned to `USER`, a
/// tag, a custom field value), 102 Bob (whatsapp, inbound two days ago,
/// unassigned conversation), 103 Cy (messenger, recent inbound), 104 Dee (no
/// inbox); 105 in `OTHER_WS`. Created oldest first: 101, 102, 103, 104.
async fn db() -> TestDb {
    let db = two_workspaces().await;
    let now = now();
    setup(&db, move |tx| {
        inbox(tx, WHATSAPP, WS, "whatsapp")?;
        inbox(tx, MESSENGER, WS, "messenger")?;
        contact_with(tx, 101, WS, Some("Ada"), Some("ada@example.com"), Some("+15551234567"))?;
        contact_with(tx, 102, WS, Some("Bob"), Some("bob@example.com"), Some("+15557654321"))?;
        contact_with(tx, 103, WS, Some("Cy"), None, None)?;
        contact_with(tx, 104, WS, Some("Dee"), None, None)?;
        contact(tx, 105, OTHER_WS)?;
        for (id, minute) in [(101, 1), (102, 2), (103, 3), (104, 4)] {
            tx.execute(
                r#"UPDATE "Contact" SET "createdAt" = ?2 WHERE "id" = ?1"#,
                params![id, 1_735_689_600_000_i64 + minute * 60_000],
            )?;
        }
        contact_inbox(tx, 4001, 101, WHATSAPP, "whatsapp", "wa-101")?;
        contact_inbox(tx, 4002, 102, WHATSAPP, "whatsapp", "wa-102")?;
        contact_inbox(tx, 4003, 103, MESSENGER, "messenger", "m-103")?;
        for (id, at) in [(4001, now - HOUR), (4002, now - 48 * HOUR), (4003, now - HOUR)] {
            tx.execute(
                r#"UPDATE "ContactInbox" SET "lastIncomingMessageAt" = ?2 WHERE "id" = ?1"#,
                params![id, at],
            )?;
        }
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "assignedUserId") VALUES (5001, ?1, 101, ?2)"#,
            params![WS, USER],
        )?;
        tx.execute(
            r#"INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES (5002, ?1, 102)"#,
            params![WS],
        )?;
        tag(tx, 1, WS, "vip")?;
        tag_link(tx, 101, 1)?;
        custom_field(tx, 1, WS, "tier", "shortText")?;
        tx.execute(
            r#"INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value") VALUES (1, 101, 1, 'gold')"#,
            [],
        )?;
        Ok(())
    })
    .await;
    db
}

fn input() -> ListContactsInput {
    ListContactsInput {
        workspace_id: WS,
        ..ListContactsInput::default()
    }
}

fn restricted() -> ListScope {
    ListScope::Member(ContactListScope {
        restrict_to_assigned_user_id: Some(USER),
        restrict_to_team_ids: None,
        can_view_email_and_phone: false,
    })
}

fn full(rows: &ListRows) -> &[ListedContact] {
    match rows {
        ListRows::Full(rows) => rows,
        ListRows::Table(_) => panic!("expected full rows"),
    }
}

fn ids(rows: &ListRows) -> Vec<i64> {
    let mut ids: Vec<i64> = match rows {
        ListRows::Full(rows) => rows.iter().map(|r| r.contact.id).collect(),
        ListRows::Table(rows) => rows.iter().map(|r| r.id).collect(),
    };
    ids.sort_unstable();
    ids
}

async fn listed(db: &TestDb, input: &ListContactsInput, options: ListOptions) -> ListRows {
    list(db, input, &options, now()).await.expect("list").data
}

async fn listed_ids(db: &TestDb, input: &ListContactsInput) -> Vec<i64> {
    ids(&listed(db, input, ListOptions::default()).await)
}

fn including(include: Vec<ContactListInclude>) -> ListOptions {
    ListOptions {
        include: Some(include),
        ..ListOptions::default()
    }
}

// ---- contactService.list ----------------------------------------------------

#[tokio::test]
async fn contact_service_list_scope_can_view_email_and_phone_false_masks_email_phone_and_forwards_include_email_and_phone_false_restrict_to_assigned_user_id()
 {
    let db = db().await;
    let rows = listed(
        &db,
        &input(),
        ListOptions {
            scope: restricted(),
            ..ListOptions::default()
        },
    )
    .await;
    assert_eq!(ids(&rows), vec![101]);
    let row = &full(&rows)[0];
    assert_eq!(row.contact.email, None);
    assert_eq!(row.contact.phone_number, None);
}

#[tokio::test]
async fn contact_service_list_unscoped_call_scope_unscoped_does_not_mask_email_phone() {
    let db = db().await;
    let rows = listed(&db, &input(), ListOptions::default()).await;
    assert_eq!(ids(&rows), vec![101, 102, 103, 104]);
    let ada = full(&rows)
        .iter()
        .find(|r| r.contact.id == 101)
        .expect("ada");
    assert_eq!(ada.contact.email.as_deref(), Some("ada@example.com"));
    assert_eq!(ada.contact.phone_number.as_deref(), Some("+15551234567"));
}

#[tokio::test]
async fn contact_service_list_with_count_false_skips_the_count_round_trip_and_returns_total_count_0()
 {
    let db = db().await;
    let result = list(
        &db,
        &input(),
        &ListOptions {
            with_count: false,
            ..ListOptions::default()
        },
        now(),
    )
    .await
    .expect("list");
    assert_eq!(result.total_count, 0);
    assert!(!result.total_count_capped);
    assert_eq!(result.page_count, 0);
    assert_eq!(ids(&result.data).len(), 4);
}

#[tokio::test]
async fn contact_service_list_projection_table_uses_list_table_rows_not_list_with_relations() {
    let db = db().await;
    let rows = listed(
        &db,
        &input(),
        ListOptions {
            projection: Projection::Table,
            ..ListOptions::default()
        },
    )
    .await;
    assert!(matches!(rows, ListRows::Table(_)));
    assert_eq!(ids(&rows), vec![101, 102, 103, 104]);
}

#[tokio::test]
async fn contact_service_list_projection_table_for_a_restricted_member_masks_email_phone_and_passes_include_email_and_phone_false_restrict_to_assigned_user_id()
 {
    let db = db().await;
    let rows = listed(
        &db,
        &input(),
        ListOptions {
            scope: restricted(),
            projection: Projection::Table,
            ..ListOptions::default()
        },
    )
    .await;
    let ListRows::Table(rows) = rows else {
        panic!("expected table rows")
    };
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![101]);
    assert_eq!(rows[0].email, None);
    assert_eq!(rows[0].phone_number, None);
}

#[tokio::test]
async fn contact_service_list_projection_table_keeps_email_phone_for_a_caller_allowed_to_see_them()
{
    let db = db().await;
    let rows = listed(
        &db,
        &input(),
        ListOptions {
            projection: Projection::Table,
            ..ListOptions::default()
        },
    )
    .await;
    let ListRows::Table(rows) = rows else {
        panic!("expected table rows")
    };
    let ada = rows.iter().find(|r| r.id == 101).expect("ada");
    assert_eq!(ada.email.as_deref(), Some("ada@example.com"));
    assert_eq!(ada.phone_number.as_deref(), Some("+15551234567"));
}

#[tokio::test]
async fn contact_service_list_include_omitting_both_tags_and_custom_fields_uses_list_with_inboxes_and_conversation()
 {
    let db = db().await;
    let rows = listed(&db, &input(), including(vec![ContactListInclude::Inboxes])).await;
    let ada = full(&rows)
        .iter()
        .find(|r| r.contact.id == 101)
        .expect("ada");
    assert!(ada.tags.is_none());
    assert!(ada.contact_custom_fields.is_none());
    assert_eq!(ada.contact_inboxes.as_ref().map(Vec::len), Some(1));
    assert!(ada.conversation.is_none());
}

#[tokio::test]
async fn contact_service_list_include_containing_tags_uses_list_with_relations() {
    let db = db().await;
    let rows = listed(&db, &input(), including(vec![ContactListInclude::Tags])).await;
    let ada = full(&rows)
        .iter()
        .find(|r| r.contact.id == 101)
        .expect("ada");
    assert_eq!(ada.tags.as_ref().map(Vec::len), Some(1));
    assert!(ada.contact_custom_fields.is_none());
    assert!(ada.contact_inboxes.is_none());
    assert!(ada.conversation.is_none());
}

#[tokio::test]
async fn contact_service_list_include_containing_custom_fields_uses_list_with_relations() {
    let db = db().await;
    let rows = listed(
        &db,
        &input(),
        including(vec![ContactListInclude::CustomFields]),
    )
    .await;
    let ada = full(&rows)
        .iter()
        .find(|r| r.contact.id == 101)
        .expect("ada");
    assert_eq!(ada.contact_custom_fields.as_ref().map(Vec::len), Some(1));
    assert!(ada.tags.is_none());
}

#[tokio::test]
async fn contact_service_list_include_omitted_entirely_uses_list_with_relations_default_full_relation_set()
 {
    let db = db().await;
    let rows = listed(&db, &input(), ListOptions::default()).await;
    let ada = full(&rows)
        .iter()
        .find(|r| r.contact.id == 101)
        .expect("ada");
    assert_eq!(ada.tags.as_ref().map(Vec::len), Some(1));
    assert_eq!(ada.contact_custom_fields.as_ref().map(Vec::len), Some(1));
    assert_eq!(ada.contact_inboxes.as_ref().map(Vec::len), Some(1));
    assert!(matches!(ada.conversation, Some(Some(_))));
}

// ---- contactService inbox scope keys ----------------------------------------

#[tokio::test]
async fn contact_service_inbox_scope_keys_list_resolves_channels_to_inbox_ids_and_passes_them_to_build_list_where()
 {
    let db = db().await;
    let input = ListContactsInput {
        channels: Some(vec!["whatsapp".into()]),
        subaction: Some("whatsappWithin24Hours".into()),
        ..input()
    };
    assert_eq!(listed_ids(&db, &input).await, vec![101]);
}

#[tokio::test]
async fn contact_service_inbox_scope_keys_list_without_inbox_keys_never_resolves_inboxes_and_sends_no_scope()
 {
    let db = db().await;
    assert_eq!(listed_ids(&db, &input()).await, vec![101, 102, 103, 104]);
}

#[tokio::test]
async fn contact_service_inbox_scope_keys_an_empty_channels_array_is_treated_as_not_provided() {
    let db = db().await;
    let input = ListContactsInput {
        channels: Some(Vec::new()),
        ..input()
    };
    assert_eq!(listed_ids(&db, &input).await, vec![101, 102, 103, 104]);
}

#[tokio::test]
async fn contact_service_inbox_scope_keys_an_empty_inbox_ids_array_is_treated_as_not_provided_so_channels_still_narrow()
 {
    let db = db().await;
    let input = ListContactsInput {
        inbox_ids: Some(Vec::new()),
        channels: Some(vec!["whatsapp".into()]),
        ..input()
    };
    assert_eq!(listed_ids(&db, &input).await, vec![101, 102]);
}

#[tokio::test]
async fn contact_service_inbox_scope_keys_inbox_ids_alone_applies_no_inbox_restriction() {
    let db = db().await;
    let input = ListContactsInput {
        inbox_ids: Some(Vec::new()),
        ..input()
    };
    assert_eq!(listed_ids(&db, &input).await, vec![101, 102, 103, 104]);
}

#[tokio::test]
async fn contact_service_inbox_scope_keys_subaction_alone_applies_only_the_recent_interaction_window()
 {
    let db = db().await;
    let input = ListContactsInput {
        subaction: Some("messengerActiveContacts".into()),
        ..input()
    };
    assert_eq!(listed_ids(&db, &input).await, vec![101, 103]);
}

#[tokio::test]
async fn contact_service_inbox_scope_keys_count_with_only_an_inbox_key_does_not_use_the_unfiltered_stats_shortcut()
 {
    let db = db().await;
    let input = ListContactsInput {
        inbox_ids: Some(vec![WHATSAPP]),
        ..input()
    };
    let result = count(&db, &input, &ListScope::Unscoped, None, now())
        .await
        .expect("count");
    // The stats sum is 3 (three contact inboxes); the real count is 2.
    assert_eq!(
        result,
        CountOutcome {
            total: 2,
            total_count_capped: false,
            count_cap: None
        }
    );
}

#[tokio::test]
async fn contact_service_inbox_scope_keys_bounded_count_stops_at_the_server_cap_and_reports_a_lower_bound()
 {
    let db = db().await;
    let input = ListContactsInput {
        keyword: Some("ada".into()),
        ..input()
    };
    let result = count(
        &db,
        &input,
        &ListScope::Unscoped,
        Some(CountMode::Bounded),
        now(),
    )
    .await
    .expect("count");
    assert_eq!(
        result,
        CountOutcome {
            total: 1,
            total_count_capped: false,
            count_cap: Some(BOUNDED_COUNT_CAP)
        }
    );
}

#[tokio::test]
async fn contact_service_inbox_scope_keys_the_unfiltered_stats_shortcut_is_exact_in_either_mode() {
    let db = db().await;
    let result = count(
        &db,
        &input(),
        &ListScope::Unscoped,
        Some(CountMode::Bounded),
        now(),
    )
    .await
    .expect("count");
    assert_eq!(
        result,
        CountOutcome {
            total: 3,
            total_count_capped: false,
            count_cap: None
        }
    );
}
