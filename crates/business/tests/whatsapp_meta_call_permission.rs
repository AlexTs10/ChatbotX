//! Port of `apps/builder/__tests__/whatsapp-meta-call-permission.test.ts`
//! (the Meta call-permission read: status, expiry, action budget).
//! ponytail: the 60 s lookup cache is not ported, so its two cache cases have no Rust twin.
use chatbotx_business::whatsapp_voip::{
    can_perform_call_action, meta_permission_expiration, meta_permission_status,
};
use serde_json::json;

#[test]
fn a_successful_read_carries_the_permissions_through() {
    let response =
        json!({ "permission": { "status": "temporary", "expiration_time": 1_900_000_000 } });
    assert_eq!(meta_permission_status(&response), Some("temporary"));
    assert_eq!(meta_permission_expiration(&response), Some(1_900_000_000.0));
    assert_eq!(
        meta_permission_status(&json!({ "permission": { "status": "weird" } })),
        None
    );
}

#[test]
fn request_budget_is_read_off_metas_own_action_flag() {
    let response = json!({ "actions": [
        { "action_name": "send_call_permission_request", "can_perform_action": false },
        { "action_name": "start_call", "can_perform_action": true },
    ] });
    assert!(can_perform_call_action(&response, "start_call"));
    assert!(!can_perform_call_action(
        &response,
        "send_call_permission_request"
    ));
}

#[test]
fn an_action_meta_did_not_report_is_treated_as_no_budget() {
    assert!(!can_perform_call_action(
        &json!({ "actions": [] }),
        "start_call"
    ));
    assert!(!can_perform_call_action(&json!({}), "start_call"));
}
