//! Port of `packages/business/__tests__/tag-service-bulk-attach.test.ts`
//! (P3.6-D2 slice contacts bulk). Adapted: the mocked tag / contact queries
//! and the `RETURNING` of the insert become stored rows; `emitTagApplied`
//! for the new pairs becomes `BulkAttachOutcome::newly_linked` (the route
//! emits them); "queues channel tag sync for newly attached pairs" becomes
//! the newly linked tag ids `attachToContact` returns.
//! Not applicable: "recovers already-inserted unsynced pairs on retry" (no
//! channel tag sync state is ported, so no pair is ever unsynced), the two
//! `contactInboxId` event cases of `attachToContact` (the event is emitted by
//! the route layer, which has no contact inbox in scope), the access-scope
//! argument, the cache invalidations and the ads-conversion enqueue.

mod common;

use chatbotx_business::tag::{BulkAttachOutcome, attach_to_contact, bulk_attach_to_contacts};
use common::{OTHER_WS, TestDb, WS, contact, linked_tag_ids, setup, tag, tag_link, two_workspaces};

/// Contacts 10 (`WS`, has tag 1) and 20 (`OTHER_WS`); tags 1, 2 (`WS`) and 9
/// (`OTHER_WS`).
async fn fixture() -> TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        contact(tx, 10, WS)?;
        contact(tx, 20, OTHER_WS)?;
        tag(tx, 1, WS, "one")?;
        tag(tx, 2, WS, "two")?;
        tag(tx, 9, OTHER_WS, "nine")?;
        tag_link(tx, 10, 1)?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn tag_service_bulk_attach_to_contacts_attaches_only_scoped_contacts_and_syncs_only_newly_inserted_pairs()
 {
    let db = fixture().await;
    // 10 twice (deduplicated), 20 is another workspace's contact.
    let outcome = bulk_attach_to_contacts(&db, WS, &[10, 10, 20], &[1, 2])
        .await
        .expect("bulk attach");
    assert_eq!(
        outcome,
        BulkAttachOutcome {
            attached_pair_count: 1,
            newly_linked: vec![(10, 2)],
        }
    );
    assert_eq!(linked_tag_ids(&db, 10).await, vec![1, 2]);
    assert!(linked_tag_ids(&db, 20).await.is_empty());
}

#[tokio::test]
async fn tag_service_bulk_attach_to_contacts_returns_without_writing_when_no_active_tag_belongs_to_the_workspace()
 {
    let db = fixture().await;
    let outcome = bulk_attach_to_contacts(&db, WS, &[10], &[9])
        .await
        .expect("bulk attach");
    assert_eq!(outcome, BulkAttachOutcome::default());
    assert_eq!(linked_tag_ids(&db, 10).await, vec![1]);
}

#[tokio::test]
async fn tag_service_attach_to_contact_queues_channel_tag_sync_for_newly_attached_pairs_only() {
    let db = fixture().await;
    let linked = attach_to_contact(&db, WS, 10, &[1, 2])
        .await
        .expect("attach");
    assert_eq!(linked, vec![2]);
}

#[tokio::test]
async fn tag_service_attach_to_contact_does_not_queue_tag_sync_when_every_pair_already_existed() {
    let db = fixture().await;
    let linked = attach_to_contact(&db, WS, 10, &[1]).await.expect("attach");
    assert!(linked.is_empty());
}
