//! Ported from `apps/builder/__tests__/workspace-settings-public-api.test.ts`.
//! The TS test mocks the service; here the handlers run against a migrated
//! SQLite file (`handlers_common`), and the scope test reads the route table.

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use handlers_common::{WS, call, fixture};
use serde_json::json;

async fn settings(fx: &handlers_common::Fixture, sql: &'static str) {
    fx.state
        .db
        .write(move |tx| {
            tx.execute(sql, [WS])?;
            Ok(())
        })
        .await
        .expect("seed settings");
}

#[test]
fn uses_the_dedicated_settings_scope() {
    for id in ["workspaceSettings.get", "workspaceSettings.update"] {
        let route = route_by_id(id).expect("route");
        assert!(
            matches!(route.auth, RouteAuth::WorkspaceToken(&["settings"])),
            "{id}"
        );
    }
}

#[tokio::test]
async fn get_returns_only_the_settings_fields_of_the_tokens_workspace() {
    let fx = fixture().await;
    settings(
        &fx,
        r#"UPDATE "Workspace" SET "smartResponseDelaySeconds" = 10, "targetCountry" = 'VN', "language" = 'vi', "timezone" = 'Asia/Ho_Chi_Minh' WHERE "id" = ?1"#,
    )
    .await;
    let (status, body) = call(&fx, "workspaceSettings.get", WS, &[], None, None).await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        json!({
            "defaultReply": null,
            "defaultReplyFrequency": "allTime",
            "smartResponseDelaySeconds": 10,
            "capiLimitedDataUse": false,
            "logo": null,
            "targetCountry": "VN",
            "language": "vi",
            "timezone": "Asia/Ho_Chi_Minh",
            "brandColor": "#016DFF",
            "developmentMode": false,
        })
    );
}

#[tokio::test]
async fn get_resolves_a_stored_logo_path_to_a_usable_url() {
    let fx = fixture().await;
    settings(
        &fx,
        r#"UPDATE "Workspace" SET "logo" = 'public/space/ws-1/logos/a.jpg' WHERE "id" = ?1"#,
    )
    .await;
    let (status, body) = call(&fx, "workspaceSettings.get", WS, &[], None, None).await;
    assert_eq!(status, 200);
    // Adapted: the storage base is the tenant's (`<app>/storage/` here, not
    // the TS mock's `https://cdn.example.com`).
    let logo = body["logo"].as_str().expect("logo url");
    assert!(logo.starts_with("http"), "{logo}");
    assert!(logo.ends_with("/public/space/ws-1/logos/a.jpg"), "{logo}");
}

#[tokio::test]
async fn update_passes_the_input_to_update_settings_for_the_tokens_workspace() {
    let fx = fixture().await;
    let (status, body) = call(
        &fx,
        "workspaceSettings.update",
        WS,
        &[],
        None,
        Some(json!({ "capiLimitedDataUse": true })),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["capiLimitedDataUse"], json!(true));
    assert!(body.get("name").is_none());
    let stored = handlers_common::scalar(
        &fx,
        r#"SELECT "capiLimitedDataUse" FROM "Workspace" WHERE "id" = ?1"#,
        WS,
    )
    .await;
    assert_eq!(stored, Some(json!(1)));
}

async fn accepts(fx: &handlers_common::Fixture, body: serde_json::Value) -> bool {
    let (status, _) = call(fx, "workspaceSettings.update", WS, &[], None, Some(body)).await;
    assert!(status == 200 || status == 422, "status {status}");
    status == 200
}

#[tokio::test]
async fn the_request_accepts_only_valid_delays_frequencies_and_logo_urls() {
    let fx = fixture().await;
    assert!(accepts(&fx, json!({ "smartResponseDelaySeconds": 30 })).await);
    assert!(accepts(&fx, json!({ "smartResponseDelaySeconds": null })).await);
    assert!(!accepts(&fx, json!({ "smartResponseDelaySeconds": 7 })).await);
    assert!(!accepts(&fx, json!({ "defaultReplyFrequency": "sometimes" })).await);
    assert!(!accepts(&fx, json!({ "logo": "javascript:alert(1)" })).await);
    assert!(accepts(&fx, json!({ "logo": "https://cdn.example.com/l.png" })).await);
}

#[tokio::test]
async fn the_request_accepts_only_the_builders_country_language_and_timezone_values() {
    let fx = fixture().await;
    assert!(
        accepts(
            &fx,
            json!({
                "targetCountry": "VN",
                "language": "vi",
                "timezone": "Asia/Ho_Chi_Minh",
                "brandColor": "#112233",
                "developmentMode": true,
            })
        )
        .await
    );
    assert!(accepts(&fx, json!({ "targetCountry": "unknown" })).await);
    assert!(!accepts(&fx, json!({ "targetCountry": "XX" })).await);
    assert!(!accepts(&fx, json!({ "timezone": "Mars/Base" })).await);
    assert!(!accepts(&fx, json!({ "language": "xx" })).await);
    assert!(!accepts(&fx, json!({ "brandColor": "red" })).await);
}
