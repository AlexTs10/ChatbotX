//! Q.D4 slice 1 ports of `apps/builder/__tests__/templates-public-*.test.ts`
//! (read-permissions: list paging, getLink / updateShare / rotateShare;
//! hardening: installation list projection and paging; scope: the list
//! carries no snapshot or selection). Adapted: real rows instead of mocked
//! services; the token permission comes from `Principal.read_only`.

mod handlers_common;

use chatbotx_api::handlers::{Call, Principal, dispatch_as};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, fixture};
use serde_json::{Value, json};

async fn run(
    fixture: &Fixture,
    route: &str,
    read_only: bool,
    params: &[(&'static str, &str)],
    query: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let params: Vec<(&'static str, String)> = params.iter().map(|(k, v)| (*k, (*v).to_string())).collect();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let principal = Principal {
        read_only,
        ..Principal::default()
    };
    let response = dispatch_as(
        &fixture.state,
        route,
        Call {
            workspace_id: WS,
            params: &params,
            query,
            body: body.as_bytes(),
        },
        &principal,
    )
    .await
    .expect("ported route");
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.expect("body");
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json body")
    };
    (status, json)
}

async fn seed(fixture: &Fixture) {
    fixture
        .state
        .db
        .write(|tx| {
            for (id, created) in [(101_i64, 1_000_i64), (102, 2_000)] {
                tx.execute(
                    r#"INSERT INTO "Template" ("id", "createdAt", "workspaceId", "tenantId", "name",
                         "selection", "payload", "categoryCounts", "formatVersion", "shareToken",
                         "defaultPermissions")
                       VALUES (?1, ?2, ?3, (SELECT "tenantId" FROM "Workspace" WHERE "id" = ?3),
                         'Welcome', '{"flows":{"mode":"all"}}', '{"secret":true}', '{"flows":2}', 1,
                         ?4, '{}')"#,
                    params![id, created, WS, format!("tok-{id}")],
                )?;
            }
            tx.execute(
                r#"INSERT INTO "TemplateInstallation" ("id", "workspaceId", "templateId", "templateName",
                     "sourceWorkspaceId", "formatVersion", "status", "permissions", "sourceTemplateId",
                     "errorCode", "errorMessage", "claimToken", "warnings")
                   VALUES (201, ?1, 101, 'Welcome', ?2, 1, 'failed', '{"secret":"claim-secret"}', 101,
                     'templateInstallFailed', 'secret-internal-detail', 'claim-secret',
                     '[{"category":"flows","entityKind":"name","path":"p","value":"v"},
                       {"category":"flows","entityKind":"internalOnly","path":"p","value":"v"}]')"#,
                params![WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "TemplateInstallation" ("id", "workspaceId", "templateId", "templateName",
                     "sourceWorkspaceId", "formatVersion", "status", "permissions", "sourceTemplateId",
                     "successfulInstallEvents")
                   VALUES (301, ?1, 101, 'Welcome', ?1, 1, 'completed', '{}', 101, 3)"#,
                params![OTHER_WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
}

// --- templates.list ---------------------------------------------------------

#[tokio::test]
async fn templates_list_pages_with_the_cursor_and_drops_snapshot_and_selection() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (status, first) = run(&fixture, "templates.list", true, &[], Some("limit=1"), None).await;
    assert_eq!(status, 200);
    assert_eq!(first["items"][0]["id"], "102");
    assert!(first["items"][0].get("payload").is_none());
    assert!(first["items"][0].get("selection").is_none());
    assert!(first["items"][0].get("shareToken").is_none());
    let cursor = first["nextCursor"].as_str().expect("cursor").to_string();
    let query = format!("limit=1&cursor={cursor}");
    let (_, second) = run(&fixture, "templates.list", true, &[], Some(&query), None).await;
    assert_eq!(second["items"][0]["id"], "101");
    assert_eq!(second["items"][0]["installCount"], 3);
    assert_eq!(second["items"][0]["categoryCounts"], json!({ "flows": 2 }));
    assert_eq!(second["nextCursor"], Value::Null);
}

#[tokio::test]
async fn templates_list_rejects_a_limit_above_the_maximum() {
    let fixture = fixture().await;
    let (status, body) = run(&fixture, "templates.list", false, &[], Some("limit=101"), None).await;
    assert!(status >= 400, "{body}");
}

#[tokio::test]
async fn templates_get_returns_the_selection_and_404_for_another_workspace() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (status, body) = run(&fixture, "templates.get", true, &[("id", "101")], None, None).await;
    assert_eq!(status, 200);
    assert_eq!(body["selection"], json!({ "flows": { "mode": "all" } }));
    assert!(body.get("payload").is_none());
    let (status, _) = run(&fixture, "templates.get", true, &[("id", "999")], None, None).await;
    assert_eq!(status, 404);
}

// --- share link -------------------------------------------------------------

#[tokio::test]
async fn get_link_refuses_a_read_only_token() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (status, body) = run(&fixture, "templates.getLink", true, &[("id", "101")], None, None).await;
    assert_eq!((status, body["code"].as_str()), (403, Some("templateManagePermissionDenied")));
}

#[tokio::test]
async fn get_link_hands_a_full_token_the_share_url() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (status, body) = run(&fixture, "templates.getLink", false, &[("id", "101")], None, None).await;
    assert_eq!(status, 200);
    assert_eq!(body["url"], "http://app/t/template?id=101&key=tok-101");
    assert_eq!((body["templateId"].as_str(), body["templateKey"].as_str()), (Some("101"), Some("tok-101")));
}

