//! Ports of `apps/builder/__tests__/integrations-public-api.test.ts` (the
//! crud routes and `disconnectAiProvider`) and
//! `packages/business/src/integration/__tests__/find-token-refresh-errors.test.ts`.
//! Adapted (Q.D2c): the mocked services become a migrated database with
//! real rows; the procedures run through `handlers::dispatch`. The
//! `connectAiProvider` key check is covered by
//! `crates/business/tests/connection_credentials.rs` (`allowUpdate`) and
//! parity p1-10 `31`; here only the checks before the upstream call.

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

async fn seed(f: &Fixture, sql: &'static str) {
    f.state
        .db
        .write(move |tx| {
            tx.execute_batch(sql)?;
            Ok(())
        })
        .await
        .expect("seed");
}

async fn integration(f: &Fixture, id: i64, ws: i64, kind: &'static str) {
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (?1, ?2, ?3)"#,
                params![id, ws, kind],
            )?;
            Ok(())
        })
        .await
        .expect("seed integration");
}

// ts:158 describe("GET /v1/integrations")

#[tokio::test]
async fn get_v1_integrations_delegates_to_integration_service_list_by_workspace_id() {
    let f = fixture().await;
    integration(&f, 1, WS, "googleSheets").await;
    integration(&f, 2, OTHER_WS, "googleSheets").await;
    let (status, body) = call(
        &f,
        "integrations.list",
        WS,
        &[],
        Some("page=1&perPage=50"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["pageCount"], 1);
    let ids: Vec<&Value> = body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|i| &i["id"])
        .collect();
    assert_eq!(ids, vec![&json!("1")]);
}

#[tokio::test]
async fn list_hides_a_meta_catalog_row_without_a_live_catalog_and_pages_in_memory() {
    let f = fixture().await;
    for (id, kind) in [
        (1, "googleSheets"),
        (2, "mailchimp"),
        (3, "metaCatalog"),
        (4, "metaCatalog"),
    ] {
        integration(&f, id, WS, kind).await;
    }
    seed(
        &f,
        r#"INSERT INTO "IntegrationMetaCatalog" ("id", "workspaceId", "integrationId") VALUES (40, 2001, 4);"#,
    )
    .await;
    let (_, body) = call(&f, "integrations.list", WS, &[], None, None).await;
    let types: Vec<&str> = body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|i| i["integrationType"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(types, vec!["googleSheets", "mailchimp", "metaCatalog"]);
    let item = &body["data"][0];
    let mut keys: Vec<&String> = item.as_object().expect("object").keys().collect();
    keys.sort();
    assert_eq!(keys, ["createdAt", "id", "integrationType", "updatedAt"]);
    let (_, page2) = call(
        &f,
        "integrations.list",
        WS,
        &[],
        Some("page=2&perPage=2"),
        None,
    )
    .await;
    assert_eq!(page2["pageCount"], 2);
    assert_eq!(page2["data"].as_array().map(Vec::len), Some(1));
    let (status, _) = call(&f, "integrations.list", WS, &[], Some("perPage=51"), None).await;
    assert_eq!(status, 422);
}

// ts:178 describe("GET /v1/integrations/{id}")

#[tokio::test]
async fn get_v1_integrations_id_delegates_to_integration_service_find_by_id_for_workspace() {
    let f = fixture().await;
    integration(&f, 7, WS, "metaCatalog").await;
    let (status, body) = call(&f, "integrations.get", WS, &[("id", "7")], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], "7");
    assert_eq!(body["integrationType"], "metaCatalog");
}

#[tokio::test]
async fn get_v1_integrations_id_throws_not_found_when_the_integration_does_not_exist() {
    let f = fixture().await;
    integration(&f, 7, OTHER_WS, "googleSheets").await;
    for id in ["missing", "7"] {
        let (status, body) = call(&f, "integrations.get", WS, &[("id", id)], None, None).await;
        assert_eq!(status, 404, "{body}");
        assert_eq!(body["message"], "Integration not found");
    }
}

// ts:210 describe("GET /v1/integrations/status/token-errors") and
// find-token-refresh-errors.test.ts

const CHANNELS: &str = r#"
INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES
  (11, 'z', 'zalo', 'z', 2001), (12, 'ig', 'instagram', 'ig', 2001), (13, 'fb', 'instagram', 'fb', 2001),
  (14, 'm', 'messenger', 'm', 2001), (15, 't', 'tiktok', 't', 2001), (16, 'o', 'zalo', 'o', 2002),
  (17, 'th', 'threads', 'th', 2001), (18, 'ok', 'zalo', 'ok', 2001);
INSERT INTO "IntegrationZalo" ("id", "auth", "oaId", "name", "workspaceId", "inboxId", "tokenRefreshError") VALUES
  (21, '{}', 'oa1', 'Shop', 2001, 11, 'expired'), (22, '{}', 'oa2', 'Other', 2002, 16, 'x'),
  (23, '{}', 'oa3', 'Fine', 2001, 18, NULL);
INSERT INTO "IntegrationInstagram" ("id", "auth", "igId", "pageId", "name", "username", "conversationStarters", "persistentMenus", "workspaceId", "inboxId", "type", "tokenRefreshError") VALUES
  (31, '{}', 'i1', 'p1', 'IG', 'ig', '[]', '[]', 2001, 12, 'instagram', 'expired'),
  (32, '{}', 'i2', 'p2', 'FB IG', 'fb', '[]', '[]', 2001, 13, 'facebook', 'expired');
INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters", "persistentMenus", "personas", "workspaceId", "inboxId", "tokenRefreshError") VALUES
  (41, '{}', 'pg', 'My Page', '[]', '[]', '[]', 2001, 14, 'refresh failed');
