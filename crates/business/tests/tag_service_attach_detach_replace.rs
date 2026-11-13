//! Port of `packages/business/__tests__/tag-service-attach-detach-replace.test.ts`.
//! Adapted: the mocked tag / contact queries become real rows; "enqueue
//! attach / detach for a pair" becomes the stored `"ContactToTag"` links.
//! n-a: `emitTagApplied` / `emitTagRemoved` (no event bus), tag-sync and
//! ads-conversion enqueues (no jobs yet), cache invalidation, member access
//! scopes (workspace tokens only).

mod common;

use chatbotx_business::tag::{self as svc, ContactsOutcome};
use common::{
    OTHER_WS, WS, contact, linked_tag_ids, scalar_i64, setup, tag, tag_link, two_workspaces,
};

/// Contacts 10, 11 (WS) and 20 (OTHER_WS); tags 1 "alpha", 2 "beta" (WS)
/// and 9 "alpha" (OTHER_WS); contact 11 has "alpha".
async fn fixture() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        contact(tx, 10, WS)?;
        contact(tx, 11, WS)?;
        contact(tx, 20, OTHER_WS)?;
        tag(tx, 1, WS, "alpha")?;
        tag(tx, 2, WS, "beta")?;
        tag(tx, 9, OTHER_WS, "alpha")?;
        tag_link(tx, 11, 1)?;
        Ok(())
    })
    .await;
    db
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| (*n).to_string()).collect()
}

async fn tag_count(db: &chatbotx_db::Db) -> i64 {
    scalar_i64(
        db,
        r#"SELECT count(*) FROM "Tag" WHERE "workspaceId" = ?1"#,
        WS,
    )
    .await
    .unwrap_or_default()
}

fn outcome(processed: &[i64], skipped: &[i64]) -> ContactsOutcome {
    ContactsOutcome {
        processed_contact_ids: processed.to_vec(),
        skipped_contact_ids: skipped.to_vec(),
    }
}

// ---- attachByNamesToContacts ----

#[tokio::test]
async fn attach_by_names_returns_early_when_contact_ids_array_is_empty() {
    let db = fixture().await;
    let out = svc::attach_by_names_to_contacts(&db, WS, &[], &names(&["new"]))
        .await
        .expect("attach");
    assert_eq!(out, outcome(&[], &[]));
    assert_eq!(tag_count(&db).await, 2);
}

#[tokio::test]
async fn attach_by_names_returns_early_when_names_array_is_empty() {
    let db = fixture().await;
    let out = svc::attach_by_names_to_contacts(&db, WS, &[10], &[])
        .await
        .expect("attach");
    assert_eq!(out, outcome(&[], &[10]));
    assert!(linked_tag_ids(&db, 10).await.is_empty());
}

#[tokio::test]
async fn attach_by_names_returns_early_when_no_tags_found_zero_rows() {
    let db = fixture().await;
    let out = svc::attach_by_names_to_contacts(&db, WS, &[10], &names(&["   "]))
        .await
        .expect("attach");
    assert_eq!(out, outcome(&[], &[10]));
    assert_eq!(tag_count(&db).await, 2);
}

#[tokio::test]
async fn attach_by_names_continues_skips_chunk_when_contact_chunk_returns_empty() {
    let db = fixture().await;
    let out = svc::attach_by_names_to_contacts(&db, WS, &[20], &names(&["alpha"]))
        .await
        .expect("attach");
    assert_eq!(out, outcome(&[], &[20]));
    assert!(linked_tag_ids(&db, 20).await.is_empty());
}

#[tokio::test]
async fn attach_by_names_calls_enqueue_attach_for_each_newly_inserted_pair() {
    let db = fixture().await;
    let out = svc::attach_by_names_to_contacts(&db, WS, &[10], &names(&["alpha", "gamma"]))
        .await
        .expect("attach");
    assert_eq!(out, outcome(&[10], &[]));
    let gamma = svc::find_by_key_or_fail(&db, WS, "gamma")
        .await
        .expect("created")
        .id;
    let mut expected = vec![1, gamma];
    expected.sort_unstable();
    assert_eq!(linked_tag_ids(&db, 10).await, expected);
}

#[tokio::test]
async fn attach_by_names_processes_contacts_in_chunks_of_200() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        for id in 1..=201 {
            contact(tx, id, WS)?;
        }
        Ok(())
    })
    .await;
    let ids: Vec<i64> = (1..=201).collect();
    let out = svc::attach_by_names_to_contacts(&db, WS, &ids, &names(&["bulk"]))
        .await
        .expect("attach");
    assert_eq!(out.processed_contact_ids.len(), 201);
    let links = scalar_i64(
        &db,
        r#"SELECT count(*) FROM "ContactToTag" WHERE ?1 > 0"#,
        1,
    )
    .await;
    assert_eq!(links, Some(201));
}

#[tokio::test]
async fn attach_by_names_only_enqueues_attach_for_truly_new_pairs_mixed_returning() {
    let db = fixture().await;
    let out = svc::attach_by_names_to_contacts(&db, WS, &[10, 11], &names(&["alpha"]))
        .await
        .expect("attach");
    assert_eq!(out, outcome(&[10, 11], &[]));
    assert_eq!(linked_tag_ids(&db, 10).await, vec![1]);
    assert_eq!(linked_tag_ids(&db, 11).await, vec![1]);
}

