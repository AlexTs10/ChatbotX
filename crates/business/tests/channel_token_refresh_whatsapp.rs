//! Port of `apps/worker/__tests__/refresh-whatsapp-tokens.test.ts`: the
//! WhatsApp leg of the `refreshChannelTokens` schedule (the phone rows, then
//! one encrypted WhatsApp Business Account credential per WABA) over a
//! migrated database and a scripted Graph API.
//! ponytail: the stale-revision case needs a reconnect between the read and
//! the CAS write, which this harness cannot inject; the CAS is the
//! `revision` guard of `upsert_credential`.

mod channel_fake;
mod common;
mod x1_support;

use channel_fake::{FakeHttp, auth_row, channel_row, graph_error, ok};
use chatbotx_business::audit::AuditService;
use chatbotx_business::channel_token_refresh::{RefreshDeps, refresh_scheduled};
use chatbotx_business::meta_catalog::process_encryption;
use chatbotx_core::encryption::EncryptedData;
use chatbotx_db::rusqlite::params;
use common::{WS, setup, two_workspaces};
use serde_json::{Value, json};
use x1_support::RecordingEnqueuer;

const TABLE: &str = "IntegrationWhatsapp";

fn make_auth(token: &str) -> Value {
    json!({
        "authType": "oauth2",
        "clientId": "client",
        "clientSecret": "secret",
        "tokens": { "accessToken": token },
        "metadata": { "isManual": false, "wabaId": "waba-1" },
    })
}

fn aad(waba: &str) -> String {
    format!("whatsapp-waba:{WS}:{waba}")
}

/// The `fb_exchange_token` of a request, answered as `refreshed-<token>`
/// (`fail-token`: a Graph outage).
fn exchange(
    r: &chatbotx_business::connection::credentials::UpstreamRequest,
) -> chatbotx_business::connection::credentials::UpstreamReply {
    let url = url::Url::parse(&r.url).expect("url");
    let token = url
        .query_pairs()
        .find(|(k, _)| k == "fb_exchange_token")
        .map(|(_, v)| v.into_owned())
        .unwrap_or_default();
    if token == "fail-token" {
        graph_error(503, "Meta unavailable", 2, None)
    } else {
        ok(json!({ "access_token": format!("refreshed-{token}") }))
    }
}

/// Phone rows `(id, wabaId)` and WABA credentials `(wabaId, accessToken)`.
async fn seed(
    phones: &'static [(i64, &'static str)],
    wabas: &'static [(&'static str, &'static str)],
) -> common::TestDb {
    let db = two_workspaces().await;
    let sealed: Vec<(String, String)> = wabas
        .iter()
        .map(|(waba, token)| {
            let credential = json!({ "accessToken": token, "apiVersion": "v22.0" });
            let data = process_encryption()
                .encrypt_object(&credential, Some(&aad(waba)))
                .expect("encrypt");
            (
                (*waba).to_owned(),
                serde_json::to_string(&data).expect("json"),
            )
        })
        .collect();
    setup(&db, move |tx| {
        for (id, waba) in phones {
            channel_row(tx, TABLE, *id, WS, &format!("phone-{id}"), &make_auth(&format!("phone-token-{id}")))?;
            tx.execute(r#"UPDATE "IntegrationWhatsapp" SET "wabaId" = ?2 WHERE "id" = ?1"#, params![id, waba])?;
        }
        for (waba, credential) in sealed {
            tx.execute(
                r#"INSERT INTO "WhatsappBusinessAccount" ("workspaceId", "wabaId", "businessId", "credential",
                       "grantedScopes", "scopeCheckedAt", "revision")
                   VALUES (?1, ?2, ?3, ?4, '["whatsapp_business_manage_events"]', 1788825600000, 4)"#,
                params![WS, waba, format!("business-{waba}"), credential],
            )?;
        }
        Ok(())
    })
    .await;
    db
}

