//! The platform settings, platform admin and billing actions of Q.D11 slice
//! 10 under an enterprise license signed by the test (`ApiState::with_license`).

mod rpc_common;

use std::sync::Arc;

use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use chatbotx_business::keys::Edition;
use chatbotx_business::license::LICENSE_ISSUER;
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use rpc_common::{AGENT, AGENT_TOKEN, OWNER, OWNER_TOKEN, action, fixture_as, scalar};
use serde_json::{Value, json};

const KID: &str = "test-license-key";

/// A fresh Ed25519 key, its SPKI PEM and a valid `tier` license.
fn license(tier: &str) -> (String, &'static [(&'static str, &'static str)]) {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let mut der = vec![
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
    ];
    der.extend_from_slice(pair.public_key().as_ref());
    let pem = format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
        STANDARD.encode(der)
    );
    let now = chatbotx_core::js::now_ms() / 1000;
    let claims = json!({
        "customerName": "Acme Corp", "tier": tier, "features": [],
        "limits": { "maxWorkspaces": null, "maxSeats": null, "maxChannels": null },
        "licenseId": "lic_settings", "iss": LICENSE_ISSUER, "sub": "customer_1",
        "iat": now - 60, "exp": now + 86_400,
    });
    let head =
        URL_SAFE_NO_PAD.encode(json!({ "alg": "EdDSA", "kid": KID, "typ": "JWT" }).to_string());
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let message = format!("{head}.{body}");
    let signature = URL_SAFE_NO_PAD.encode(pair.sign(message.as_bytes()).as_ref());
    let keys: &'static [(&'static str, &'static str)] =
        Box::leak(vec![(KID, &*Box::leak(pem.into_boxed_str()))].into_boxed_slice());
    (format!("{message}.{signature}"), keys)
}

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

#[tokio::test]
async fn smtp_needs_a_password_once_then_keeps_the_stored_one() {
    let fx = fixture(Edition::Enterprise, true).await;
    let smtp = |password: Option<&str>| {
        let mut v = json!({ "host": " smtp.test ", "port": "587", "username": "mailer", "fromEmail": "noreply@acme.test" });
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
