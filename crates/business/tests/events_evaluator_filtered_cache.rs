//! Port of `packages/events/__tests__/evaluator-filtered-cache.test.ts`
//! (P3.6-X1). The `describe.each` over the trigger and webhook caches runs
//! each case on both (one `ActiveCache` type, two condition sources); the
//! mocked `findMany` is a fake condition source; Redis was a no-op mock.

mod x1_support;

use std::sync::atomic::{AtomicU32, Ordering};

use chatbotx_business::events::cache::{ActiveCache, RAM_TTL};
use x1_support::FakeConditions;

static WORKSPACE_COUNTER: AtomicU32 = AtomicU32::new(0);

/// A fresh workspace id per case, so the in-memory cache never carries over.
fn next_workspace_id() -> String {
    format!(
        "ws-{}",
        WORKSPACE_COUNTER.fetch_add(1, Ordering::SeqCst) + 1
    )
}

/// The trigger and the webhook cache over the same conditions.
fn caches(conditions: &[(&str, Option<&str>)]) -> [(&'static str, ActiveCache); 2] {
    [
        (
            "trigger",
            ActiveCache::new(FakeConditions::new(conditions), RAM_TTL),
        ),
        (
            "webhook",
            ActiveCache::new(FakeConditions::new(conditions), RAM_TTL),
        ),
    ]
}

// %s cache — evaluator-filtered events > admits %s with a condition for %s, without an event sourceId
#[tokio::test]
async fn admits_evaluator_filtered_types_without_an_event_source_id() {
    let cases: [(&str, &str, Option<&str>); 8] = [
        ("shoppingCartSent", "any product", None),
        (
            "shoppingCartSent",
            "a specific product only",
            Some("product-1"),
        ),
        ("userAskedAboutProduct", "any product", None),
        (
            "userAskedAboutProduct",
            "a specific product only",
            Some("product-1"),
        ),
        (
            "productOrdered",
            "a specific product only",
            Some("product-1"),
        ),
        (
            "productAddedToCart",
            "a specific product only",
            Some("product-1"),
        ),
        (
            "productRemovedFromCart",
            "a specific product only",
            Some("product-1"),
        ),
        (
            "categoryAddedToCart",
            "a specific category only",
            Some("category-1"),
        ),
    ];
    for (event_type, label, source_id) in cases {
        for (name, cache) in caches(&[(event_type, source_id)]) {
            let admitted = cache
                .has_active(&next_workspace_id(), &[event_type], None)
                .await
                .expect("has_active");
            assert!(admitted, "{name} cache: {event_type} for {label}");
        }
    }
}

// %s cache — evaluator-filtered events > admits a mix of any-product and specific-product conditions
#[tokio::test]
async fn admits_a_mix_of_any_product_and_specific_product_conditions() {
    let conditions = [
        ("shoppingCartSent", Some("product-1")),
        ("shoppingCartSent", None),
    ];
    for (name, cache) in caches(&conditions) {
        let admitted = cache
            .has_active(&next_workspace_id(), &["shoppingCartSent"], None)
            .await
            .expect("has_active");
        assert!(admitted, "{name} cache");
    }
}

// %s cache — evaluator-filtered events > does not admit when no condition of that type exists
#[tokio::test]
async fn does_not_admit_when_no_condition_of_that_type_exists() {
    for (name, cache) in caches(&[("userAskedAboutProduct", None)]) {
        let admitted = cache
            .has_active(&next_workspace_id(), &["shoppingCartSent"], None)
            .await
            .expect("has_active");
        assert!(!admitted, "{name} cache");
    }
}

// %s cache — evaluator-filtered events > still requires a matching sourceId for events filtered by it
#[tokio::test]
async fn still_requires_a_matching_source_id_for_events_filtered_by_it() {
    for (name, cache) in caches(&[("tagApplied", Some("tag-1"))]) {
        let workspace_id = next_workspace_id();
        let check = |source: Option<&'static str>| {
            let cache = &cache;
            let workspace_id = workspace_id.clone();
            async move {
                cache
                    .has_active(&workspace_id, &["tagApplied"], source)
                    .await
                    .expect("has_active")
            }
        };
        assert!(!check(None).await, "{name} cache: no source");
        assert!(!check(Some("tag-2")).await, "{name} cache: tag-2");
        assert!(check(Some("tag-1")).await, "{name} cache: tag-1");
    }
}