/// `(decrypted credential, revision)` of a WABA, if any.
async fn waba(
    db: &chatbotx_db::Db,
    waba: &'static str,
) -> Option<(Value, i64, Value, Option<i64>)> {
    let row: Option<(String, i64, String, Option<i64>)> = db
        .read(move |conn| {
            use chatbotx_db::rusqlite::OptionalExtension as _;
            Ok(conn
                .query_row(
                    r#"SELECT "credential", "revision", "grantedScopes", "scopeCheckedAt"
                       FROM "WhatsappBusinessAccount" WHERE "workspaceId" = ?1 AND "wabaId" = ?2"#,
                    params![WS, waba],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?)
        })
        .await
        .expect("read waba");
    row.map(|(credential, revision, scopes, checked)| {
        let data: EncryptedData = serde_json::from_str(&credential).expect("sealed");
        let plain: Value = process_encryption()
            .decrypt_object(&data, Some(&aad(waba)))
            .expect("decrypt");
        (
            plain,
            revision,
            serde_json::from_str(&scopes).unwrap(),
            checked,
        )
    })
}

async fn run(db: &chatbotx_db::Db, http: &FakeHttp, audit: &AuditService) {
    let deps = RefreshDeps {
        db,
        http,
        audit,
        buses: None,
    };
    refresh_scheduled(deps, Some(&["whatsapp".to_owned()])).await;
}

fn audit_details(jobs: &RecordingEnqueuer) -> Vec<String> {
    jobs.calls()
        .into_iter()
        .filter_map(|c| c.data["data"]["detail"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn refreshes_every_phone_row_and_one_waba_credential_for_shared_waba_rows() {
    let db = seed(
        &[(77_001, "waba-1"), (77_002, "waba-1")],
        &[("waba-1", "waba-token")],
    )
    .await;
    let http = FakeHttp::new(exchange);
    let jobs = RecordingEnqueuer::new();
    run(&db, &http, &AuditService::new(jobs.clone())).await;
    for id in [77_001, 77_002] {
        let (auth, error) = auth_row(&db, TABLE, id).await;
        assert_eq!(error, None);
        assert_eq!(
            auth["tokens"]["accessToken"],
            format!("refreshed-phone-token-{id}")
        );
    }
    let (credential, revision, scopes, checked) = waba(&db, "waba-1").await.expect("waba row");
    assert_eq!(
        credential,
        json!({ "accessToken": "refreshed-waba-token", "apiVersion": "v22.0" })
    );
    assert_eq!(revision, 5);
    assert_eq!(scopes, json!(["whatsapp_business_manage_events"]));
    assert_eq!(checked, Some(1_788_825_600_000));
    let waba_calls = http
        .urls()
        .into_iter()
        .filter(|u| u.contains("fb_exchange_token=waba-token"))
        .count();
    assert_eq!(waba_calls, 1);
    let details = audit_details(&jobs);
    assert!(
        details
            .iter()
            .any(|d| d == "auto-refreshed the WhatsApp channel token")
    );
    assert!(
        details
            .iter()
            .any(|d| d == "auto-refreshed the WhatsApp Business Account credential")
    );
}

#[tokio::test]
async fn keeps_refreshing_phone_rows_without_creating_a_missing_waba_record() {
    let db = seed(&[(77_101, "waba-1")], &[]).await;
    let http = FakeHttp::new(exchange);
    run(&db, &http, &AuditService::disabled()).await;
    assert_eq!(
        auth_row(&db, TABLE, 77_101).await.0["tokens"]["accessToken"],
        "refreshed-phone-token-77101"
    );
    assert_eq!(http.seen().len(), 1);
    assert!(waba(&db, "waba-1").await.is_none());
}

#[tokio::test]
async fn continues_refreshing_other_wabas_after_one_waba_refresh_fails() {
    let db = seed(
        &[(77_201, "waba-1"), (77_202, "waba-2")],
        &[("waba-1", "fail-token"), ("waba-2", "good-token")],
    )
    .await;
    let http = FakeHttp::new(exchange);
    run(&db, &http, &AuditService::disabled()).await;
    let (first, first_revision, _, _) = waba(&db, "waba-1").await.unwrap();
    let (second, second_revision, _, _) = waba(&db, "waba-2").await.unwrap();
    assert_eq!(first["accessToken"], "fail-token");
    assert_eq!(first_revision, 4);
    assert_eq!(second["accessToken"], "refreshed-good-token");
    assert_eq!(second_revision, 5);
}

#[tokio::test]
async fn skips_manual_phone_rows_and_their_waba() {
    let db = seed(&[(77_301, "waba-1")], &[("waba-1", "waba-token")]).await;
    setup(&db, |tx| {
        tx.execute(
            r#"UPDATE "IntegrationWhatsapp" SET "auth" = json_set("auth", '$.metadata.isManual', json('true')) WHERE "id" = 77301"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let http = FakeHttp::new(exchange);
    run(&db, &http, &AuditService::disabled()).await;
    assert!(http.seen().is_empty());
    assert_eq!(waba(&db, "waba-1").await.unwrap().1, 4);
}
