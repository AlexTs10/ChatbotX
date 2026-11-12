//! Port of `packages/flow-config/__tests__/klaviyo-sync-profile.test.ts`.

mod flow_config_support;
use flow_config_support::*;

#[test]
fn klaviyo_sync_profile_flow_contract_creates_safe_defaults_with_stable_state_ids() {
    let value = fc::klaviyo_sync_profile_default_fn();
    assert_match_object(
        &value,
        &json!({ "stepType": "klaviyoSyncProfile", "emailField": "email", "mergeFields": [] }),
    );
    let states = value["states"].as_array().unwrap();
    let types: Vec<&Value> = states.iter().map(|s| &s["stateType"]).collect();
    assert_eq!(types, vec!["success", "error"]);
    assert!(states.iter().all(|s| truthy(&s["id"])));
}

#[test]
fn klaviyo_sync_profile_flow_contract_trims_optional_selectors() {
    let value = fc::klaviyo_sync_profile_schema()
        .parse(&spread(
            &fc::klaviyo_sync_profile_default_fn(),
            json!({ "listId": " list-1 ", "titleField": " title ", "orgField": " org " }),
        ))
        .unwrap();
    assert_match_object(
        &value,
        &json!({ "listId": "list-1", "titleField": "title", "orgField": "org" }),
    );
}

#[test]
fn klaviyo_sync_profile_flow_contract_rejects_duplicate_properties() {
    let defaults = fc::klaviyo_sync_profile_default_fn();
    assert!(
        fc::klaviyo_sync_profile_schema()
            .parse(&spread(
                &defaults,
                json!({
                    "mergeFields": [
                        { "contactFieldId": "a", "klaviyoProperty": "plan" },
                        { "contactFieldId": "b", "klaviyoProperty": "plan" },
                    ],
                }),
            ))
            .is_err()
    );
}

#[test]
fn klaviyo_sync_profile_flow_contract_drops_legacy_tag_ids() {
    let parsed = fc::klaviyo_sync_profile_schema()
        .parse(&spread(
            &fc::klaviyo_sync_profile_default_fn(),
            json!({ "tagIds": ["legacy-tag"] }),
        ))
        .unwrap();
    assert!(parsed.get("tagIds").is_none());
}

#[test]
fn klaviyo_sync_profile_flow_contract_is_registered_in_the_shared_action_union() {
    let defaults = fc::klaviyo_sync_profile_default_fn();
    assert!(any_ok(&fc::action_steps(), &defaults));
}
