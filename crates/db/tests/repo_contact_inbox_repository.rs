//! Port of `packages/database/__tests__/contact-inbox-repository.test.ts` (P3.2).
//! The mocked chains (selected columns, joined models, rendered SQL) become real rows of a
//! migrated SQLite file (adapted): a join scoped to the workspace = rows of another
//! workspace are not returned; "no query" = an empty result for an empty id list.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::contact_inbox::{
    self as ci, AdEligibleRow, CtwaInboxRow, IdentityFields, IdentityHistoryAppend, WorkspaceRow,
};
use chatbotx_db::repo::{self};
use chatbotx_db::rusqlite::Connection;
use repo_common::ok;
use serde_json::json;

const WS: i64 = 2001;
const OTHER: i64 = 2002;

async fn db() -> common::TestDb {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    common::seed_workspace(&db, 1002, OTHER).await;
    db
}

fn some(v: &str) -> Option<Option<String>> {
    Some(Some(v.to_owned()))
}

fn ci_row(
    tx: &Connection,
    id: i64,
    contact: i64,
    inbox: i64,
    channel: &str,
    source_id: &str,
    referral: Option<&str>,
) {
    repo_common::insert_ci(tx, id, contact, inbox, channel, source_id).expect("ci");
    tx.execute(
        r#"UPDATE "ContactInbox" SET "referral" = ?1 WHERE "id" = ?2"#,
        chatbotx_db::rusqlite::params![referral, id],
    )
    .expect("referral");
}

