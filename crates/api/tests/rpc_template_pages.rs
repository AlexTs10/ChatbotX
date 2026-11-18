//! Q.D4 follow-up: the reads behind the SPA template screens (the page
//! reads of `templateActions`, `getRootDefaultTemplateSettings`) and the
//! share-page reads (`pages.templateShare` / `pages.templateInstall`).

mod rpc_common;

use axum::http::StatusCode;
use chatbotx_business::template::request::{
    ViewerMembership, find_public_by_share_token, install_page,
};
use chatbotx_db::rusqlite::params;
use rpc_common::{AGENT_TOKEN, OWNER, OWNER_TOKEN, WS, action, fixture};
use serde_json::{Value, json};

const KEY: &str = "templateActions";

async fn read(
    fx: &rpc_common::Fixture,
    name: &str,
    token: &str,
    input: Value,
) -> (StatusCode, Value) {
    action(fx, KEY, name, token, input).await
}

#[tokio::test]
async fn template_pages_read_through_the_member_gates() {
    let fx = fixture().await;
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
    let ws = WS.to_string();
    let (status, out) = read(
        &fx,
        "saveTemplate",
        OWNER_TOKEN,
        json!({
            "workspaceId": ws,
            "name": "Starter",
            "selection": { "keywords": { "mode": "ids", "ids": ["21"] } },
            "defaultPermissions": { "allowEdit": true },
            "createInstallFolder": false,
            "defaultAutoUpdate": false,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let template = out["json"]["templateId"].as_str().unwrap().to_string();

    // `templates/page.tsx`: the manage permission, else refused.
    let (status, _) = read(
        &fx,
        "listTemplates",
        AGENT_TOKEN,
        json!({ "workspaceId": ws }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, out) = read(
        &fx,
        "listTemplates",
        OWNER_TOKEN,
        json!({ "workspaceId": ws }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["items"][0]["id"], json!(template), "{out}");
    assert_eq!(out["json"]["nextCursor"], Value::Null);

    // The editor drops a picked resource that was deleted since the save.
    let edit = json!({ "workspaceId": ws, "id": template });
    let (_, out) = read(&fx, "getEditableTemplate", OWNER_TOKEN, edit.clone()).await;
    assert_eq!(
        out["json"]["selection"]["keywords"]["ids"],
        json!(["21"]),
        "{out}"
    );
    assert!(out["json"].get("payload").is_none(), "{out}");
    fx.db()
        .write(|tx| {
            tx.execute(r#"DELETE FROM "AutomatedResponse" WHERE "id" = 21"#, [])?;
            Ok(())
        })
        .await
        .unwrap();
    let (_, out) = read(&fx, "getEditableTemplate", OWNER_TOKEN, edit.clone()).await;
    assert_eq!(
        out["json"]["selection"]["keywords"]["ids"],
        json!([]),
        "{out}"
    );
    let (status, out) = read(
        &fx,
        "getEditableTemplate",
        OWNER_TOKEN,
        json!({ "workspaceId": ws, "id": "999" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{out}");

    let (_, out) = read(&fx, "listTemplateVersions", OWNER_TOKEN, edit).await;
    assert_eq!(out["json"][0]["version"], json!(1), "{out}");

    // `templates/installs`: `canViewTemplateInstalls`.
    let (status, out) = read(
        &fx,
        "listTemplateInstallations",
        OWNER_TOKEN,
        json!({ "workspaceId": ws, "activeOnly": true, "limit": 1 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["items"], json!([]), "{out}");
    let (status, _) = read(
        &fx,
        "listTemplateSyncHistory",
        OWNER_TOKEN,
        json!({ "workspaceId": ws, "id": "999" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // `/admin/default-template`: a platform super admin only.
    let settings = |token: &'static str| {
        let fx = &fx;
        async move {
            action(
                fx,
                "actions",
                "getRootDefaultTemplateSettings",
                token,
                json!({ "args": [] }),
            )
            .await
        }
    };
    let (status, _) = settings(AGENT_TOKEN).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, out) = settings(OWNER_TOKEN).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"], Value::Null, "{out}");

    // Share pages: nothing while the share is off.
    let (_, link) = read(
        &fx,
        "getShareSettings",
        OWNER_TOKEN,
        json!({ "workspaceId": ws, "id": template }),
    )
    .await;
    let token = link["json"]["shareToken"].as_str().unwrap().to_string();
    assert_eq!(
        find_public_by_share_token(fx.db(), token.clone())
            .await
            .unwrap(),
        None
    );
    let (status, out) = read(
        &fx,
        "updateShareSettings",
        OWNER_TOKEN,
        json!({ "workspaceId": ws, "templateId": template, "shareEnabled": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let public = find_public_by_share_token(fx.db(), token.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(public["name"], "Starter");
    assert!(public.get("id").is_none() && public.get("tenantId").is_none());

    let (status, out) = action(
        &fx,
        "platformSettingsActions",
        "updateRootDefaultTemplate",
        OWNER_TOKEN,
        json!({ "templateId": template, "templateKey": token }),
    )
    .await;
    assert_eq!(out["json"], json!({ "status": "saved" }), "{status} {out}");
    let (_, out) = settings(OWNER_TOKEN).await;
    assert_eq!(
        out["json"],
        json!({ "templateId": template, "status": "ready", "templateKey": token }),
    );

    // The install page offers the viewer's other admin workspaces only.
    let member = |workspace_id, role: &str, permissions| ViewerMembership {
        workspace_id,
        tenant_id: 1,
        role: role.to_string(),
        permissions,
    };
    let page = install_page(
        fx.db(),
        token.clone(),
        vec![
            member(WS, "owner", json!({})),
            member(202, "owner", json!({ "flows": true })),
            member(203, "member", json!({ "flows": true })),
            member(204, "member", json!({ "superAdmin": true })),
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        page["workspaces"],
        json!([
            { "id": "202", "crossTenant": false },
            { "id": "204", "crossTenant": false },
        ]),
        "{page}"
    );
    assert_eq!(page["installedCopies"], json!({}));
    let none = install_page(
        fx.db(),
        "nope".into(),
        vec![member(202, "owner", json!({}))],
    )
    .await
    .unwrap();
    assert_eq!(none["workspaces"], json!([]));
}
