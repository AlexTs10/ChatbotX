//! Ports of `packages/business/__tests__/google-ads-transaction-id.test.ts`,
//! `google-ads-login-account.test.ts`, the `sendJobId` part of
//! `google-ads-send-queue.test.ts` and
//! `integrations/google-ads/__tests__/legacy-order-id.test.ts`.
//! Workspace ids are numbers in Rust (`1` for the original's `ws-1`).

use chatbotx_business::google_ads_conversion::{
    legacy_order_id, login_account_id, send_job_id, transaction_id,
};
use chatbotx_core::utils::crypto::sha256_hex;

const CLICK_ID: &str = "Cj0KCQjw-super-secret-click";
const CUSTOMER: &str = "7778889999";

fn sha(value: &str) -> String {
    sha256_hex(value)[..32].to_owned()
}

fn click(ws: i64, customer: &str, action: &str, click_id: &str) -> String {
    transaction_id(ws, customer, action, "c", click_id)
}

fn by_id(dedup: &str) -> String {
    transaction_id(1, CUSTOMER, "123", "i", dedup)
}

fn by_event(key: &str) -> String {
    transaction_id(1, CUSTOMER, "123", "e", key)
}

/// Port of: "buildTransactionId > click mode / id mode / event mode: gads-v2-{action}-{c|i|e}-{sha256(ns:key)[0..32]}"
#[test]
fn transaction_id_has_the_documented_shape_per_mode() {
    assert_eq!(
        click(1, CUSTOMER, "123", CLICK_ID),
        format!("gads-v2-123-c-{}", sha(&format!("1:{CUSTOMER}:{CLICK_ID}")))
    );
    assert_eq!(
        by_id("A-1042"),
        format!("gads-v2-123-i-{}", sha(&format!("1:{CUSTOMER}:A-1042")))
    );
    assert_eq!(
        by_event("flow:job-42:ci-1:step-1"),
        format!(
            "gads-v2-123-e-{}",
            sha(&format!("1:{CUSTOMER}:flow:job-42:ci-1:step-1"))
        )
    );
}

/// Port of: "event mode: one key is one identity, another key is another" (+ "event mode ignores the click")
#[test]
fn event_mode_is_keyed_by_the_occurrence_only() {
    assert_eq!(by_event("job-1"), by_event("job-1"));
    assert_ne!(by_event("job-1"), by_event("job-2"));
    assert_ne!(by_event("job-1"), click(1, CUSTOMER, "123", "job-1"));
}

/// Port of: "is at most 59 characters for a 16-digit action id, under Google's 64"
#[test]
fn transaction_id_fits_googles_limit() {
    let id = click(1, CUSTOMER, "1234567890123456", CLICK_ID);
    assert_eq!(id.len(), 8 + 16 + 3 + 32);
    assert!(id.len() <= 64);
}

/// Port of: "is deterministic" (+ "id mode ignores the click")
#[test]
fn transaction_id_is_deterministic() {
    assert_eq!(
        click(1, CUSTOMER, "123", CLICK_ID),
        click(1, CUSTOMER, "123", CLICK_ID)
    );
    assert_eq!(by_id("A-1"), by_id("A-1"));
}

/// Port of: "distinct IDs, actions, workspaces and accounts give distinct ids" (+ "click mode differs per click, action, workspace and account")
#[test]
fn transaction_id_differs_per_key_action_workspace_and_account() {
    let base = by_id("A-1");
    assert_ne!(by_id("A-2"), base);
    assert_ne!(transaction_id(1, CUSTOMER, "456", "i", "A-1"), base);
    assert_ne!(transaction_id(2, CUSTOMER, "123", "i", "A-1"), base);
    assert_ne!(transaction_id(1, "1", "123", "i", "A-1"), base);
    let base = click(1, CUSTOMER, "123", CLICK_ID);
    assert_ne!(click(1, CUSTOMER, "123", "other-click-1"), base);
    assert_ne!(click(1, CUSTOMER, "456", CLICK_ID), base);
    assert_ne!(click(2, CUSTOMER, "123", CLICK_ID), base);
    assert_ne!(click(1, "1", "123", CLICK_ID), base);
}

/// Port of: "the click and id spaces never collide for the same text"
#[test]
fn click_and_id_spaces_never_collide() {
    assert_ne!(
        click(1, CUSTOMER, "123", "same-text-1"),
        by_id("same-text-1")
    );
}

/// Port of: "never contains ':' and never leaks the click id or the business ID"
#[test]
fn transaction_id_never_leaks_its_key() {
    for id in [
        click(1, CUSTOMER, "123", CLICK_ID),
        by_id("order-secret-77"),
        by_event("flow:k:1"),
    ] {
        assert!(!id.contains(':'));
        assert!(!id.contains(CLICK_ID));
        assert!(!id.contains("order-secret-77"));
    }
}

/// Port of: "sendJobId > encodes event and generation without ':'"
#[test]
fn send_job_id_encodes_event_and_generation_without_colons() {
    assert_eq!(send_job_id(1, 3), "google-ads-send-1-a3");
    assert!(!send_job_id(1, 0).contains(':'));
}

/// Port of: "resolveLoginAccountId" (4 cases)
#[test]
fn login_account_id_follows_the_manager_route() {
    let (client, manager, owner) = ("1112223333", "4445556666", "7778889999");
    assert_eq!(login_account_id(client, None, client), client);
    assert_eq!(login_account_id(client, Some(manager), client), manager);
    assert_eq!(login_account_id(client, None, owner), owner);
    assert_eq!(login_account_id(client, Some(manager), owner), manager);
}

/// Port of: "legacyOrderId > is byte-identical to v1- + first 40 hex of node sha256 (%s)" (+ "pins a known vector")
#[test]
fn legacy_order_id_is_v1_and_forty_hex() {
    for id in ["tx-1", "workspace:42:contact:7:purchase", "ünï-çødé"] {
        assert_eq!(legacy_order_id(id), format!("v1-{}", &sha256_hex(id)[..40]));
    }
    assert_eq!(
        legacy_order_id("abc"),
        "v1-ba7816bf8f01cfea414140de5dae2223b00361a3"
    );
}