#[tokio::test]
async fn update_share_and_rotate_share_return_the_id_and_key_too() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (status, shared) = run(
        &fixture,
        "templates.updateShare",
        false,
        &[("id", "101")],
        None,
        Some(json!({ "enabled": true, "expiresAt": "2030-01-01T00:00:00.000Z" })),
    )
    .await;
    assert_eq!(status, 200, "{shared}");
    assert_eq!((shared["templateId"].as_str(), shared["templateKey"].as_str()), (Some("101"), Some("tok-101")));
    assert_eq!(shared["shareExpiresAt"], "2030-01-01T00:00:00.000Z");
    let (_, rotated) = run(&fixture, "templates.rotateShare", false, &[("id", "101")], None, None).await;
    let key = rotated["templateKey"].as_str().expect("key");
    assert_ne!(key, "tok-101");
    assert_eq!(rotated["url"], format!("http://app/t/template?id=101&key={key}"));
    // Omitted `expiresAt` keeps the expiry; `null` removes it.
    let (_, kept) = run(&fixture, "templates.updateShare", false, &[("id", "101")], None, Some(json!({ "enabled": false }))).await;
    assert_eq!(kept["shareExpiresAt"], "2030-01-01T00:00:00.000Z");
    let (_, cleared) = run(
        &fixture,
        "templates.updateShare",
        false,
        &[("id", "101")],
        None,
        Some(json!({ "enabled": false, "expiresAt": null })),
    )
    .await;
    assert_eq!(cleared["shareExpiresAt"], Value::Null);
}

#[tokio::test]
async fn delete_soft_deletes_and_stamps_installations() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (status, _) = run(&fixture, "templates.delete", false, &[("id", "101")], None, None).await;
    assert_eq!(status, 204);
    let (status, _) = run(&fixture, "templates.get", false, &[("id", "101")], None, None).await;
    assert_eq!(status, 404);
    let (status, _) = run(&fixture, "templates.delete", false, &[("id", "101")], None, None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn locked_resources_validates_and_reads_the_page() {
    let fixture = fixture().await;
    let (status, body) = run(
        &fixture,
        "templates.listLockedResources",
        true,
        &[],
        None,
        Some(json!({ "resourceKind": "flow", "resourceIds": ["1", "2"] })),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body, json!({ "lockedIds": [], "undeletableIds": [] }));
    let (status, _) = run(
        &fixture,
        "templates.listLockedResources",
        true,
        &[],
        None,
        Some(json!({ "resourceKind": "nope", "resourceIds": ["1"] })),
    )
    .await;
    assert!(status >= 400);
}

// --- installations ----------------------------------------------------------

#[tokio::test]
async fn the_installation_list_drops_error_message_permissions_and_claim_fields() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (status, result) = run(&fixture, "templateInstallations.list", true, &[], None, None).await;
    assert_eq!(status, 200);
    let serialized = result.to_string();
    assert!(!serialized.contains("secret-internal-detail"));
    assert!(!serialized.contains("claim-secret"));
    let item = &result["items"][0];
    assert!(item.get("errorMessage").is_none() && item.get("permissions").is_none());
    assert_eq!(item["errorCode"], "templateInstallFailed");
    assert_eq!(item["updateAvailable"], false);
    assert_eq!(item["uninstalledBy"], Value::Null);
    assert_eq!(item["warningCount"], 0);
    assert_eq!(item["installerWarningCount"], 1);
    assert_eq!(result["nextCursor"], Value::Null);
}

#[tokio::test]
async fn installation_paging_forwards_the_cursor_and_rejects_bad_sizes() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (_, result) = run(&fixture, "templateInstallations.list", true, &[], Some("cursor=201&limit=100"), None).await;
    assert_eq!(result, json!({ "items": [], "nextCursor": null }));
    for limit in ["0", "101"] {
        let query = format!("limit={limit}");
        let (status, body) = run(&fixture, "templateInstallations.list", true, &[], Some(&query), None).await;
        assert_eq!(body["code"], "invalidRequestData", "{status}");
    }
}

#[tokio::test]
async fn installation_update_and_syncs_check_the_installation() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (status, body) = run(
        &fixture,
        "templateInstallations.update",
        false,
        &[("id", "201")],
        None,
        Some(json!({ "autoUpdate": true })),
    )
    .await;
    assert_eq!((status, body["code"].as_str()), (409, Some("templateInstallationNotSyncable")));
    let (status, body) = run(&fixture, "templateInstallations.listSyncs", true, &[("id", "201")], None, None).await;
    assert_eq!((status, body), (200, json!({ "items": [] })));
    let (status, _) = run(
        &fixture,
        "templateInstallations.getSync",
        true,
        &[("id", "201"), ("syncId", "5")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = run(&fixture, "templateInstallations.listSyncs", true, &[("id", "301")], None, None).await;
    assert_eq!(status, 404);
}
