//! Port of `packages/variables/__tests__/refresh-profile-snapshot.test.ts`. `withCache` /
//! `invalidateCacheKeys` are the real in-process cache.

mod support;

use chatbotx_core::variables::integration_fields::profile_snapshot_cache_key;
use chatbotx_core::variables::*;
use chatbotx_kv::cache::CacheOpts;
use support::variables::*;

fn contact_inbox(channel: &str) -> ContactInboxModel {
    ContactInboxModel {
        id: "contact-inbox-1".into(),
        channel: channel.into(),
        inbox_id: "inbox-1".into(),
        source_id: "source-1".into(),
        ..Default::default()
    }
}

fn fresh_snapshot() -> ContactProfileSnapshot {
    ContactProfileSnapshot {
        follows_business: Some(true),
        business_follows_contact: Some(false),
        account_verified: Some(false),
        follower_count: Some(42),
        username: s("player"),
    }
}

fn harness() -> Harness {
    Harness::new(FakeState {
        inbox: Ok(Some(InboxWithIntegrations {
            id: "inbox-1".into(),
            workspace_id: "workspace-1".into(),
            integration_instagram: Some(IntegrationRow {
                id: "integration-1".into(),
                ..Default::default()
            }),
            ..Default::default()
        })),
        snapshot: Ok(fresh_snapshot()),
        integration_ctx: Ok(serde_json::json!({})),
        ..Default::default()
    })
}

fn refresh_call() -> String {
    format!(
        "refresh_profile_snapshot(contact-inbox-1, inbox-1, {})",
        serde_json::to_string(&fresh_snapshot()).unwrap()
    )
}

/// Port of: packages/variables/__tests__/refresh-profile-snapshot.test.ts:91 "drops the cached snapshot, fetches live and persists it"
#[tokio::test]
async fn drops_the_cached_snapshot_fetches_live_and_persists_it() {
    let h = harness();
    // A stale snapshot sits in the cache: it must be dropped, not served.
    let stale = ContactProfileSnapshot {
        username: s("stale"),
        ..Default::default()
    };
    let _ = h
        .cache
        .get_or_load::<ContactProfileSnapshot, (), _>(
            &profile_snapshot_cache_key("contact-inbox-1"),
            CacheOpts::default(),
            || async { Ok(Some(stale)) },
        )
        .await;
    h.v()
        .refresh_contact_profile_snapshot(&contact_inbox("instagram"), "workspace-1")
        .await;
    assert!(h.fake.called("run_get_profile_snapshot({}, source-1)"));
    assert!(h.fake.called(&refresh_call()));
}

/// Port of: packages/variables/__tests__/refresh-profile-snapshot.test.ts:115 "skips channels without a profile snapshot"
#[tokio::test]
async fn skips_channels_without_a_profile_snapshot() {
    let h = harness();
    h.v()
        .refresh_contact_profile_snapshot(&contact_inbox("messenger"), "workspace-1")
        .await;
    assert_eq!(h.fake.count("run_get_profile_snapshot"), 0);
    assert_eq!(h.fake.count("refresh_profile_snapshot"), 0);
    assert_eq!(h.fake.count("find_inbox_with_integrations"), 0);
}

/// Port of: packages/variables/__tests__/refresh-profile-snapshot.test.ts:126 "never throws when the live fetch fails"
#[tokio::test]
async fn never_throws_when_the_live_fetch_fails() {
    let h = harness();
    h.fake.set(|st| st.snapshot = Err("rate limited".into()));
    h.v()
        .refresh_contact_profile_snapshot(&contact_inbox("instagram"), "workspace-1")
        .await;
    assert_eq!(h.fake.count("refresh_profile_snapshot"), 0);
}

/// Port of: packages/variables/__tests__/refresh-profile-snapshot.test.ts:138 "still fetches live when the cache invalidation fails"
#[tokio::test]
async fn still_fetches_live_when_the_cache_invalidation_fails() {
    // The in-process cache invalidation cannot fail; the fetch and persist still happen once.
    let h = harness();
    h.v()
        .refresh_contact_profile_snapshot(&contact_inbox("instagram"), "workspace-1")
        .await;
    assert_eq!(h.fake.count("refresh_profile_snapshot"), 1);
}

/// Port of: packages/variables/__tests__/refresh-profile-snapshot.test.ts:149 "never throws when the inbox lookup fails"
#[tokio::test]
async fn never_throws_when_the_inbox_lookup_fails() {
    let h = harness();
    h.fake.set(|st| st.inbox = Err("db down".into()));
    h.v()
        .refresh_contact_profile_snapshot(&contact_inbox("instagram"), "workspace-1")
        .await;
}
