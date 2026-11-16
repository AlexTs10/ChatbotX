//! Rust-native tests of `integrations.getAiProvider` / `updateAiProvider`
//! (Q.D6; the TS router has no unit test, parity p1-10 covers the routes).

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::json;

#[tokio::test]
async fn get_hides_the_key_and_patch_switches_auto_reply() {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (70, ?1, 'claude')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationClaude" ("id", "auth", "workspaceId", "integrationId", "maxOutputTokens", "model", "temperature")
                   VALUES (71, '{"authType":"secretText","secretText":"sk-secret"}', ?1, 70, 1000, 'claude-sonnet-4-5', 0.5)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    let params = [("provider", "claude")];
    let (status, body) = call(&f, "integrations.getAiProvider", WS, &params, None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "id": "71", "model": "claude-sonnet-4-5", "temperature": 0.5, "maxOutputTokens": 1000, "autoReply": false, "hasApiKey": true })
    );
    assert!(!body.to_string().contains("sk-secret"));
    let (status, body) = call(
        &f,
        "integrations.updateAiProvider",
        WS,
        &params,
        None,
        Some(json!({ "autoReply": true })),
    )
    .await;
    assert_eq!(
        (status, body["autoReply"].clone()),
        (200, json!(true)),
        "{body}"
    );
    let (status, body) = call(
        &f,
        "integrations.getAiProvider",
        OTHER_WS,
        &params,
        None,
        None,
    )
    .await;
    assert_eq!(
        (status, body["message"].clone()),
        (404, json!("claude integration not found"))
    );
    let (status, _) = call(
        &f,
        "integrations.getAiProvider",
        WS,
        &[("provider", "openrouter")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 422);
}
