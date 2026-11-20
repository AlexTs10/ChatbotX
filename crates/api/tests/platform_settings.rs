//! The platform settings, platform admin and billing actions of Q.D11 slice
//! 10 under an enterprise license signed by the test (`ApiState::with_license`).

mod rpc_common;

use std::sync::Arc;

use axum::http::StatusCode;
use chatbotx_business::keys::Edition;
use rpc_common::{
    AGENT, AGENT_TOKEN, OWNER, OWNER_TOKEN, WS, action, call, fixture_as, license, scalar,
};
use serde_json::{Value, json};

/// `OWNER` is a super admin; the edition is licensed unless `licensed` is false.
async fn fixture(edition: Edition, licensed: bool) -> rpc_common::Fixture {
    let mut fx = fixture_as(edition).await;
    if licensed {
        let tier = if edition.is_cloud() {
            "cloud"
        } else {
            "enterprise"
        };
        let (token, keys) = license(tier);
        fx.api = Arc::new((*fx.api).clone().with_license(Some(token), keys));
    }
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "PlatformAdmin" ("id", "userId", "role") VALUES (1, ?1, 'super-admin')"#,
                [OWNER],
            )?;
            tx.execute(r#"INSERT INTO "Tenant" ("id", "ownerId", "status") VALUES (7, ?1, 'active')"#, [AGENT])?;
            Ok(())
        })
        .await
        .unwrap();
    fx
}

