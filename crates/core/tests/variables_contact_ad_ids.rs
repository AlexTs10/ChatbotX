//! Port of `packages/variables/__tests__/contact-ad-ids.test.ts`.

mod support;

use chatbotx_core::variables::system_field::CONTACT_AD_IDS_LIMIT;
use support::variables::*;

/// Port of: packages/variables/__tests__/contact-ad-ids.test.ts:22 "listContactAdIdsString > joins the contact's ad ids in the order the log returns them"
#[tokio::test]
async fn listcontactadidsstring_joins_the_contact_s_ad_ids_in_the_order_the_log_returns_them() {
    let h = Harness::new(FakeState {
        ad_ids: vec!["ad-1".into(), "ad-2".into(), "ad-3".into()],
        ..Default::default()
    });
    assert_eq!(
        h.v()
            .list_contact_ad_ids_string("contact-1", "workspace-1")
            .await
            .unwrap(),
        s("ad-1, ad-2, ad-3")
    );
}

/// Port of: packages/variables/__tests__/contact-ad-ids.test.ts:30 "listContactAdIdsString > is bounded so a heavy clicker cannot flood a message"
#[tokio::test]
async fn listcontactadidsstring_is_bounded_so_a_heavy_clicker_cannot_flood_a_message() {
    let h = Harness::new(FakeState::default());
    h.v()
        .list_contact_ad_ids_string("contact-1", "workspace-1")
        .await
        .unwrap();
    assert!(h.fake.called(&format!(
        "list_ad_ids(workspace-1, contact-1, {CONTACT_AD_IDS_LIMIT})"
    )));
}

/// Port of: packages/variables/__tests__/contact-ad-ids.test.ts:42 "listContactAdIdsString > is null for a contact that never came from an ad"
#[tokio::test]
async fn listcontactadidsstring_is_null_for_a_contact_that_never_came_from_an_ad() {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        h.v()
            .list_contact_ad_ids_string("contact-1", "workspace-1")
            .await
            .unwrap(),
        None
    );
}

/// Port of: packages/variables/__tests__/contact-ad-ids.test.ts:56 "getFirstContactAdId > asks the log for just the oldest ad"
#[tokio::test]
async fn getfirstcontactadid_asks_the_log_for_just_the_oldest_ad() {
    let h = Harness::new(FakeState {
        ad_ids: vec!["ad-1".into()],
        ..Default::default()
    });
    assert_eq!(
        h.v()
            .get_first_contact_ad_id("contact-1", "workspace-1")
            .await
            .unwrap(),
        s("ad-1")
    );
    assert!(h.fake.called("list_ad_ids(workspace-1, contact-1, 1)"));
}

/// Port of: packages/variables/__tests__/contact-ad-ids.test.ts:69 "getFirstContactAdId > is null for a contact that never came from an ad"
#[tokio::test]
async fn getfirstcontactadid_is_null_for_a_contact_that_never_came_from_an_ad() {
    let h = Harness::new(FakeState::default());
    assert_eq!(
        h.v()
            .get_first_contact_ad_id("contact-1", "workspace-1")
            .await
            .unwrap(),
        None
    );
}