INSERT INTO "IntegrationTiktok" ("id", "auth", "openId", "name", "workspaceId", "inboxId", "tokenRefreshError") VALUES
  (51, '{}', 'o1', 'T', 2001, 15, 'e2');
INSERT INTO "IntegrationThreads" ("id", "auth", "threadsUserId", "username", "name", "workspaceId", "inboxId", "tokenRefreshError") VALUES
  (61, '{}', 'u1', 'th', 'TH', 2001, 17, 'e4');
"#;

#[tokio::test]
async fn token_errors_reads_each_channel_satellite_s_own_token_refresh_error_with_its_own_id() {
    let f = fixture().await;
    seed(&f, CHANNELS).await;
    let (status, body) = call(&f, "integrations.tokenErrors", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "data": [
            { "id": "21", "channel": "zalo", "name": "Shop", "error": "expired" },
            { "id": "51", "channel": "tiktok", "name": "T", "error": "e2" },
            { "id": "31", "channel": "instagram", "name": "IG", "error": "expired" },
            { "id": "32", "channel": "instagramFacebook", "name": "FB IG", "error": "expired" },
            { "id": "41", "channel": "messenger", "name": "My Page", "error": "refresh failed" },
            { "id": "61", "channel": "threads", "name": "TH", "error": "e4" },
        ] })
    );
}

#[tokio::test]
async fn token_errors_returns_an_empty_list_when_no_satellite_row_has_a_token_refresh_error() {
    let f = fixture().await;
    let (status, body) = call(&f, "integrations.tokenErrors", WS, &[], None, None).await;
    assert_eq!((status, body), (200, json!({ "data": [] })));
}

// ts:337 describe("PUT /v1/integrations/ai/{provider}")

#[tokio::test]
async fn connect_validates_the_input_before_calling_the_provider() {
    let f = fixture().await;
    let base = json!({ "apiKey": "k", "model": "m", "temperature": 1, "maxOutputTokens": 10 });
    let cases: Vec<(&str, Value, &str)> = vec![
        ("openrouter", base.clone(), "provider"),
        (
            "claude",
            json!({ "model": "m", "temperature": 1, "maxOutputTokens": 10 }),
            "apiKey",
        ),
        (
            "claude",
            json!({ "apiKey": "k", "model": "", "temperature": 1, "maxOutputTokens": 10 }),
            "model",
        ),
        (
            "claude",
            json!({ "apiKey": "k", "model": "m", "temperature": 2.5, "maxOutputTokens": 10 }),
            "temperature",
        ),
        (
            "claude",
            json!({ "apiKey": "k", "model": "m", "temperature": 1, "maxOutputTokens": 8193 }),
            "maxOutputTokens",
        ),
        (
            "claude",
            json!({ "apiKey": "k", "model": "m", "temperature": 1, "maxOutputTokens": 1.5 }),
            "maxOutputTokens",
        ),
    ];
    for (provider, body, field) in cases {
        let (status, out) = call(
            &f,
            "integrations.connectAiProvider",
            WS,
            &[("provider", provider)],
            None,
            Some(body),
        )
        .await;
        assert_eq!(status, 422, "{out}");
        assert!(out.to_string().contains(field), "{field}: {out}");
    }
}

// ts:437 describe("DELETE /v1/integrations/ai/{provider}")

#[tokio::test]
async fn delete_resolves_the_connection_and_disconnects_through_connection_service() {
    let f = fixture().await;
    integration(&f, 70, WS, "deepseek").await;
    seed(
        &f,
        r#"INSERT INTO "IntegrationDeepseek" ("id", "auth", "workspaceId", "integrationId", "maxOutputTokens", "model")
             VALUES (71, '{"authType":"secretText","secretText":"k"}', 2001, 70, 1024, 'deepseek-flash');
           INSERT INTO "Connection" ("id", "workspaceId", "provider", "kind", "integrationId", "sourceId", "displayName", "status")
             VALUES (72, 2001, 'deepseek', 'integration', 70, 'workspace', 'DeepSeek', 'connected');"#,
    )
    .await;
    let params = [("provider", "deepseek")];
    let (status, _) = call(
        &f,
        "integrations.disconnectAiProvider",
        WS,
        &params,
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (status, body) = call(&f, "integrations.getAiProvider", WS, &params, None, None).await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], "deepseek integration not found");
}

#[tokio::test]
async fn delete_falls_back_to_the_legacy_per_provider_disconnect_when_no_connection_row_exists() {
    let f = fixture().await;
    integration(&f, 70, WS, "deepseek").await;
    seed(
        &f,
        r#"INSERT INTO "IntegrationDeepseek" ("id", "auth", "workspaceId", "integrationId", "maxOutputTokens", "model")
             VALUES (71, '{"authType":"secretText","secretText":"k"}', 2001, 70, 1024, 'deepseek-flash');"#,
    )
    .await;
    let params = [("provider", "deepseek")];
    let (status, _) = call(
        &f,
        "integrations.disconnectAiProvider",
        WS,
        &params,
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (status, _) = call(&f, "integrations.getAiProvider", WS, &params, None, None).await;
    assert_eq!(status, 404);
    let (_, list) = call(&f, "integrations.list", WS, &[], None, None).await;
    assert_eq!(list["data"], json!([]));
    // Nothing left: still 204 (idempotent).
    let (status, _) = call(
        &f,
        "integrations.disconnectAiProvider",
        WS,
        &params,
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (status, _) = call(
        &f,
        "integrations.disconnectAiProvider",
        WS,
        &[("provider", "foo")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 422);
}
