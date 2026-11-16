//! Ported from `packages/business/__tests__/platform-credential-service.test.ts`
//! (Q.D11 slice 7). The TS test mocks drizzle and `encryptUtils`; here rows
//! are real SQLite rows encrypted with a test key, so the AAD checks are real
//! decrypt failures.

mod common;

use chatbotx_business::platform_credential::{
    self as pc, DecryptOptions, PublicCredential, UpsertCredential,
};
use chatbotx_core::encryption::{Encryption, Keys};
use chatbotx_db::Db;
use chatbotx_db::repo::partials::credential::credential_aad_for;
use chatbotx_db::rusqlite::{Connection, params};
use common::{TestDb, fresh_db, setup};
use serde_json::{Value, json};

const OWNER: i64 = 7;
const OTHER: i64 = 8;

fn encryption() -> Encryption {
    Encryption::new(Keys::new(&"a1".repeat(32)).expect("test key"))
}

async fn fixture() -> TestDb {
    let db = fresh_db().await;
    setup(&db, |tx| {
        for id in [OWNER, OTHER] {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
                params![id, format!("u{id}@example.test")],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

fn meta(client: &str) -> Value {
    json!({ "clientId": client, "version": "v21.0", "verifyToken": "verify", "clientSecret": format!("{client}-secret") })
}

fn whatsapp() -> Value {
    json!({
        "clientId": "wa-app", "version": "v21.0", "configId": "cfg", "systemUserId": "su",
        "businessName": "Biz", "verifyToken": "verify", "clientSecret": "wa-secret",
        "systemUserToken": "system-token",
    })
}

async fn upsert(db: &Db, input: UpsertCredential) -> i64 {
    chatbotx_business::tx::write(db, move |tx| pc::upsert(tx, &encryption(), input))
        .await
        .expect("upsert")
}

async fn read<R: Send + 'static>(
    db: &Db,
    f: impl FnOnce(&Connection, &Encryption) -> R + Send + 'static,
) -> R {
    db.read(move |conn| Ok(f(conn, &encryption())))
        .await
        .expect("read")
}

/// Stores a row whose value is encrypted under `aad_user`'s AAD while the
/// row itself belongs to `row_user`.
async fn store_with_aad(
    db: &Db,
    row_user: Option<i64>,
    aad_user: Option<i64>,
    kind: &str,
    config: Value,
) {
    let aad_owner = aad_user.map(|u| u.to_string());
    let aad = credential_aad_for(aad_owner.as_deref(), kind);
    let blob = encryption()
        .encrypt_object(&config, Some(&aad))
        .expect("encrypt");
    let value = serde_json::to_string(&blob).expect("json");
    let kind = kind.to_string();
    db.write(move |tx| {
        tx.execute(
            r#"INSERT INTO "PlatformCredential" ("id", "userId", "type", "publicConfig", "value") VALUES (?1, ?2, ?3, '{}', ?4)"#,
            params![chatbotx_db::repo::new_id(), row_user, kind, value],
        )?;
        Ok(())
    })
    .await
    .expect("store");
}

// --- credential decryption AAD ---

#[tokio::test]
async fn passes_the_user_credential_aad_to_decrypt() {
    let db = fixture().await;
    store_with_aad(&db, Some(OWNER), Some(OWNER), "messenger", meta("own")).await;
    let found = read(&db, |c, e| {
        pc::find_decrypted(c, e, Some(OWNER), "messenger", DecryptOptions::default())
    })
    .await
    .expect("find")
    .expect("credential");
    assert_eq!(found.text("clientSecret"), Some("own-secret"));
}

#[tokio::test]
async fn passes_the_platform_credential_aad_to_decrypt() {
    let db = fixture().await;
    store_with_aad(&db, None, None, "messenger", meta("platform")).await;
    let found = read(&db, |c, e| {
        pc::find_decrypted(c, e, None, "messenger", DecryptOptions::default())
    })
    .await
    .expect("find")
    .expect("credential");
    assert_eq!(found.text("clientId"), Some("platform"));
}

#[tokio::test]
async fn rejects_a_ciphertext_bound_to_another_users_credential_row() {
    let db = fixture().await;
    store_with_aad(&db, Some(OWNER), Some(OTHER), "messenger", meta("stolen")).await;
    let strict = read(&db, |c, e| {
        pc::find_decrypted(
            c,
            e,
            Some(OWNER),
            "messenger",
            DecryptOptions {
                strict: true,
                ..Default::default()
            },
        )
    })
    .await;
    assert!(strict.is_err());
}

// --- resolveForOwner ---

#[tokio::test]
async fn returns_an_own_credential() {
    let db = fixture().await;
    upsert(
        &db,
        UpsertCredential::new(Some(OWNER), "messenger", meta("own")),
    )
    .await;
    upsert(
        &db,
        UpsertCredential::new(None, "messenger", meta("platform")),
    )
    .await;
    let found = read(&db, |c, e| {
        pc::resolve_for_owner(c, e, OWNER, "messenger", false)
    })
    .await
    .expect("resolve");
    assert_eq!(
        found
            .and_then(|f| f.text("clientId").map(String::from))
            .as_deref(),
        Some("own")
    );
}

#[tokio::test]
async fn falls_back_to_the_platform_credential_when_no_own_row_exists() {
    let db = fixture().await;
    upsert(
        &db,
        UpsertCredential::new(None, "messenger", meta("platform")),
    )
    .await;
    let found = read(&db, |c, e| {
        pc::resolve_for_owner(c, e, OWNER, "messenger", false)
    })
    .await
    .expect("resolve");
    assert_eq!(found.map(|f| f.user_id), Some(None));
}

#[tokio::test]
async fn does_not_fall_back_to_the_platform_credential_when_an_own_credential_cannot_decrypt() {
    let db = fixture().await;
    store_with_aad(&db, Some(OWNER), Some(OTHER), "messenger", meta("bad")).await;
    upsert(
        &db,
        UpsertCredential::new(None, "messenger", meta("platform")),
    )
    .await;
    let found = read(&db, |c, e| {
        pc::resolve_for_owner(c, e, OWNER, "messenger", false)
    })
    .await
    .expect("resolve");
    assert_eq!(found, None);
}

#[tokio::test]
async fn falls_back_to_the_platform_credential_when_an_owner_delegates() {
    let db = fixture().await;
    let mut own = UpsertCredential::new(Some(OWNER), "messenger", meta("own"));
    own.use_platform_credential = true;
    upsert(&db, own).await;
    upsert(
        &db,
        UpsertCredential::new(None, "messenger", meta("platform")),
    )
    .await;
    let found = read(&db, |c, e| {
        pc::resolve_for_owner(c, e, OWNER, "messenger", false)
    })
    .await
    .expect("resolve");
    assert_eq!(
        found
            .and_then(|f| f.text("clientId").map(String::from))
            .as_deref(),
        Some("platform")
    );
}

// --- upserts ---

#[tokio::test]
async fn returns_the_user_credential_id_and_updates_in_place() {
    // Also "persists a caller-provided credential id" (adapted: ids are
    // server-made; a second upsert keeps the row id).
    let db = fixture().await;
    let first = upsert(
        &db,
        UpsertCredential::new(Some(OWNER), "messenger", meta("one")),
    )
    .await;
    let second = upsert(
        &db,
        UpsertCredential::new(Some(OWNER), "messenger", meta("two")),
    )
    .await;
    assert_eq!(first, second);
    let platform = upsert(&db, UpsertCredential::new(None, "messenger", meta("p"))).await;
    assert_ne!(platform, first);
    let row = read(&db, |c, _| pc::find_for_user(c, OWNER, "messenger"))
        .await
        .expect("read")
        .expect("row");
    assert_eq!(
        row.public_config.0,
        json!({ "clientId": "two", "version": "v21.0", "verifyToken": "verify" })
    );
}

// --- findDecryptedForWebhook ---

#[tokio::test]
async fn decrypts_the_matching_user_owned_credential_and_returns_none_when_none_exists() {
    let db = fixture().await;
    let id = upsert(
        &db,
        UpsertCredential::new(
            Some(OWNER),
            "stripe",
            json!({
                "publishableKey": "pk", "verifyToken": "v", "secretKey": "sk",
            }),
        ),
    )
    .await;
    let platform = upsert(
        &db,
        UpsertCredential::new(
            None,
            "stripe",
            json!({
                "publishableKey": "pk", "verifyToken": "v", "secretKey": "sk",
            }),
        ),
    )
    .await;
    let found = read(&db, move |c, e| {
        pc::find_decrypted_for_webhook(c, e, id, "stripe")
    })
    .await
    .expect("find");
    assert_eq!(found.map(|f| f.id), Some(id));
    let missing = read(&db, move |c, e| {
        pc::find_decrypted_for_webhook(c, e, platform, "stripe")
    })
    .await
    .expect("find");
    assert_eq!(missing, None, "a platform row is never addressed");
}

// --- resolveWhatsappSystemUserToken ---

#[tokio::test]
async fn returns_the_system_user_token_from_the_resolved_whatsapp_credential() {
    let db = fixture().await;
    upsert(
        &db,
        UpsertCredential::new(Some(OWNER), "whatsapp", whatsapp()),
    )
    .await;
    let token = read(&db, |c, e| {
        pc::resolve_whatsapp_system_user_token(c, e, OWNER)
    })
    .await
    .expect("token");
    assert_eq!(token.as_deref(), Some("system-token"));
}

#[tokio::test]
async fn returns_none_when_the_owner_has_no_whatsapp_credential() {
    let db = fixture().await;
    let token = read(&db, |c, e| {
        pc::resolve_whatsapp_system_user_token(c, e, OWNER)
    })
    .await
    .expect("token");
    assert_eq!(token, None);
}

// --- resolvePublicForUser ---

async fn public(db: &Db, user: i64, kind: &'static str) -> Option<PublicCredential> {
    read(db, move |c, e| {
        pc::resolve_public_for_user(c, e, user, kind)
    })
    .await
    .expect("public")
}

#[tokio::test]
async fn own_credential_set_returns_it_as_not_inherited() {
    let db = fixture().await;
    let id = upsert(
        &db,
        UpsertCredential::new(Some(OWNER), "messenger", meta("own")),
    )
    .await;
    let found = public(&db, OWNER, "messenger").await.expect("public");
    assert_eq!((found.credential_id, found.is_inherited), (Some(id), false));
    assert_eq!(found.public_config["clientId"], "own");
    assert!(found.public_config.get("clientSecret").is_none());
}

#[tokio::test]
async fn no_own_credential_falls_back_to_platform_as_inherited() {
    let db = fixture().await;
    upsert(
        &db,
        UpsertCredential::new(None, "messenger", meta("platform")),
    )
    .await;
    let found = public(&db, OWNER, "messenger").await.expect("public");
    assert_eq!((found.credential_id, found.is_inherited), (None, true));
}

#[tokio::test]
async fn own_credential_flagged_use_platform_credential_falls_back_to_platform() {
    let db = fixture().await;
    let mut own = UpsertCredential::new(Some(OWNER), "messenger", meta("own"));
    own.use_platform_credential = true;
    upsert(&db, own).await;
    upsert(
        &db,
        UpsertCredential::new(None, "messenger", meta("platform")),
    )
    .await;
    let found = public(&db, OWNER, "messenger").await.expect("public");
    assert!(found.is_inherited);
    assert_eq!(found.public_config["clientId"], "platform");
}

#[tokio::test]
async fn neither_own_nor_platform_returns_none() {
    let db = fixture().await;
    assert_eq!(public(&db, OWNER, "messenger").await, None);
}

// --- resolvePlatformAppAccessToken ---

#[tokio::test]
async fn returns_client_id_and_client_secret_joined_as_a_meta_app_token() {
    let db = fixture().await;
    upsert(&db, UpsertCredential::new(None, "messenger", meta("app"))).await;
    let token = read(&db, |c, e| {
        pc::resolve_platform_app_access_token(c, e, "messenger")
    })
    .await
    .expect("token");
    assert_eq!(token.as_deref(), Some("app|app-secret"));
}

#[tokio::test]
async fn returns_none_when_the_platform_credential_is_missing() {
    let db = fixture().await;
    let token = read(&db, |c, e| {
        pc::resolve_platform_app_access_token(c, e, "messenger")
    })
    .await
    .expect("token");
    assert_eq!(token, None);
}

// --- findDecryptedThreadsByClientId ---

#[tokio::test]
async fn queries_non_delegated_threads_credentials_and_decrypts_the_first_deterministic_match() {
    let db = fixture().await;
    let mut delegating = UpsertCredential::new(Some(OTHER), "threads", meta("th"));
    delegating.use_platform_credential = true;
    upsert(&db, delegating).await;
    upsert(
        &db,
        UpsertCredential::new(Some(OWNER), "threads", meta("th")),
    )
    .await;
    upsert(&db, UpsertCredential::new(None, "threads", meta("th"))).await;
    let found = read(&db, |c, e| {
        pc::find_decrypted_threads_by_client_id(c, e, "th")
    })
    .await
    .expect("match");
    assert_eq!(found.user_id, None, "the platform row sorts first");
}

#[tokio::test]
async fn returns_none_when_no_threads_credential_matches_the_client_id() {
    let db = fixture().await;
    upsert(
        &db,
        UpsertCredential::new(Some(OWNER), "threads", meta("th")),
    )
    .await;
    assert_eq!(
        read(&db, |c, e| pc::find_decrypted_threads_by_client_id(
            c, e, "other"
        ))
        .await,
        None
    );
}

// --- strict credential lookups ---

async fn broken_rows(db: &Db) {
    store_with_aad(db, Some(OWNER), Some(OTHER), "messenger", meta("bad")).await;
    store_with_aad(db, None, Some(OTHER), "messenger", meta("bad")).await;
}

#[tokio::test]
async fn non_strict_lookups_swallow_failures() {
    let db = fixture().await;
    broken_rows(&db).await;
    for user in [Some(OWNER), None] {
        let found = read(&db, move |c, e| {
            pc::find_decrypted(c, e, user, "messenger", DecryptOptions::default())
        })
        .await;
        assert_eq!(found.expect("swallowed"), None);
    }
}

#[tokio::test]
async fn strict_lookups_rethrow_failures() {
    let db = fixture().await;
    broken_rows(&db).await;
    for user in [Some(OWNER), None] {
        let found = read(&db, move |c, e| {
            pc::find_decrypted(
                c,
                e,
                user,
                "messenger",
                DecryptOptions {
                    strict: true,
                    ..Default::default()
                },
            )
        })
        .await;
        assert!(found.is_err(), "{user:?}");
    }
}

#[tokio::test]
async fn resolve_for_owner_forwards_strict_to_the_platform_fallback() {
    let db = fixture().await;
    store_with_aad(&db, None, Some(OTHER), "messenger", meta("bad")).await;
    assert!(
        read(&db, |c, e| pc::resolve_for_owner(
            c,
            e,
            OWNER,
            "messenger",
            true
        ))
        .await
        .is_err()
    );
    assert_eq!(
        read(&db, |c, e| pc::resolve_for_owner(
            c,
            e,
            OWNER,
            "messenger",
            false
        ))
        .await
        .expect("lenient"),
        None
    );
}

#[tokio::test]
async fn strict_resolve_for_owner_propagates_an_own_credential_failure_without_falling_back() {
    let db = fixture().await;
    store_with_aad(&db, Some(OWNER), Some(OTHER), "messenger", meta("bad")).await;
    upsert(
        &db,
        UpsertCredential::new(None, "messenger", meta("platform")),
    )
    .await;
    assert!(
        read(&db, |c, e| pc::resolve_for_owner(
            c,
            e,
            OWNER,
            "messenger",
            true
        ))
        .await
        .is_err()
    );
}

// --- googleAds hasDeveloperToken derivation (legacy rows) ---

async fn legacy_google_ads(db: &Db, user: Option<i64>, developer_token: Option<&str>) {
    let mut config = json!({ "clientId": "gid", "clientSecret": "gsecret" });
    if let Some(token) = developer_token {
        config["developerToken"] = json!(token);
    }
    store_with_aad(db, user, user, "googleAds", config).await;
    db.write(|tx| {
        tx.execute(r#"UPDATE "PlatformCredential" SET "publicConfig" = '{"clientId":"gid"}' WHERE "type" = 'googleAds'"#, [])?;
        Ok(())
    })
    .await
    .expect("legacy public config");
}

#[tokio::test]
async fn legacy_row_whose_encrypted_value_holds_a_token_reads_true_token_never_leaked() {
    let db = fixture().await;
    legacy_google_ads(&db, None, Some("dev-token")).await;
    let public = read(&db, |c, e| pc::find_platform_public(c, e, "googleAds"))
        .await
        .expect("read")
        .expect("row");
    assert_eq!(
        public,
        json!({ "clientId": "gid", "hasDeveloperToken": true })
    );
}

#[tokio::test]
async fn legacy_row_without_a_token_reads_false() {
    let db = fixture().await;
    legacy_google_ads(&db, None, None).await;
    let public = read(&db, |c, e| pc::find_platform_public(c, e, "googleAds"))
        .await
        .expect("read")
        .expect("row");
    assert_eq!(public["hasDeveloperToken"], false);
}

#[tokio::test]
async fn row_already_carrying_the_flag_is_not_decrypted() {
    // The stored value cannot decrypt (wrong AAD): the flag is read as stored.
    let db = fixture().await;
    store_with_aad(
        &db,
        None,
        Some(OTHER),
        "googleAds",
        json!({ "clientId": "gid", "clientSecret": "s" }),
    )
    .await;
    db.write(|tx| {
        tx.execute(r#"UPDATE "PlatformCredential" SET "publicConfig" = '{"clientId":"gid","hasDeveloperToken":false}'"#, [])?;
        Ok(())
    })
    .await
    .expect("flagged");
    let public = read(&db, |c, e| pc::find_platform_public(c, e, "googleAds"))
        .await
        .expect("read")
        .expect("row");
    assert_eq!(public["hasDeveloperToken"], false);
}

#[tokio::test]
async fn resolve_public_for_user_derives_the_flag_for_the_users_own_legacy_row() {
    let db = fixture().await;
    legacy_google_ads(&db, Some(OWNER), Some("dev-token")).await;
    let found = public(&db, OWNER, "googleAds").await.expect("public");
    assert_eq!(found.public_config["hasDeveloperToken"], true);
    assert!(found.public_config.get("developerToken").is_none());
}

#[tokio::test]
async fn non_google_ads_rows_are_never_decrypted() {
    let db = fixture().await;
    store_with_aad(&db, None, Some(OTHER), "messenger", meta("bad")).await;
    let public = read(&db, |c, e| pc::find_platform_public(c, e, "messenger"))
        .await
        .expect("read");
    assert_eq!(public, Some(json!({})));
}

// --- schema projection ---

#[test]
fn public_config_picks_the_public_fields_only() {
    let config = pc::parse_config(
        "smtp",
        &json!({
            "host": "smtp.example.test", "port": 587, "username": "u", "password": "p",
            "fromEmail": "a@example.test", "extra": 1,
        }),
    )
    .expect("valid");
    assert!(config.get("extra").is_none(), "unknown keys are stripped");
    assert_eq!(
        pc::public_config("smtp", &config),
        json!({ "host": "smtp.example.test", "port": 587, "username": "u", "fromEmail": "a@example.test" })
    );
    assert!(pc::parse_config("smtp", &json!({ "host": "h" })).is_err());
}
