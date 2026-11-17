//! The platform credential actions and procedure under `/rpc` (Q.D11 slice
//! 7). Ports `apps/builder/__tests__/platform-credentials-scope.test.ts`:
//! the TS test mocks `isSuperAdmin` / `canUseOwnCredentials` and calls
//! `resolveCredentialScopedUserId`; here the scope runs through the actions
//! against a migrated in-memory database (a `PlatformAdmin` row = super
//! admin, `UserQuota.customAppCredentials` = the entitlement) and the stored
//! rows are checked. Also covers the blank-secret rules of the WhatsApp and
//! Google Ads forms, `cloneFromMessenger` and `getGiphyApiKeyAPI`.

mod rpc_common;

use axum::http::StatusCode;
use chatbotx_business::keys::Edition;
use chatbotx_business::platform_credential::{self as pc, DecryptOptions, UpsertCredential};
use rpc_common::{Fixture, OWNER, OWNER_TOKEN, action, call, encryption, fixture, fixture_as};
use serde_json::{Map, Value, json};

const KEY: &str = "platformCredentialActions";

fn messenger(scope: &str) -> Value {
    json!({
        "scope": scope,
        "clientId": "app-1",
        "version": "v21.0",
        "verifyToken": "verify",
        "clientSecret": "secret",
    })
}

async fn make_super_admin(fx: &Fixture) {
    fx.db()
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "PlatformAdmin" ("id", "userId") VALUES (1, ?1)"#,
                [OWNER],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

async fn grant_entitlement(fx: &Fixture, granted: bool) {
    fx.db()
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "UserQuota" ("id", "userId", "planStatus", "customAppCredentials", "syncedAt") VALUES (1, ?1, 'active', ?2, 0)"#,
                chatbotx_db::rusqlite::params![OWNER, granted],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

/// The decrypted config of a scope's row.
async fn stored(fx: &Fixture, user: Option<i64>, kind: &'static str) -> Option<Map<String, Value>> {
    chatbotx_business::tx::read(&fx.api.db, move |conn| {
        pc::find_decrypted(conn, &encryption(), user, kind, DecryptOptions::default())
    })
    .await
    .unwrap()
    .map(|c| c.config)
}

async fn seed(fx: &Fixture, user: Option<i64>, kind: &'static str, config: Value) {
    chatbotx_business::tx::write(&fx.api.db, move |conn| {
        pc::upsert(
            conn,
            &encryption(),
            UpsertCredential::new(user, kind, config),
        )
    })
    .await
    .unwrap();
}

fn assert_unauthorized(status: StatusCode, out: &Value) {
    assert_eq!(status, StatusCode::FORBIDDEN, "{out}");
    assert_eq!(out["json"]["message"], json!("Unauthorized"));
}

#[tokio::test]
async fn rejects_a_non_super_admin_requesting_platform_scope() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateMessengerSetting",
        OWNER_TOKEN,
        messenger("platform"),
    )
    .await;
    assert_unauthorized(status, &out);
    assert!(stored(&fx, None, "messenger").await.is_none());
}

#[tokio::test]
async fn resolves_platform_scope_to_undefined_for_a_super_admin() {
    let fx = fixture().await;
    make_super_admin(&fx).await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateMessengerSetting",
        OWNER_TOKEN,
        messenger("platform"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        stored(&fx, None, "messenger").await.unwrap()["clientId"],
        json!("app-1")
    );
    assert!(stored(&fx, Some(OWNER), "messenger").await.is_none());
}

#[tokio::test]
async fn rejects_user_scope_for_a_cloud_user_without_credential_entitlement() {
    let fx = fixture_as(Edition::Cloud).await;
    grant_entitlement(&fx, false).await;
    // A super admin row must not matter for the user scope (TS: `isSuperAdmin` not called).
    make_super_admin(&fx).await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateMessengerSetting",
        OWNER_TOKEN,
        messenger("user"),
    )
    .await;
    assert_unauthorized(status, &out);
    assert!(stored(&fx, Some(OWNER), "messenger").await.is_none());
}

#[tokio::test]
async fn resolves_user_scope_for_a_cloud_user_with_credential_entitlement() {
    let fx = fixture_as(Edition::Cloud).await;
    grant_entitlement(&fx, true).await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateMessengerSetting",
        OWNER_TOKEN,
        messenger("user"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        stored(&fx, Some(OWNER), "messenger").await.unwrap()["clientId"],
        json!("app-1")
    );
    assert!(stored(&fx, None, "messenger").await.is_none());
}

