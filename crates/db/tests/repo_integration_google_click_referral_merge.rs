//! Port of `packages/database/__tests__/integration/google-click-referral-merge.test.ts`
//! (P3.2b). The TS runs the merge on a real Postgres in a rolled-back transaction; here it
//! runs on a migrated temp file (ported 1:1): one contact inbox, its `referral` updated by
//! the merge expression and read back.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo10.rs"]
mod repo10;
#[path = "common/repo3.rs"]
mod repo3;

use repo10::{click_db, merge_referral, seed_referral};
use serde_json::{Value, json};

const CI: i64 = 3101;

fn click_a() -> Value {
    json!({
        "gclid": "gclid-A-1234567890",
        "gbraid": null,
        "googleCampaignId": "1",
        "googleAdGroupId": "2",
        "googleAdId": "3",
        "googleClickReceivedAt": "2026-10-01T10:00:00.000Z",
    })
}

fn click_b() -> Value {
    json!({
        "gclid": null,
        "gbraid": "gbraid-B-1234567890",
        "googleCampaignId": "10",
        "googleAdGroupId": "20",
        "googleAdId": "30",
        "googleClickReceivedAt": "2026-10-01T11:00:00.000Z",
    })
}

/// `toMatchObject`: every key of `expected` is in `actual` with the same value.
fn assert_matches(actual: &Value, expected: &Value) {
    for (key, value) in expected.as_object().expect("object") {
        assert_eq!(actual.get(key), Some(value), "key {key} of {actual}");
    }
}

fn with(base: Value, key: &str, value: Value) -> Value {
    let mut map = base.as_object().cloned().expect("object");
    map.insert(key.to_owned(), value);
    Value::Object(map)
}

#[tokio::test]
async fn newest_google_click_referral_merge_against_postgres_delivery_order_b_then_a_keeps_the_newer_click_b()
 {
    let db = click_db().await;
    merge_referral(&db, CI, click_b()).await;
    let referral = merge_referral(&db, CI, click_a()).await;
    assert_matches(&referral, &click_b());
}

#[tokio::test]
async fn newest_google_click_referral_merge_against_postgres_delivery_order_a_then_b_lets_the_newer_click_b_win_and_clears_gclid()
 {
    let db = click_db().await;
    merge_referral(&db, CI, click_a()).await;
    let referral = merge_referral(&db, CI, click_b()).await;
    assert_matches(&referral, &click_b());
    assert_eq!(referral.get("gclid"), Some(&Value::Null));
}

#[tokio::test]
async fn newest_google_click_referral_merge_against_postgres_the_same_timestamp_applies_the_incoming_click()
 {
    let db = click_db().await;
    merge_referral(&db, CI, click_a()).await;
    let same = with(
        click_b(),
        "googleClickReceivedAt",
        click_a()["googleClickReceivedAt"].clone(),
    );
    let referral = merge_referral(&db, CI, same.clone()).await;
    assert_matches(&referral, &same);
}

#[tokio::test]
async fn newest_google_click_referral_merge_against_postgres_no_stored_click_or_a_null_referral_applies_the_incoming_click()
 {
    let db = click_db().await;
    seed_referral(&db, CI, Some(json!({ "ctwaClid": "meta-1" }))).await;
    let referral = merge_referral(&db, CI, click_a()).await;
    assert_matches(&referral, &with(click_a(), "ctwaClid", json!("meta-1")));
    seed_referral(&db, CI, None).await;
    let referral = merge_referral(&db, CI, click_a()).await;
    assert_matches(&referral, &click_a());
}

#[tokio::test]
async fn newest_google_click_referral_merge_against_postgres_a_stale_click_still_merges_its_non_google_keys()
 {
    let db = click_db().await;
    merge_referral(&db, CI, click_b()).await;
    let referral = merge_referral(&db, CI, with(click_a(), "ctwaClid", json!("meta-2"))).await;
    assert_matches(&referral, &with(click_b(), "ctwaClid", json!("meta-2")));
}

#[tokio::test]
async fn newest_google_click_referral_merge_against_postgres_a_non_iso_garbage_stored_timestamp_is_treated_as_absent_and_overwritten()
 {
    let db = click_db().await;
    seed_referral(&db, CI, Some(json!({ "googleClickReceivedAt": "garbage" }))).await;
    let referral = merge_referral(&db, CI, click_a()).await;
    assert_matches(&referral, &click_a());
}

#[tokio::test]
async fn newest_google_click_referral_merge_against_postgres_an_iso_shaped_but_impossible_stored_timestamp_never_throws()
 {
    let db = click_db().await;
    seed_referral(
        &db,
        CI,
        Some(json!({ "googleClickReceivedAt": "2026-99-99T00:00:00.000Z" })),
    )
    .await;
    let referral = merge_referral(&db, CI, click_a()).await;
    assert!(referral.is_object());
}

#[tokio::test]
async fn newest_google_click_referral_merge_against_postgres_an_iso_shaped_garbage_value_that_is_not_fixed_width_is_treated_as_absent()
 {
    let db = click_db().await;
    seed_referral(
        &db,
        CI,
        Some(json!({ "googleClickReceivedAt": "2026-99-99Tgarbage" })),
    )
    .await;
    let referral = merge_referral(&db, CI, click_a()).await;
    assert_matches(&referral, &click_a());
}

#[tokio::test]
async fn newest_google_click_referral_merge_against_postgres_a_reversed_order_older_incoming_after_newer_stored_keeps_the_stored_click()
 {
    let db = click_db().await;
    seed_referral(&db, CI, Some(click_b())).await;
    let referral = merge_referral(&db, CI, click_a()).await;
    assert_matches(&referral, &click_b());
}
