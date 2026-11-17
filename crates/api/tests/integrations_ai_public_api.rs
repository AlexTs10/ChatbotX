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

// ---- openai-compatible-actions.test.ts (update) and the update action schema ----

async fn seed_openai_compatible(f: &handlers_common::Fixture) {
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (80, ?1, 'openaiCompatible')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationOpenaiCompatible" ("id", "auth", "baseURL", "defaultModel", "integrationId", "name", "preset", "workspaceId")
                   VALUES (81, '{"authType":"secretText","secretText":"existing-key"}', 'http://127.0.0.1:1234/v1', 'model-1', 80, 'Provider', 'nim', ?1)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

async fn openai_compatible_row(f: &handlers_common::Fixture) -> (String, String, bool) {
    f.state
        .db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT "auth", "preset", "enabled" FROM "IntegrationOpenaiCompatible" WHERE "id" = 81"#,
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .await
        .expect("read")
}

async fn openai_compatible_action(
    f: &handlers_common::Fixture,
    path: &str,
    cloud: bool,
    input: serde_json::Value,
) -> Result<serde_json::Value, chatbotx_api::orpc::OrpcError> {
    let mut context = chatbotx_api::rpc_ai::RpcAiContext::new(WS, 1001);
    context.cloud = cloud;
    let input = json!({ "args": [WS.to_string(), "81", input] });
    chatbotx_api::rpc_ai::call(&f.state, path, &context, &input)
        .await
        .expect("ported")
        .map(|out| out.json)
}

#[tokio::test]
async fn update_validates_existing_base_url_before_verifying_an_api_key_change() {
    let f = fixture().await;
    seed_openai_compatible(&f).await;
    let error = openai_compatible_action(
        &f,
        "actions.updateOpenaiCompatibleAction",
        true,
        json!({ "apiKey": "new-key" }),
    )
    .await
    .expect_err("blocked base URL");
    assert_eq!(
        error.data,
        Some(
            json!({ "validationErrors": { "baseURL": { "_errors": ["Base URL is invalid or not allowed."] } } })
        )
    );
    let (auth, preset, _) = openai_compatible_row(&f).await;
    assert!(auth.contains("existing-key"), "not updated: {auth}");
    assert_eq!(preset, "nim");
}

#[tokio::test]
async fn update_rejects_empty_and_non_http_base_urls() {
    let f = fixture().await;
    seed_openai_compatible(&f).await;
    for base_url in ["", "ftp://example.com"] {
        let error = openai_compatible_action(
            &f,
            "actions.updateOpenaiCompatibleAction",
            false,
            json!({ "baseURL": base_url }),
        )
        .await
        .expect_err("schema");
        let errors = &error.data.expect("data")["validationErrors"];
        assert!(errors["baseURL"]["_errors"][0].is_string(), "{errors}");
    }
}

#[tokio::test]
async fn enabled_action_switches_only_enabled() {
    let f = fixture().await;
    seed_openai_compatible(&f).await;
    let out = openai_compatible_action(
        &f,
        "actions.updateOpenaiCompatibleEnabledAction",
        false,
        json!({ "enabled": false }),
    )
    .await
    .expect("update");
    assert_eq!(out, serde_json::Value::Null);
    let (auth, preset, enabled) = openai_compatible_row(&f).await;
    assert_eq!((preset.as_str(), enabled), ("nim", false));
    assert!(auth.contains("existing-key"));
}