#[tokio::test]
async fn resolves_a_user_scope_for_credential_cleanup_after_an_entitlement_downgrade() {
    let fx = fixture_as(Edition::Cloud).await;
    grant_entitlement(&fx, false).await;
    seed(&fx, Some(OWNER), "messenger", messenger("user")).await;
    let (status, out) = action(
        &fx,
        KEY,
        "deleteMessengerSettings",
        OWNER_TOKEN,
        json!({ "scope": "user" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert!(stored(&fx, Some(OWNER), "messenger").await.is_none());
}

#[tokio::test]
async fn community_user_scope_writes_the_platform_row() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateMessengerSetting",
        OWNER_TOKEN,
        messenger("user"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert!(stored(&fx, None, "messenger").await.is_some());
}

fn whatsapp(client_secret: &str, token: &str) -> Value {
    json!({
        "scope": "user",
        "clientId": "wa-app",
        "version": "v21.0",
        "configId": "cfg",
        "systemUserId": "su",
        "businessName": "Acme",
        "verifyToken": "verify",
        "clientSecret": client_secret,
        "systemUserToken": token,
    })
}

#[tokio::test]
async fn whatsapp_blank_secrets_need_a_stored_credential() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateWhatsappSettings",
        OWNER_TOKEN,
        whatsapp("", ""),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{out}");
    assert_eq!(
        out["json"]["code"],
        json!("whatsapp.credentialSecretsRequired")
    );
    assert!(stored(&fx, None, "whatsapp").await.is_none());
}

#[tokio::test]
async fn whatsapp_blank_secrets_keep_the_stored_ones() {
    let fx = fixture().await;
    let (status, out) = action(
        &fx,
        KEY,
        "updateWhatsappSettings",
        OWNER_TOKEN,
        whatsapp("s1", "t1"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"], json!({}));
    let mut renamed = whatsapp("", "");
    renamed["businessName"] = json!("Acme 2");
    let (status, out) = action(&fx, KEY, "updateWhatsappSettings", OWNER_TOKEN, renamed).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let config = stored(&fx, None, "whatsapp").await.unwrap();
    assert_eq!(config["businessName"], json!("Acme 2"));
    assert_eq!(config["clientSecret"], json!("s1"));
    assert_eq!(config["systemUserToken"], json!("t1"));
}

#[tokio::test]
async fn google_ads_keeps_secrets_and_clears_the_developer_token() {
    let fx = fixture().await;
    let input = |secret: &str, token: &str| json!({ "scope": "user", "clientId": "ads", "clientSecret": secret, "developerToken": token, "uploadMethod": "legacy" });
    let (status, out) = action(
        &fx,
        KEY,
        "updateGoogleAdsSettings",
        OWNER_TOKEN,
        input("", ""),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{out}");
    assert_eq!(
        out["json"]["code"],
        json!("googleAds.credentialSecretsRequired")
    );

    let (status, out) = action(
        &fx,
        KEY,
        "updateGoogleAdsSettings",
        OWNER_TOKEN,
        input("cs", "dt"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let (status, out) = action(
        &fx,
        KEY,
        "updateGoogleAdsSettings",
        OWNER_TOKEN,
        json!({ "scope": "user", "clientId": "ads-2", "clientSecret": "", "developerToken": "" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let config = stored(&fx, None, "googleAds").await.unwrap();
    assert_eq!(config["clientId"], json!("ads-2"));
    assert_eq!(config["clientSecret"], json!("cs"));
    assert_eq!(config["developerToken"], json!("dt"));
    assert_eq!(config["uploadMethod"], json!("legacy"));

    let (status, out) = action(
        &fx,
        KEY,
        "clearGoogleAdsDeveloperToken",
        OWNER_TOKEN,
        json!({ "scope": "user" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let config = stored(&fx, None, "googleAds").await.unwrap();
    assert!(!config.contains_key("developerToken"));
    assert_eq!(config["clientSecret"], json!("cs"));
}

#[tokio::test]
async fn clone_from_messenger_copies_the_config() {
    let fx = fixture().await;
    let (status, _) = action(
        &fx,
        KEY,
        "cloneFromMessenger",
        OWNER_TOKEN,
        json!({ "scope": "user" }),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

    seed(&fx, None, "messenger", messenger("user")).await;
    let (status, out) = action(
        &fx,
        KEY,
        "cloneFromMessenger",
        OWNER_TOKEN,
        json!({ "scope": "user" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{out}");
    let config = stored(&fx, None, "instagramFacebook").await.unwrap();
    assert_eq!(config["clientId"], json!("app-1"));
    assert_eq!(config["clientSecret"], json!("secret"));
}

#[tokio::test]
async fn messenger_rejects_a_missing_field() {
    let fx = fixture().await;
    let mut input = messenger("user");
    input.as_object_mut().unwrap().remove("clientId");
    let (status, out) = action(&fx, KEY, "updateMessengerSetting", OWNER_TOKEN, input).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert!(
        out["json"]["data"]["validationErrors"]["clientId"].is_object(),
        "{out}"
    );
}

#[tokio::test]
async fn get_giphy_api_key_reads_the_platform_key() {
    let fx = fixture().await;
    let path = "/rpc/platformCredentialsAPI/getGiphyApiKeyAPI";
    let (status, out) = call(&fx, "GET", path, Some(OWNER_TOKEN), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"], json!({ "apiKey": null }));

    seed(&fx, None, "giphy", json!({ "apiKey": "gk" })).await;
    let (status, out) = call(&fx, "GET", path, Some(OWNER_TOKEN), json!({})).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["json"], json!({ "apiKey": "gk" }));
}

// ─── `apps/builder/__tests__/zalo-settings-actions.test.ts` ───

fn zalo(scope: &str) -> Value {
    json!({
        "scope": scope,
        "clientId": "id",
        "version": "v1",
        "verifyToken": "verify",
        "clientSecret": "secret",
    })
}

#[tokio::test]
async fn zalo_upserts_all_fields() {
    let fx = fixture_as(Edition::Cloud).await;
    grant_entitlement(&fx, true).await;
    let (status, out) = action(&fx, KEY, "updateZaloSettings", OWNER_TOKEN, zalo("user")).await;
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(
        Value::Object(stored(&fx, Some(OWNER), "zalo").await.expect("stored")),
        json!({ "clientId": "id", "version": "v1", "verifyToken": "verify", "clientSecret": "secret" })
    );
}

#[tokio::test]
async fn zalo_rejects_each_empty_field() {
    let fx = fixture().await;
    for field in ["clientId", "version", "verifyToken", "clientSecret"] {
        let mut input = zalo("user");
        input[field] = json!("  ");
        let (status, out) = action(&fx, KEY, "updateZaloSettings", OWNER_TOKEN, input).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {out}");
        assert!(
            out["json"]["data"]["validationErrors"][field].is_object(),
            "{field}: {out}"
        );
    }
}

#[tokio::test]
async fn zalo_and_tiktok_delete_user_and_platform_credentials() {
    let fx = fixture_as(Edition::Cloud).await;
    make_super_admin(&fx).await;
    let tiktok = json!({ "clientId": "id", "clientSecret": "secret" });
    let mut zalo_config = zalo("user");
    zalo_config.as_object_mut().expect("object").remove("scope");
    for (kind, name, config) in [
        ("zalo", "deleteZaloSettings", zalo_config),
        ("tiktok", "deleteTiktokSettings", tiktok),
    ] {
        seed(&fx, Some(OWNER), kind, config.clone()).await;
        seed(&fx, None, kind, config).await;
        for scope in ["user", "platform"] {
            let (status, out) =
                action(&fx, KEY, name, OWNER_TOKEN, json!({ "scope": scope })).await;
            assert_eq!(status, StatusCode::OK, "{name} {scope}: {out}");
        }
        assert!(stored(&fx, Some(OWNER), kind).await.is_none());
        assert!(stored(&fx, None, kind).await.is_none());
    }
}

// ─── `apps/builder/__tests__/tiktok-settings-actions.test.ts` (input schema;
// the subscribe-then-save flow is in `crates/business/tests/connection_oauth_channels.rs`) ───

#[tokio::test]
async fn tiktok_rejects_both_features_off_and_empty_fields() {
    let fx = fixture().await;
    let base = json!({ "scope": "user", "clientId": "id", "clientSecret": "secret",
        "features": { "messaging": false, "comments": false } });
    let (status, out) = action(&fx, KEY, "updateTiktokSetting", OWNER_TOKEN, base.clone()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{out}");
    assert!(
        out["json"]["data"]["validationErrors"]["features"]["messaging"].is_object(),
        "{out}"
    );
    for field in ["clientId", "clientSecret"] {
        let mut input = base.clone();
        input["features"] = json!({ "messaging": true, "comments": true });
        input[field] = json!("");
        let (status, out) = action(&fx, KEY, "updateTiktokSetting", OWNER_TOKEN, input).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{field}: {out}");
        assert!(
            out["json"]["data"]["validationErrors"][field].is_object(),
            "{field}: {out}"
        );
    }
    assert!(stored(&fx, Some(OWNER), "tiktok").await.is_none());
}
