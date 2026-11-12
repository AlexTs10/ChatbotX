//! Port of `packages/decision-config/__tests__/profile.test.ts`.
//! `globalRouterConfigSchema.safeParse(x).success` becomes
//! `parse_global_router_config(&x).is_ok()`.

use chatbotx_core::decision_config::parse_global_router_config;
use serde_json::{Value, json};

fn fallback() -> Value {
    json!({ "id": "1", "type": "flow" })
}

/// Port of: packages/decision-config/__tests__/profile.test.ts:7 "globalRouterConfigSchema > accepts tagged destinations and Noul defaults"
#[test]
fn globalrouterconfigschema_accepts_tagged_destinations_and_noul_defaults() {
    assert!(parse_global_router_config(&json!({ "fallback": fallback() })).is_ok());
}

/// Port of: packages/decision-config/__tests__/profile.test.ts:11 "globalRouterConfigSchema > rejects overlapping score routes and invalid Noul bands"
#[test]
fn globalrouterconfigschema_rejects_overlapping_score_routes_and_invalid_noul_bands() {
    assert!(
        parse_global_router_config(&json!({
            "fallback": fallback(),
            "noul": { "falseMaximum": 0.8, "trueMinimum": 0.2 },
        }))
        .is_err()
    );
    assert!(
        parse_global_router_config(&json!({
            "fallback": fallback(),
            "routes": [
                { "destination": fallback(), "from": 1, "to": 2, "type": "score" },
                { "destination": fallback(), "from": 2, "to": 3, "type": "score" },
            ],
        }))
        .is_err()
    );
}

/// Port of: packages/decision-config/__tests__/profile.test.ts:29 "globalRouterConfigSchema > rejects conflicting Choice/Noul routes and result mappings"
#[test]
fn globalrouterconfigschema_rejects_conflicting_choice_noul_routes_and_result_mappings() {
    assert!(
        parse_global_router_config(&json!({
            "fallback": fallback(),
            "routes": [
                { "destination": fallback(), "type": "choice", "value": "buy" },
                { "destination": fallback(), "type": "choice", "value": "buy" },
            ],
        }))
        .is_err()
    );
    assert!(
        parse_global_router_config(&json!({
            "fallback": fallback(),
            "routes": [
                { "destination": fallback(), "type": "noul", "value": true },
                { "destination": fallback(), "type": "noul", "value": true },
            ],
        }))
        .is_err()
    );
    assert!(
        parse_global_router_config(&json!({
            "fallback": fallback(),
            "metadataMappings": [
                { "customFieldId": "1", "customFieldType": "number", "value": "confidence" },
                { "customFieldId": "2", "customFieldType": "number", "value": "confidence" },
            ],
        }))
        .is_err()
    );
}