async fn root_column(fx: &rpc_common::Fixture, column: &'static str) -> Value {
    let sql: &'static str =
        Box::leak(format!(r#"SELECT "{column}" FROM "Tenant" WHERE "id" = ?1"#).into_boxed_str());
    scalar(fx, sql, 1).await
}

fn branding() -> Value {
    json!({
        "brandName": "  Acme  ",
        "logoLight": { "url": "https://cdn.test/light.svg", "mode": "url" },
        "logoDark": { "url": "" },
        "favicon": { "url": "", "mode": "file" },
        "theme": "Teal",
        "customCss": "body{}",
        "customJs": null,
        "policyUrl": "",
        "termsOfServiceUrl": null,
    })
}

const KEY: &str = "platformSettingsActions";

#[tokio::test]
async fn root_branding_updates_the_root_tenant() {
    let fx = fixture(Edition::Enterprise, true).await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateRootPlatformBranding",
        OWNER_TOKEN,
        branding(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(root_column(&fx, "brandName").await, json!("Acme"));
    assert_eq!(
        root_column(&fx, "logoLightPath").await,
        json!("https://cdn.test/light.svg")
    );
    assert_eq!(root_column(&fx, "logoDarkPath").await, Value::Null);
    assert_eq!(root_column(&fx, "theme").await, json!("Teal"));
    assert_eq!(root_column(&fx, "policyUrl").await, json!(""));

    let mut bad = branding();
    bad["theme"] = json!("Black");
    bad["logoDark"] = json!({ "url": "not a url" });
    let (status, out) = action(&fx, KEY, "updateRootPlatformBranding", OWNER_TOKEN, bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    // A non super admin is refused before the schema runs.
    let (status, _) = action(
        &fx,
        KEY,
        "updateRootPlatformBranding",
        AGENT_TOKEN,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// `platformAdminActionClient` + `upsertByOwner`: on cloud the owner of an
/// active tenant edits that tenant.
#[tokio::test]
async fn own_branding_and_templates_go_to_the_callers_tenant_on_cloud() {
    let fx = fixture(Edition::Cloud, true).await;
    let (status, out) = action(&fx, KEY, "updatePlatformBranding", AGENT_TOKEN, branding()).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let sql = r#"SELECT "brandName" FROM "Tenant" WHERE "id" = ?1"#;
    assert_eq!(scalar(&fx, sql, 7).await, json!("Acme"));
    assert_eq!(root_column(&fx, "brandName").await, Value::Null);

    let template = json!({ "type": "magicLink", "subject": "Sign in", "body": "<p>{{url}}</p>" });
    let (status, out) = action(&fx, KEY, "updateEmailTemplate", AGENT_TOKEN, template).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let sql = r#"SELECT "magicLinkEmailTemplate" FROM "Tenant" WHERE "id" = ?1"#;
    let stored: Value = serde_json::from_str(scalar(&fx, sql, 7).await.as_str().unwrap()).unwrap();
    assert_eq!(
        stored,
        json!({ "subject": "Sign in", "body": "<p>{{url}}</p>" })
    );
    // A blank body clears the template.
    let blank = json!({ "type": "magicLink", "subject": "Sign in", "body": "  " });
    let (status, _) = action(&fx, KEY, "updateEmailTemplate", AGENT_TOKEN, blank).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(scalar(&fx, sql, 7).await, Value::Null);
    // The owner has no tenant: not a platform admin on cloud.
    let (status, _) = action(&fx, KEY, "updatePlatformBranding", OWNER_TOKEN, branding()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn root_email_template_is_stored_on_the_root_tenant() {
    let fx = fixture(Edition::Enterprise, true).await;
    let template = json!({ "type": "accountCredentials", "subject": null, "body": "<p>Hi</p>" });
    let (status, out) = action(&fx, KEY, "updateRootEmailTemplate", OWNER_TOKEN, template).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let stored = root_column(&fx, "accountCredentialsEmailTemplate").await;
    assert_eq!(
        serde_json::from_str::<Value>(stored.as_str().unwrap()).unwrap(),
        json!({ "body": "<p>Hi</p>" })
    );
    let bad = json!({ "type": "welcome", "subject": null, "body": null });
    let (status, _) = action(&fx, KEY, "updateRootEmailTemplate", OWNER_TOKEN, bad).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

const SMTP_ROWS: &str = r#"SELECT COUNT(*) FROM "PlatformCredential" WHERE "type" = 'smtp' AND "userId" IS NULL AND ?1 = 1"#;

/// A local SMTP server that accepts any `AUTH` (`verifySmtpTransport` is
/// mocked in the original test); the port of a closed one refuses.
async fn fake_smtp() -> u16 {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (read, mut write) = socket.into_split();
                let mut lines = BufReader::new(read).lines();
                let _ = write.write_all(b"220 fake ESMTP\r\n").await;
                while let Ok(Some(line)) = lines.next_line().await {
                    let verb = line
                        .split(' ')
                        .next()
                        .unwrap_or_default()
                        .to_ascii_uppercase();
                    let reply: &[u8] = match verb.as_str() {
                        "EHLO" | "HELO" => b"250-fake\r\n250 AUTH PLAIN LOGIN\r\n",
                        "AUTH" => b"235 ok\r\n",
                        "QUIT" => b"221 bye\r\n",
                        _ => b"250 ok\r\n",
                    };
                    let _ = write.write_all(reply).await;
                    if verb == "QUIT" {
                        break;
                    }
                }
            });
        }
    });
    port
}

#[tokio::test]
async fn smtp_needs_a_password_once_then_keeps_the_stored_one() {
    let fx = fixture(Edition::Enterprise, true).await;
    let port = fake_smtp().await;
    let smtp = |password: Option<&str>| {
        let mut v = json!({ "host": " 127.0.0.1 ", "port": port.to_string(), "username": "mailer", "fromEmail": "noreply@acme.test" });
        if let Some(p) = password {
            v["password"] = json!(p);
        }
        v
    };
    let (status, out) = action(&fx, KEY, "updatePlatformSmtp", OWNER_TOKEN, smtp(Some(""))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["data"]["validationErrors"]["password"]["_errors"],
        json!(["Enter the SMTP password."]),
        "{out}"
    );
    assert_eq!(scalar(&fx, SMTP_ROWS, 1).await, json!(0));

    let (status, out) = action(
        &fx,
        KEY,
        "updatePlatformSmtp",
        OWNER_TOKEN,
        smtp(Some("secret")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (status, out) = action(&fx, KEY, "updatePlatformSmtp", OWNER_TOKEN, smtp(None)).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(scalar(&fx, SMTP_ROWS, 1).await, json!(1));

    let (status, out) = action(&fx, KEY, "deletePlatformSmtp", OWNER_TOKEN, json!({})).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(scalar(&fx, SMTP_ROWS, 1).await, json!(0));
}

#[tokio::test]
async fn smtp_that_cannot_connect_is_not_saved() {
    let fx = fixture(Edition::Enterprise, true).await;
    // A port with nothing listening on it.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let input = json!({ "host": "127.0.0.1", "port": port, "username": "mailer", "password": "secret", "fromEmail": "noreply@acme.test" });
    let (status, out) = action(&fx, KEY, "updatePlatformSmtp", OWNER_TOKEN, input).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert_eq!(
        out["json"]["message"],
        json!("Cannot connect to SMTP server. Please check your credentials and try again."),
        "{out}"
    );
    assert_eq!(scalar(&fx, SMTP_ROWS, 1).await, json!(0));
}

#[tokio::test]
async fn hidden_platform_channels_must_be_creatable() {
    let fx = fixture(Edition::Enterprise, true).await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateRootPlatformChannels",
        OWNER_TOKEN,
        json!({ "hiddenChannels": ["whatsapp", "telegram"] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        root_column(&fx, "hiddenChannels").await,
        json!(r#"["whatsapp","telegram"]"#)
    );

    let (status, out) = action(
        &fx,
        KEY,
        "updateRootPlatformChannels",
        OWNER_TOKEN,
        json!({ "hiddenChannels": ["smtp"] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");

    // Not a super admin.
    let (status, out) = action(
        &fx,
        KEY,
        "updateRootPlatformChannels",
        AGENT_TOKEN,
        json!({ "hiddenChannels": [] }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");
}

#[tokio::test]
async fn email_preview_renders_mjml_and_wraps_html() {
    let fx = fixture(Edition::Enterprise, true).await;
    let (status, out) = action(
        &fx,
        KEY,
        "previewEmailTemplate",
        OWNER_TOKEN,
        json!({ "body": "<p>Hi {{userName}} {{other}}</p>" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        out["json"],
        json!(
            "<!DOCTYPE html><html><body style=\"font-family:sans-serif;padding:16px\"><p>Hi John Doe {{other}}</p></body></html>"
        )
    );

    let mjml = "<mjml><mj-body><mj-section><mj-column><mj-text>Hello {{brandName}}</mj-text></mj-column></mj-section></mj-body></mjml>";
    let (status, out) = action(
        &fx,
        KEY,
        "previewEmailTemplate",
        OWNER_TOKEN,
        json!({ "body": mjml }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let html = out["json"].as_str().unwrap_or_default();
    assert!(
        html.contains("Hello ChatbotX") && !html.contains("<mj-"),
        "{html}"
    );

    // Not a super admin nor a platform admin (no own active tenant off cloud).
    let (status, out) = action(
        &fx,
        KEY,
        "previewEmailTemplate",
        AGENT_TOKEN,
        json!({ "body": "x" }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");
}

#[tokio::test]
async fn without_a_license_every_settings_action_is_refused() {
    let fx = fixture(Edition::Enterprise, false).await;
    for (name, input) in [
        ("updateRootPlatformBranding", branding()),
        ("deletePlatformSmtp", json!({})),
    ] {
        let (status, out) = action(&fx, KEY, name, OWNER_TOKEN, input).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{name}: {out}");
        assert_eq!(
            out["json"]["code"],
            json!("enterpriseFeatureRequired"),
            "{out}"
        );
    }
    assert_eq!(root_column(&fx, "brandName").await, Value::Null);
}

#[tokio::test]
async fn add_platform_admin_through_the_action_with_a_license() {
    let fx = fixture(Edition::Enterprise, true).await;
    let input = json!({ "email": " Agent@RPC.test ", "role": "support" });
    let (status, out) = action(
        &fx,
        "platformAdminActions",
        "addPlatformAdmin",
        OWNER_TOKEN,
        input,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        (out["json"]["userId"].clone(), out["json"]["role"].clone()),
        (json!(AGENT.to_string()), json!("support"))
    );
    let input = json!({ "userId": AGENT.to_string() });
    let (status, out) = action(
        &fx,
        "platformAdminActions",
        "removePlatformAdmin",
        OWNER_TOKEN,
        input,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
}

/// `reconcileTenantEntitlementAction`: cloud only.
#[tokio::test]
async fn reconcile_tenant_entitlement_provisions_on_cloud_only() {
    for (edition, expected) in [(Edition::Enterprise, 0), (Edition::Cloud, 1)] {
        let fx = fixture(edition, false).await;
        fx.db()
            .write(|tx| {
                tx.execute(
                    r#"INSERT INTO "UserQuota" ("id", "userId", "whiteLabel") VALUES (1, ?1, 1)"#,
                    [OWNER],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let (status, out) = action(
            &fx,
            "billingActions",
            "reconcileTenantEntitlement",
            OWNER_TOKEN,
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{out}");
        let sql = r#"SELECT COUNT(*) FROM "Tenant" WHERE "ownerId" = ?1"#;
        assert_eq!(
            scalar(&fx, sql, OWNER).await,
            json!(expected),
            "{edition:?}"
        );
    }
}

/// A console read (`POST /rpc/actions/<name>` with the SPA's `{ args }`).
async fn read(
    fx: &rpc_common::Fixture,
    name: &str,
    token: &str,
    args: Value,
) -> (StatusCode, Value) {
    call(
        fx,
        "POST",
        &format!("/rpc/actions/{name}"),
        Some(token),
        json!({ "args": args }),
    )
    .await
}

#[tokio::test]
async fn platform_audit_logs_list_every_workspace_with_its_name() {
    let fx = fixture(Edition::Enterprise, true).await;
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (202, 'Beta', ?1)"#,
                [OWNER],
            )?;
            tx.execute(
                r#"INSERT INTO "AuditLog" ("id", "action", "detail", "workspaceId", "userId") VALUES (1, 'flow.created', 'A', ?1, ?2), (2, 'flow.deleted', 'B', 202, NULL)"#,
                [WS, OWNER],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    let (status, out) = read(&fx, "listPlatformAuditLogs", OWNER_TOKEN, json!([{}])).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let rows = out["json"]["data"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 2, "{out}");
    let beta = rows.iter().find(|r| r["action"] == "flow.deleted").unwrap();
    assert_eq!(beta["workspace"], json!({ "id": "202", "name": "Beta" }));

    let (status, out) = read(
        &fx,
        "listPlatformAuditLogs",
        OWNER_TOKEN,
        json!([{ "workspaceId": WS.to_string() }]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        out["json"]["data"].as_array().map(Vec::len),
        Some(1),
        "{out}"
    );

    // The workspace filter options: the selected one is listed.
    let (status, out) = read(
        &fx,
        "listPlatformAuditWorkspaces",
        OWNER_TOKEN,
        json!(["202"]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert!(
        out["json"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o == &json!({ "id": "202", "label": "Beta" })),
        "{out}"
    );

    // Not a super admin: `notFound()`.
    let (status, _) = read(&fx, "listPlatformAuditLogs", AGENT_TOKEN, json!([{}])).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn enterprise_console_reads_need_the_license_and_a_super_admin() {
    let fx = fixture(Edition::Enterprise, false).await;
    for name in [
        "getPlatformSmtpSettings",
        "listPlatformAuditLogs",
        "listRootHelpItems",
    ] {
        let (status, out) = read(&fx, name, OWNER_TOKEN, json!([{}])).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{name}: {out}");
    }
    // Outside the `(enterprise)` group: the root tenant and the deletion records.
    let (status, out) = read(&fx, "getRootTenant", OWNER_TOKEN, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (status, out) = read(&fx, "listDeletedWorkspaces", OWNER_TOKEN, json!([{}])).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"], json!({ "data": [], "pageCount": 0 }));
    let (status, _) = read(&fx, "listDeletedWorkspaces", AGENT_TOKEN, json!([{}])).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn platform_smtp_settings_are_not_served_on_cloud() {
    let fx = fixture(Edition::Cloud, true).await;
    let (status, out) = read(&fx, "getPlatformSmtpSettings", OWNER_TOKEN, json!([])).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{out}");
    let fx = fixture(Edition::Enterprise, true).await;
    let (status, out) = read(&fx, "getPlatformSmtpSettings", OWNER_TOKEN, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"], Value::Null);
}

#[tokio::test]
async fn credential_cards_of_the_platform_need_a_super_admin() {
    let fx = fixture(Edition::Enterprise, true).await;
    let (status, out) = read(
        &fx,
        "getPlatformCredentialCards",
        OWNER_TOKEN,
        json!(["platform"]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let cards = &out["json"];
    assert_eq!(cards["hasStripeWebhook"], json!(false), "{out}");
    assert_eq!(cards["whatsapp"]["publicConfig"], Value::Null);
    assert_eq!(cards["whatsapp"]["isInherited"], json!(false));
    assert!(
        cards["messenger"]["webhookUrl"]
            .as_str()
            .is_some_and(|u| u.ends_with("/integrations/messenger/webhook")),
        "{out}"
    );
    // Self-hosted "user" scope is the global credentials too: a super admin.
    let (status, _) = read(
        &fx,
        "getPlatformCredentialCards",
        AGENT_TOKEN,
        json!(["user"]),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn manage_console_is_cloud_only_and_for_active_resellers() {
    // Off cloud: the manage layout is a 404.
    let fx = fixture(Edition::Enterprise, true).await;
    let (status, _) = read(&fx, "getManageAccess", AGENT_TOKEN, json!([])).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let fx = fixture(Edition::Cloud, true).await;
    // `AGENT` owns the active tenant 7.
    let (status, out) = read(&fx, "getManageAccess", AGENT_TOKEN, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["canManageReseller"], json!(true), "{out}");
    assert_eq!(out["json"]["canManageCredentials"], json!(true), "{out}");
    let (status, out) = read(&fx, "getOwnTenant", AGENT_TOKEN, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"]["id"], json!("7"), "{out}");
    let (status, out) = read(&fx, "getOwnPlatformChannels", AGENT_TOKEN, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        out["json"],
        json!({ "hiddenChannels": [], "platformHiddenChannels": [] })
    );

    // No tenant and no custom-app entitlement: no manage access.
    let (status, _) = read(&fx, "getManageAccess", OWNER_TOKEN, json!([])).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = read(&fx, "getOwnTenant", OWNER_TOKEN, json!([])).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