/// Identity fixture: contact inbox 20 (`user.bsuid-old`, parent `parent.bsuid-1`).
async fn identity_db() -> common::TestDb {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        ins!(tx, "ContactInbox", { "id" => 20_i64, "originalContactId" => 4001_i64, "contactId" => 4001_i64, "inboxId" => 3001_i64, "channel" => "whatsapp", "source" => "whatsapp", "sourceId" => "84900000001", "sourceUserId" => "user.bsuid-old", "sourceParentUserId" => "parent.bsuid-1" })?;
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:40 "contactInboxRepository.updateIdentityGuarded > returns the updated row from a guarded identity compare-and-swap"
#[tokio::test]
async fn contact_inbox_repository_update_identity_guarded_returns_the_updated_row_from_a_guarded_identity_compare_and_swap()
 {
    let db = identity_db().await;
    let guard = IdentityFields {
        source_user_id: some("user.bsuid-old"),
        source_parent_user_id: some("parent.bsuid-1"),
        ..Default::default()
    };
    let set = IdentityFields {
        source_id: some("user.bsuid-new"),
        source_user_id: some("user.bsuid-new"),
        ..Default::default()
    };
    let append = IdentityHistoryAppend {
        changed_at: "2026-09-28T05:00:00.000Z".into(),
        reason: "userIdChanged".into(),
    };
    let row = ok(repo::write(&db, move |tx| {
        ci::update_identity_guarded(tx, 20, &guard, &set, Some(&append))
    })
    .await)
    .expect("row");
    assert_eq!(row.source_id, "user.bsuid-new");
    assert_eq!(row.source_user_id.as_deref(), Some("user.bsuid-new"));
    assert_eq!(row.source_parent_user_id.as_deref(), Some("parent.bsuid-1"));
    assert_eq!(
        ci::identity_history(&row),
        json!([{ "sourceId": "84900000001", "sourceUserId": "user.bsuid-old", "sourceParentUserId": "parent.bsuid-1", "changedAt": "2026-09-28T05:00:00.000Z", "reason": "userIdChanged" }])
    );
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:104 "contactInboxRepository.updateIdentityGuarded > derives each disjoint update history entry from the row at update time"
#[tokio::test]
async fn contact_inbox_repository_update_identity_guarded_derives_each_disjoint_update_history_entry_from_the_row_at_update_time()
 {
    let db = identity_db().await;
    let g1 = IdentityFields {
        source_user_id: some("user.bsuid-old"),
        ..Default::default()
    };
    let s1 = IdentityFields {
        source_user_id: some("user.bsuid-new"),
        ..Default::default()
    };
    let a1 = IdentityHistoryAppend {
        changed_at: "2026-09-28T05:00:00.000Z".into(),
        reason: "userIdChanged".into(),
    };
    ok(repo::write(&db, move |tx| {
        ci::update_identity_guarded(tx, 20, &g1, &s1, Some(&a1))
    })
    .await)
    .expect("first");
    let g2 = IdentityFields {
        source_id: some("84900000001"),
        ..Default::default()
    };
    let s2 = IdentityFields {
        source_id: some("84900000002"),
        ..Default::default()
    };
    let a2 = IdentityHistoryAppend {
        changed_at: "2026-09-28T05:00:01.000Z".into(),
        reason: "phoneChanged".into(),
    };
    let row = ok(repo::write(&db, move |tx| {
        ci::update_identity_guarded(tx, 20, &g2, &s2, Some(&a2))
    })
    .await)
    .expect("second");
    let history = ci::identity_history(&row);
    // The second entry carries the row as it was at the second update (the new user id).
    assert_eq!(history[0]["sourceUserId"], "user.bsuid-old");
    assert_eq!(history[1]["sourceUserId"], "user.bsuid-new");
    assert_eq!(history[1]["sourceId"], "84900000001");
    assert_ne!(history[0], history[1]);
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:166 "contactInboxRepository.updateIdentityGuarded > returns undefined when the stored identity no longer matches the guard"
#[tokio::test]
async fn contact_inbox_repository_update_identity_guarded_returns_undefined_when_the_stored_identity_no_longer_matches_the_guard()
 {
    let db = identity_db().await;
    let guard = IdentityFields {
        source_user_id: Some(None),
        source_parent_user_id: some("parent.bsuid-1"),
        ..Default::default()
    };
    let set = IdentityFields {
        source_user_id: some("user.bsuid-new"),
        ..Default::default()
    };
    assert!(
        ok(repo::write(&db, move |tx| ci::update_identity_guarded(
            tx, 20, &guard, &set, None
        ))
        .await)
        .is_none()
    );
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:193 "contactInboxRepository.updateIdentityGuarded > throws without issuing an update when called without an identity guard"
#[tokio::test]
async fn contact_inbox_repository_update_identity_guarded_throws_without_issuing_an_update_when_called_without_an_identity_guard()
 {
    let db = identity_db().await;
    let set = IdentityFields {
        source_user_id: some("user.bsuid-new"),
        ..Default::default()
    };
    let message = repo_common::err_message(
        repo::write(&db, move |tx| {
            ci::update_identity_guarded(tx, 20, &IdentityFields::default(), &set, None)
        })
        .await,
    );
    assert!(
        message.contains("ContactInbox identity update requires a guard"),
        "{message}"
    );
    let user = repo_common::scalar_text(
        &db,
        r#"SELECT "sourceUserId" FROM "ContactInbox" WHERE "id" = 20"#,
        vec![],
    )
    .await;
    assert_eq!(user.as_deref(), Some("user.bsuid-old"));
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:216 "contactInboxRepository.findByIdForWorkspace > joins Inbox scoped to the workspace and returns the single row"
#[tokio::test]
async fn contact_inbox_repository_find_by_id_for_workspace_joins_inbox_scoped_to_the_workspace_and_returns_the_single_row()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        ci_row(tx, 20, 4001, 3001, "whatsapp", "psid-1", None);
        Ok(())
    })
    .await;
    let row = ok(repo::read(&db, |c| ci::find_by_id_for_workspace(c, 20, WS)).await);
    assert_eq!(
        row,
        Some(WorkspaceRow {
            id: 20,
            channel: "whatsapp".into(),
            inbox_id: 3001,
            source_id: "psid-1".into()
        })
    );
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:259 "contactInboxRepository.findByIdForWorkspace > returns null when no row matches (missing id or wrong workspace)"
#[tokio::test]
async fn contact_inbox_repository_find_by_id_for_workspace_returns_null_when_no_row_matches_missing_id_or_wrong_workspace()
 {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        ci_row(tx, 20, 4001, 3001, "whatsapp", "psid-1", None);
        Ok(())
    })
    .await;
    assert!(ok(repo::read(&db, |c| ci::find_by_id_for_workspace(c, 999, WS)).await).is_none());
    assert!(ok(repo::read(&db, |c| ci::find_by_id_for_workspace(c, 20, OTHER)).await).is_none());
}

/// WhatsApp inboxes 3101 (integration 6001 of 2001) and 3102 (integration 6002 of 2002).
async fn whatsapp_db() -> common::TestDb {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        for (inbox, ws, iw) in [(3101_i64, WS, 6001_i64), (3102, OTHER, 6002)] {
            repo_common::insert_inbox(tx, inbox, ws, "whatsapp", &format!("wa-{inbox}"))?;
            ins!(tx, "IntegrationWhatsapp", { "id" => iw, "auth" => "{}", "phoneNumberId" => format!("pn-{iw}"), "wabaId" => "w", "businessId" => "b", "name" => "WA", "workspaceId" => ws, "inboxId" => inbox })?;
        }
        repo_common::insert_contact(tx, 10, WS)?;
        repo_common::insert_contact(tx, 11, WS)?;
        ci_row(tx, 20, 10, 3101, "whatsapp", "wa-a", Some(r#"{"ctwaClid":"c1"}"#));
        ci_row(tx, 21, 10, 3101, "whatsapp", "wa-b", None);
        ci_row(tx, 22, 10, 3102, "whatsapp", "wa-c", Some(r#"{"ctwaClid":"c2"}"#));
        ci_row(tx, 23, 11, 3101, "whatsapp", "wa-d", Some(r#"{"ctwaClid":"c3"}"#));
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:287 "contactInboxRepository.listWhatsappCtwaInboxesByContact > joins IntegrationWhatsapp scoped to the workspace and selects contactInbox/integration ids"
#[tokio::test]
async fn contact_inbox_repository_list_whatsapp_ctwa_inboxes_by_contact_joins_integration_whatsapp_scoped_to_the_workspace_and_selects_contact_inbox_integration_ids()
 {
    let db = whatsapp_db().await;
    let rows = ok(repo::read(&db, |c| {
        ci::list_whatsapp_ctwa_inboxes_by_contact(c, WS, 10)
    })
    .await);
    assert_eq!(rows, vec![(20, 6001)]);
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:317 "contactInboxRepository.listWhatsappCtwaInboxesByContacts > returns [] without querying when contactIds is empty"
#[tokio::test]
async fn contact_inbox_repository_list_whatsapp_ctwa_inboxes_by_contacts_returns_without_querying_when_contact_ids_is_empty()
 {
    let db = whatsapp_db().await;
    assert!(
        ok(
            repo::read(&db, |c| ci::list_whatsapp_ctwa_inboxes_by_contacts(
                c,
                WS,
                &[]
            ))
            .await
        )
        .is_empty()
    );
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:329 "contactInboxRepository.listWhatsappCtwaInboxesByContacts > runs a single batch query across all requested contact ids"
#[tokio::test]
async fn contact_inbox_repository_list_whatsapp_ctwa_inboxes_by_contacts_runs_a_single_batch_query_across_all_requested_contact_ids()
 {
    let db = whatsapp_db().await;
    let rows = ok(repo::read(&db, |c| {
        ci::list_whatsapp_ctwa_inboxes_by_contacts(c, WS, &[10, 11])
    })
    .await);
    assert_eq!(
        rows,
        vec![
            CtwaInboxRow {
                contact_id: 10,
                contact_inbox_id: 20,
                integration_whatsapp_id: 6001
            },
            CtwaInboxRow {
                contact_id: 11,
                contact_inbox_id: 23,
                integration_whatsapp_id: 6001
            },
        ]
    );
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:375 "contactInboxRepository.listAdEligibleInboxesByContacts > returns [] without querying when contactIds is empty"
#[tokio::test]
async fn contact_inbox_repository_list_ad_eligible_inboxes_by_contacts_returns_without_querying_when_contact_ids_is_empty()
 {
    let db = whatsapp_db().await;
    assert!(
        ok(
            repo::read(&db, |c| ci::list_ad_eligible_inboxes_by_contacts(
                c,
                WS,
                &[]
            ))
            .await
        )
        .is_empty()
    );
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:387 "contactInboxRepository.listAdEligibleInboxesByContacts > runs one query per ads-eligible channel and tags each row with its channel"
#[tokio::test]
async fn contact_inbox_repository_list_ad_eligible_inboxes_by_contacts_runs_one_query_per_ads_eligible_channel_and_tags_each_row_with_its_channel()
 {
    let db = whatsapp_db().await;
    repo_common::setup(&db, |tx| {
        repo_common::insert_inbox(tx, 3103, WS, "messenger", "page-m")?;
        repo_common::insert_inbox(tx, 3104, WS, "instagram", "ig-i")?;
        ins!(tx, "IntegrationMessenger", { "id" => 7001_i64, "auth" => "{}", "pageId" => "p", "name" => "P", "conversationStarters" => "[]", "persistentMenus" => "[]", "personas" => "[]", "workspaceId" => WS, "inboxId" => 3103_i64 })?;
        ins!(tx, "IntegrationInstagram", { "id" => 8001_i64, "auth" => "{}", "igId" => "i", "pageId" => "p", "name" => "I", "username" => "u", "conversationStarters" => "[]", "persistentMenus" => "[]", "workspaceId" => WS, "inboxId" => 3104_i64 })?;
        ci_row(tx, 30, 10, 3103, "messenger", "m-a", Some(r#"{"adId":"ad-1","source":"ADS"}"#));
        ci_row(tx, 31, 10, 3103, "messenger", "m-b", Some(r#"{"adId":"ad-1","source":"SHORTLINK"}"#));
        ci_row(tx, 32, 11, 3104, "instagram", "i-a", Some(r#"{"adId":"ad-2","source":"ADS"}"#));
        Ok(())
    })
    .await;
    let rows = ok(repo::read(&db, |c| {
        ci::list_ad_eligible_inboxes_by_contacts(c, WS, &[10, 11])
    })
    .await);
    let row = |contact, ci, channel: &str, integration| AdEligibleRow {
        contact_id: contact,
        contact_inbox_id: ci,
        channel: channel.into(),
        integration_id: integration,
    };
    assert_eq!(
        rows,
        vec![
            row(10, 20, "whatsapp", 6001),
            row(11, 23, "whatsapp", 6001),
            row(10, 30, "messenger", 7001),
            row(11, 32, "instagram", 8001),
        ]
    );
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:474 "contactInboxRepository.findByInboxAndSourceIds > scopes by inbox, dedups the source ids, and returns the rows"
#[tokio::test]
async fn contact_inbox_repository_find_by_inbox_and_source_ids_scopes_by_inbox_dedups_the_source_ids_and_returns_the_rows()
 {
    let db = whatsapp_db().await;
    let ids = vec!["wa-a".to_owned(), "wa-a".to_owned()];
    let rows = ok(repo::read(&db, move |c| {
        ci::find_by_inbox_and_source_ids(c, 3101, &ids)
    })
    .await);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].id, rows[0].source_id.as_str()), (20, "wa-a"));
    // Another inbox's row with that source id is not returned.
    let other = ok(repo::read(&db, |c| {
        ci::find_by_inbox_and_source_ids(c, 3101, &["wa-c".to_owned()])
    })
    .await);
    assert!(other.is_empty());
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:499 "contactInboxRepository.findByInboxAndSourceIds > makes no query at all for an empty id list"
#[tokio::test]
async fn contact_inbox_repository_find_by_inbox_and_source_ids_makes_no_query_at_all_for_an_empty_id_list()
 {
    let db = whatsapp_db().await;
    assert!(
        ok(repo::read(&db, |c| ci::find_by_inbox_and_source_ids(c, 3101, &[])).await).is_empty()
    );
}

// `sourceId` is NOT NULL in the schema; an empty source id stands for the unaddressable
// row of the TS mock.
/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:512 "contactInboxRepository.findByInboxAndSourceIds > drops rows whose sourceId is null — they cannot be addressed by wa_id"
#[tokio::test]
async fn contact_inbox_repository_find_by_inbox_and_source_ids_drops_rows_whose_source_id_is_null_they_cannot_be_addressed_by_wa_id()
 {
    let db = whatsapp_db().await;
    repo_common::setup(&db, |tx| {
        ci_row(tx, 40, 11, 3101, "whatsapp", "", None);
        Ok(())
    })
    .await;
    let ids = vec![String::new(), "wa-b".to_owned()];
    let rows = ok(repo::read(&db, move |c| {
        ci::find_by_inbox_and_source_ids(c, 3101, &ids)
    })
    .await);
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![21]);
}

async fn attribution_db() -> common::TestDb {
    let db = db().await;
    repo_common::setup(&db, |tx| {
        repo_common::insert_inbox(tx, 3007, WS, "messenger", "p7")?;
        repo_common::insert_contact(tx, 10, WS)?;
        repo_common::insert_contact(tx, 11, OTHER)?;
        ci_row(tx, 20, 10, 3001, "messenger", "a", None);
        ci_row(tx, 21, 10, 3007, "messenger", "b", None);
        ci_row(tx, 22, 11, 3001, "messenger", "c", None);
        tx.execute(
            r#"UPDATE "ContactInbox" SET "lastMessageAt" = 100 WHERE "id" = 20"#,
            [],
        )?;
        tx.execute(
            r#"UPDATE "ContactInbox" SET "lastMessageAt" = 200 WHERE "id" = 21"#,
            [],
        )?;
        Ok(())
    })
    .await;
    db
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:564 "contactInboxRepository analytics attribution > latest attribution: one row per contact, most recently active first, scoped to the workspace"
#[tokio::test]
async fn contact_inbox_repository_analytics_attribution_latest_attribution_one_row_per_contact_most_recently_active_first_scoped_to_the_workspace()
 {
    let db = attribution_db().await;
    let rows = ok(repo::read(&db, |c| {
        ci::list_latest_analytics_attribution(c, WS, &[10, 11])
    })
    .await);
    assert_eq!(rows, vec![(10, 3007, "messenger".to_owned())]);
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:594 "contactInboxRepository analytics attribution > latest attribution with no contacts makes no query"
#[tokio::test]
async fn contact_inbox_repository_analytics_attribution_latest_attribution_with_no_contacts_makes_no_query()
 {
    let db = attribution_db().await;
    assert!(
        ok(repo::read(&db, |c| ci::list_latest_analytics_attribution(c, WS, &[])).await).is_empty()
    );
}

/// Port of: packages/database/__tests__/contact-inbox-repository.test.ts:606 "contactInboxRepository analytics attribution > per-contact attribution lists every connection of the contact in the workspace"
#[tokio::test]
async fn contact_inbox_repository_analytics_attribution_per_contact_attribution_lists_every_connection_of_the_contact_in_the_workspace()
 {
    let db = attribution_db().await;
    let rows = ok(repo::read(&db, |c| {
        ci::list_analytics_attribution_by_contact(c, WS, 10)
    })
    .await);
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (
            rows[0].inbox_id,
            rows[0].source.as_str(),
            rows[0].source_id.as_str()
        ),
        (3001, "messenger", "a")
    );
    assert!(
        ok(
            repo::read(&db, |c| ci::list_analytics_attribution_by_contact(
                c, WS, 11
            ))
            .await
        )
        .is_empty()
    );
}
