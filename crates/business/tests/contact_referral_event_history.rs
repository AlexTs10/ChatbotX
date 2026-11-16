//! Port of `packages/business/__tests__/contact-referral-event-history.test.ts`
//! (P3.6-D2, slice inboxes / ads / refresh). Adapted: the mocked repository
//! and `findByIdOrFail` become real rows; a contact of another workspace is
//! the contact "the caller cannot open". The member access scope
//! (`restrictToAssignedUserId`) has no Rust form (no member context), so the
//! scope part of the first case is the workspace check only.

mod common;

use chatbotx_business::contact_referral_event as svc;
use chatbotx_db::repo::contact_referral_event::ContactAdTouchCursor;
use common::{OTHER_WS, WS, ad_referral, contact, contact_inbox, inbox, setup, two_workspaces};

const DAY: i64 = 86_400_000;
/// 2026-10-01T00:00:00Z.
const OCT_1: i64 = 1_790_812_800_000;

/// Contact 10 (WS, contact inbox 11) and contact 20 (OTHER_WS, contact
/// inbox 21); touches 7, 8, 9 of contact 10 on Oct 1, 2, 3, touch 30 of
/// contact 20.
async fn fixture() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        inbox(tx, 1, WS, "messenger")?;
        inbox(tx, 2, OTHER_WS, "messenger")?;
        contact(tx, 10, WS)?;
        contact(tx, 20, OTHER_WS)?;
        contact_inbox(tx, 11, 10, 1, "messenger", "psid-10")?;
        contact_inbox(tx, 21, 20, 2, "messenger", "psid-20")?;
        ad_referral(tx, 7, WS, 10, 11, OCT_1, Some("ad-7"), None)?;
        ad_referral(tx, 8, WS, 10, 11, OCT_1 + DAY, Some("ad-8"), None)?;
        ad_referral(tx, 9, WS, 10, 11, OCT_1 + 2 * DAY, Some("ad-9"), None)?;
        ad_referral(tx, 30, OTHER_WS, 20, 21, OCT_1, Some("ad-30"), None)?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn checks_the_contact_is_reachable_by_the_caller_before_reading_its_touches() {
    let db = fixture().await;
    let page = svc::list_contact_ad_touches(&db, WS, 10, 20, None)
        .await
        .expect("list");
    assert_eq!(
        page.data.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![9, 8, 7]
    );
}

#[tokio::test]
async fn does_not_read_the_touches_of_a_contact_the_caller_cannot_open() {
    let db = fixture().await;
    let error = svc::list_contact_ad_touches(&db, WS, 20, 20, None)
        .await
        .expect_err("contact of another workspace");
    assert_eq!(error.code(), Some("notFound"));
    assert_eq!(error.to_string(), "Contact not found");
}

#[tokio::test]
async fn returns_the_page_and_a_cursor_when_another_page_exists() {
    let db = fixture().await;
    let page = svc::list_contact_ad_touches(&db, WS, 10, 2, None)
        .await
        .expect("list");
    assert_eq!(
        page.data.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![9, 8]
    );
    assert_eq!(
        page.next_cursor,
        Some(ContactAdTouchCursor {
            occurred_at: OCT_1 + DAY,
            id: 8,
        })
    );
    let rest = svc::list_contact_ad_touches(&db, WS, 10, 2, page.next_cursor)
        .await
        .expect("second page");
    assert_eq!(rest.data.iter().map(|t| t.id).collect::<Vec<_>>(), vec![7]);
}

#[tokio::test]
async fn has_no_cursor_on_the_last_page() {
    let db = fixture().await;
    let cursor = ContactAdTouchCursor {
        occurred_at: OCT_1 + DAY,
        id: 8,
    };
    let page = svc::list_contact_ad_touches(&db, WS, 10, 2, Some(cursor))
        .await
        .expect("list");
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.next_cursor, None);
}
