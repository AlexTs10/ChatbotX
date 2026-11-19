//! Port of `integrations/google-ads/__tests__/scopes.test.ts`.

use chatbotx_business::google_ads_api::{
    ADS_SCOPE as ADS, DATA_MANAGER_SCOPE as DM, authorize_scopes, has_required_scopes,
    method_of_grant, missing_scopes, upload_method, upload_method_of,
};
use serde_json::{Value, json};

fn granted(scopes: &[&str]) -> Vec<String> {
    scopes.iter().map(|s| (*s).to_owned()).collect()
}

/// Port of: "requiredScopesFor" (both methods) + "the authorize scopes keep openid and email"
#[test]
fn required_and_authorize_scopes_per_method() {
    assert_eq!(missing_scopes(&[], "dataManager"), [ADS, DM]);
    assert_eq!(missing_scopes(&[], "legacy"), [ADS]);
    assert_eq!(
        authorize_scopes("dataManager"),
        format!("{ADS} {DM} openid email")
    );
    assert_eq!(authorize_scopes("legacy"), format!("{ADS} openid email"));
}

/// Port of: "missingRequiredScopes > defaults to Data Manager" (+ "legacy is satisfied by adwords alone")
#[test]
fn missing_required_scopes() {
    assert_eq!(missing_scopes(&granted(&[ADS]), "dataManager"), [DM]);
    assert!(missing_scopes(&granted(&[ADS]), "legacy").is_empty());
    assert_eq!(missing_scopes(&[], "legacy"), [ADS]);
}

/// Port of: "uploadMethodOf / parseUploadMethod > parses %s as %s" (+ "reads the method from auth metadata")
#[test]
fn parses_the_upload_method() {
    for (value, expected) in [
        (Value::Null, "dataManager"),
        (json!("soap"), "dataManager"),
        (json!("dataManager"), "dataManager"),
        (json!("legacy"), "legacy"),
    ] {
        assert_eq!(upload_method(&value), expected, "{value}");
    }
    assert_eq!(
        upload_method_of(&json!({ "metadata": { "uploadMethod": "legacy" } })),
        "legacy"
    );
    assert_eq!(upload_method_of(&json!({ "metadata": {} })), "dataManager");
    assert_eq!(upload_method_of(&json!({})), "dataManager");
}

/// Port of: "methodOfGrant > credential %s with grant %j is %s"
#[test]
fn method_of_grant_never_downgrades_data_manager() {
    assert_eq!(
        method_of_grant("dataManager", &granted(&[ADS, DM])),
        "dataManager"
    );
    assert_eq!(
        method_of_grant("dataManager", &granted(&[ADS])),
        "dataManager"
    );
    assert_eq!(
        method_of_grant("legacy", &granted(&[ADS, DM])),
        "dataManager"
    );
    assert_eq!(method_of_grant("legacy", &granted(&[ADS])), "legacy");
}

/// Port of: "hasRequiredScopes > a row without a recorded scope predates the check and passes" (+ "is judged by the connection's own method")
#[test]
fn has_required_scopes_by_the_connections_method() {
    assert!(has_required_scopes(&json!({ "metadata": {} })));
    let ads_only = format!("{ADS} openid email");
    assert!(!has_required_scopes(
        &json!({ "metadata": { "scope": ads_only } })
    ));
    assert!(has_required_scopes(
        &json!({ "metadata": { "scope": ads_only, "uploadMethod": "legacy" } })
    ));
}
