//! Port of `packages/business/__tests__/tag-service-detach-from-contact.test.ts`.
//! Adapted: the `findOrFail` mock is a contact of another workspace; the
//! delete spy is the stored links. The `emitTagRemoved` warning tests and
//! the tag-sync cleanup queue are n-a (no event bus, no tag-sync jobs); the
//! removed-pairs part is asserted on the returned ids.

mod common;

use chatbotx_business::tag as svc;
use common::{OTHER_WS, WS, contact, linked_tag_ids, setup, tag, tag_link, two_workspaces};

async fn fixture() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        contact(tx, 10, WS)?;
        contact(tx, 20, OTHER_WS)?;
        tag(tx, 1, WS, "a")?;
        tag(tx, 2, WS, "b")?;
        tag_link(tx, 10, 1)?;
        tag_link(tx, 20, 1)?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn throws_before_deleting_when_the_contact_is_out_of_scope() {
    let db = fixture().await;
    let error = svc::detach_from_contact(&db, WS, 20, &[1])
        .await
        .expect_err("out of scope");
    assert_eq!(error.message(), "Contact not found");
    assert_eq!(linked_tag_ids(&db, 20).await, vec![1]);
}

#[tokio::test]
async fn queues_channel_tag_cleanup_for_the_removed_tags_only() {
    let db = fixture().await;
    let removed = svc::detach_from_contact(&db, WS, 10, &[1, 2])
        .await
        .expect("detach");
    assert_eq!(removed, vec![1]);
    assert!(linked_tag_ids(&db, 10).await.is_empty());
}

#[tokio::test]
async fn does_not_queue_tag_cleanup_when_nothing_was_removed() {
    let db = fixture().await;
    let removed = svc::detach_from_contact(&db, WS, 10, &[2])
        .await
        .expect("detach");
    assert!(removed.is_empty());
    assert_eq!(linked_tag_ids(&db, 10).await, vec![1]);
}
