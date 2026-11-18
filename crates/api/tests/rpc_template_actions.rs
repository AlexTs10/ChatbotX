//! Q.D4 slice 6: the template actions (`/rpc/templateActions/<name>`) and
//! the default template actions (`/rpc/platformSettingsActions/<name>`) on a
//! migrated database, and the default install staged for a new workspace.

mod rpc_common;

use axum::http::StatusCode;
use chatbotx_db::rusqlite::params;
use rpc_common::{AGENT_TOKEN, OWNER, OWNER_TOKEN, WS, action, fixture, scalar};
use serde_json::{Value, json};

const KEY: &str = "templateActions";

/// Saves keyword 21 (fires flow 11) as a template; its id.
async fn saved_template(fx: &rpc_common::Fixture) -> String {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "name", "workspaceId") VALUES (11, 'Welcome', ?1)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "AutomatedResponse" ("id", "workspaceId", "type", "keywords", "status", "flowId")
                   VALUES (21, ?1, 'inbound', '["hi"]', 1, 11)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "PlatformAdmin" ("id", "userId") VALUES (501, ?1)"#,
                params![OWNER],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    let (status, out) = action(
        fx,
        KEY,
        "saveTemplate",
        OWNER_TOKEN,
        json!({
            "workspaceId": WS.to_string(),
            "name": "Starter",
            "selection": { "keywords": { "mode": "ids", "ids": ["21"] } },
            "defaultPermissions": { "allowEdit": true },
            "createInstallFolder": false,
            "defaultAutoUpdate": false,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    out["json"]["templateId"]
        .as_str()
        .expect("templateId")
        .to_string()
}

async fn default_template(fx: &rpc_common::Fixture, input: Value) -> (StatusCode, Value) {
    action(
        fx,
        "platformSettingsActions",
        "updateRootDefaultTemplate",
        OWNER_TOKEN,
        input,
    )
    .await
}

#[tokio::test]
async fn template_actions_run_the_routes_as_the_member() {
    let fx = fixture().await;
    let template = saved_template(&fx).await;
    let ws = WS.to_string();

    // The agent has no `flows`: the manage gate refuses.
    let (status, out) = action(
        &fx,
        KEY,
        "getShareSettings",
        AGENT_TOKEN,
        json!({ "workspaceId": ws, "id": template }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");

    let (status, out) = action(
        &fx,
        KEY,
        "getShareSettings",
        OWNER_TOKEN,
        json!({ "workspaceId": ws, "id": template }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let token = out["json"]["shareToken"].as_str().unwrap().to_string();
    assert_eq!(out["json"]["id"], json!(template), "{out}");

    // Sharing is off: the default is refused as an outcome, not an error.
    let key = json!({ "templateId": template, "templateKey": token });
    let (status, out) = default_template(&fx, key.clone()).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        out["json"],
        json!({ "status": "rejected", "reason": "shareDisabled" })
    );
    let (_, wrong) = default_template(
        &fx,
        json!({ "templateId": template, "templateKey": "nope" }),
    )
    .await;
    assert_eq!(wrong["json"]["reason"], "unavailable", "{wrong}");

    let (status, out) = action(
        &fx,
        KEY,
        "updateShareSettings",
        OWNER_TOKEN,
        json!({ "workspaceId": ws, "templateId": template, "shareEnabled": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["shareToken"], json!(token), "{out}");
    let (_, out) = default_template(&fx, key).await;
    assert_eq!(out["json"], json!({ "status": "saved" }), "{out}");

    // A new workspace of the root tenant gets the default as a pending install.
    let staged = fx
        .db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (202, 'New', ?1)"#,
                params![OWNER],
            )?;
            chatbotx_business::template::tenant_default::stage_install(tx, 202, 1)
                .map_err(|e| chatbotx_db::DbError::other(e.message()))
        })
        .await
        .unwrap()
        .expect("staged");
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT "installSource" || ':' || "status" FROM "TemplateInstallation" WHERE "id" = ?1"#,
            staged
        )
        .await,
        json!("tenantDefault:pending")
    );

    let (status, _) = action(
        &fx,
        "platformSettingsActions",
        "clearRootDefaultTemplate",
        OWNER_TOKEN,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        scalar(
            &fx,
            r#"SELECT COUNT(*) FROM "TenantDefaultTemplate" WHERE "tenantId" = ?1"#,
            1
        )
        .await,
        json!(0)
    );

    let (status, out) = action(
        &fx,
        KEY,
        "deleteTemplate",
        OWNER_TOKEN,
        json!({ "workspaceId": ws, "id": template }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
}