// ---- detachByNamesFromContacts ----

#[tokio::test]
async fn detach_by_names_returns_early_when_contact_ids_array_is_empty() {
    let db = fixture().await;
    let out = svc::detach_by_names_from_contacts(&db, WS, &[], &names(&["alpha"]))
        .await
        .expect("detach");
    assert_eq!(out, outcome(&[], &[]));
    assert_eq!(linked_tag_ids(&db, 11).await, vec![1]);
}

#[tokio::test]
async fn detach_by_names_returns_early_when_names_array_is_empty() {
    let db = fixture().await;
    let out = svc::detach_by_names_from_contacts(&db, WS, &[11], &[])
        .await
        .expect("detach");
    assert_eq!(out, outcome(&[], &[11]));
    assert_eq!(linked_tag_ids(&db, 11).await, vec![1]);
}

#[tokio::test]
async fn detaches_nothing_but_still_reports_contacts_when_no_tag_name_matches() {
    let db = fixture().await;
    let out = svc::detach_by_names_from_contacts(&db, WS, &[11, 999], &names(&["ghost"]))
        .await
        .expect("detach");
    assert_eq!(out, outcome(&[11], &[999]));
    assert_eq!(linked_tag_ids(&db, 11).await, vec![1]);
    assert_eq!(tag_count(&db).await, 2);
}

#[tokio::test]
async fn detach_by_names_skips_chunk_when_no_contacts_found_in_chunk() {
    let db = fixture().await;
    let out = svc::detach_by_names_from_contacts(&db, WS, &[20], &names(&["alpha"]))
        .await
        .expect("detach");
    assert_eq!(out, outcome(&[], &[20]));
}

#[tokio::test]
async fn deletes_and_enqueues_detach_for_each_contact_x_tag_pair() {
    let db = fixture().await;
    setup(&db, |tx| {
        tag_link(tx, 10, 1)?;
        tag_link(tx, 10, 2)?;
        tag_link(tx, 11, 2)
    })
    .await;
    let out = svc::detach_by_names_from_contacts(&db, WS, &[10, 11], &names(&["alpha", "beta"]))
        .await
        .expect("detach");
    assert_eq!(out, outcome(&[10, 11], &[]));
    assert!(linked_tag_ids(&db, 10).await.is_empty());
    assert!(linked_tag_ids(&db, 11).await.is_empty());
}

// ---- replaceContactTagsByNames ----

#[tokio::test]
async fn replace_throws_when_contact_not_found_find_by_id_or_fail_rejects() {
    let db = fixture().await;
    let error = svc::replace_contact_tags_by_names(&db, WS, 20, &names(&["alpha"]))
        .await
        .expect_err("foreign contact");
    assert_eq!(error.code(), Some("notFound"));
}

#[tokio::test]
async fn clears_all_tags_only_enqueues_detach_for_each_previously_set_tag() {
    let db = fixture().await;
    let tags = svc::replace_contact_tags_by_names(&db, WS, 11, &[])
        .await
        .expect("replace");
    assert!(tags.is_empty());
    assert!(linked_tag_ids(&db, 11).await.is_empty());
    assert_eq!(tag_count(&db).await, 2);
}

#[tokio::test]
async fn purely_additive_only_enqueues_attach_for_new_tags() {
    let db = fixture().await;
    svc::replace_contact_tags_by_names(&db, WS, 11, &names(&["alpha", "beta"]))
        .await
        .expect("replace");
    assert_eq!(linked_tag_ids(&db, 11).await, vec![1, 2]);
}

#[tokio::test]
async fn purely_subtractive_only_enqueues_detach_for_removed_tags() {
    let db = fixture().await;
    setup(&db, |tx| tag_link(tx, 11, 2)).await;
    svc::replace_contact_tags_by_names(&db, WS, 11, &names(&["beta"]))
        .await
        .expect("replace");
    assert_eq!(linked_tag_ids(&db, 11).await, vec![2]);
}

#[tokio::test]
async fn mixed_update_enqueues_both_attach_for_new_and_detach_for_removed_tags() {
    let db = fixture().await;
    svc::replace_contact_tags_by_names(&db, WS, 11, &names(&["beta", "delta"]))
        .await
        .expect("replace");
    let delta = svc::find_by_key_or_fail(&db, WS, "delta")
        .await
        .expect("created")
        .id;
    let mut expected = vec![2, delta];
    expected.sort_unstable();
    assert_eq!(linked_tag_ids(&db, 11).await, expected);
}

#[tokio::test]
async fn replace_returns_the_resolved_tag_list() {
    let db = fixture().await;
    let tags = svc::replace_contact_tags_by_names(&db, WS, 10, &names(&["alpha"]))
        .await
        .expect("replace");
    assert_eq!(tags.len(), 1);
    assert_eq!(
        (tags[0].id, tags[0].name.as_str(), tags[0].workspace_id),
        (1, "alpha", WS)
    );
}
