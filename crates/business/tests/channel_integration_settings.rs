//! Q.F2-settings: the TikTok / Zalo channel settings tables and actions
//! (`chatbotx_business::channel_integration`): `listIntegrationTiktoks`
//! (`toResource`), the disconnects, and `refreshTiktokTokenAction`, over a
//! migrated database and a scripted TikTok API (the originals mock the
//! services and `refreshAccessToken`).

mod common;
mod x1_support;

use std::sync::{Arc, Mutex};

use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_business::channel_integration::{disconnect, refresh_tiktok_token, settings_list};
use chatbotx_business::connection::credentials::{Upstream, UpstreamReply, UpstreamRequest};
use chatbotx_db::rusqlite::params;
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::{Value, json};
use x1_support::RecordingEnqueuer;

const BASE: &str = "https://tiktok.test/";

/// Answers each request with the next scripted reply and records it.
struct Script {
    replies: Mutex<Vec<UpstreamReply>>,
    seen: Mutex<Vec<UpstreamRequest>>,
}

impl Script {
    fn with(replies: Vec<UpstreamReply>) -> Script {
        Script {
            replies: Mutex::new(replies.into_iter().rev().collect()),
            seen: Mutex::new(Vec::new()),
        }
    }
}

impl Upstream for Script {
    async fn send(&self, request: UpstreamRequest) -> UpstreamReply {
        self.seen.lock().expect("lock").push(request);
        self.replies
            .lock()
            .expect("lock")
            .pop()
            .unwrap_or_else(|| Err("no scripted reply".into()))
    }
}

fn audit() -> (AuditService, Arc<RecordingEnqueuer>) {
    let jobs = RecordingEnqueuer::new();
    (AuditService::new(jobs.clone()), jobs)
}

fn actor() -> AuditActor {
    AuditActor {
        user_id: Some("1001".into()),
        workspace_id: Some(WS.to_string()),
        ..AuditActor::default()
    }
}

/// TikTok row 801 (inbox 811, Connection 821) and Zalo row 901 (inbox 911,
/// Connection 921, one `TagChannel`) in `WS`.
async fn seeded(auth: Value) -> common::TestDb {
    let db = two_workspaces().await;
    let auth = auth.to_string();
    setup(&db, move |tx| {
        tx.execute(
            r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId")
               VALUES (811, 'TT', 'tiktok', 'open-1', ?1), (911, 'OA', 'zalo', 'oa-1', ?1)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationTiktok" ("id", "auth", "openId", "name", "workspaceId", "inboxId")
               VALUES (801, ?2, 'open-1', 'TikTok account', ?1, 811)"#,
            params![WS, auth],
        )?;
        tx.execute(
            r#"INSERT INTO "IntegrationZalo" ("id", "auth", "oaId", "name", "workspaceId", "inboxId")
               VALUES (901, '{}', 'oa-1', 'Zalo OA', ?1, 911)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Connection" ("id", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId", "displayName")
               VALUES (821, ?1, 'tiktok', 'channel', 'tiktok', 811, 'open-1', 'TT'),
                      (921, ?1, 'zalo', 'channel', 'zalo', 911, 'oa-1', 'OA')"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "Tag" ("id", "name", "workspaceId") VALUES (931, 'vip', ?1)"#,
            params![WS],
        )?;
        tx.execute(
            r#"INSERT INTO "TagChannel" ("id", "workspaceId", "tagId", "channelType", "integrationId", "externalLabelId")
               VALUES (941, ?1, 931, 'zalo', 901, 'label-1')"#,
            params![WS],
        )?;
        Ok(())
    })
    .await;
    db
}

fn tiktok_auth() -> Value {
    json!({
        "clientId": "ck",
        "clientSecret": "cs",
        "tokens": { "accessToken": "old-at", "refreshToken": "old-rt" },
        "metadata": { "openId": "open-1" },
    })
}

async fn text(db: &common::TestDb, sql: &'static str) -> Option<String> {
    db.read(move |conn| Ok(conn.query_row(sql, [], |r| r.get::<_, Option<String>>(0))?))
        .await
        .expect("read")
}

/// Port of: apps/builder/__tests__/integration-tiktok-queries.test.ts:24 "listIntegrationTiktoks > includes inboxId in the client-safe resource"
#[tokio::test]
async fn list_integration_tiktoks_includes_inbox_id_in_the_client_safe_resource() {
    let db = seeded(json!({ "metadata": {} })).await;
    let rows = settings_list(&db, "tiktok", WS).await.expect("list");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["id"], json!("801"));
    assert_eq!(rows[0]["inboxId"], json!("811"));
    // No scopes stored: the re-authorize warning; no cached C2M status.
    assert_eq!(rows[0]["needsReauthorization"], json!(true));
    assert_eq!(rows[0]["commentToMessageStatus"], Value::Null);
    // The credentials never leave the server.
    assert!(rows[0].get("auth").is_none());
    assert!(
        settings_list(&db, "tiktok", OTHER_WS)
            .await
            .expect("other")
            .is_empty()
    );
}

/// Own: a full comment grant needs no re-authorization; the cached C2M
/// status is read from `auth.metadata.commentToMessage`.
#[tokio::test]
async fn list_integration_tiktoks_reads_scopes_and_the_cached_comment_to_message_status() {
    let db = seeded(json!({ "metadata": {
        "scopes": ["comment.list", "comment.list.manage", "video.list"],
        "commentToMessage": { "status": "ENABLE" },
    } }))
    .await;
    let rows = settings_list(&db, "tiktok", WS).await.expect("list");
    assert_eq!(rows[0]["needsReauthorization"], json!(false));
    assert_eq!(rows[0]["commentToMessageStatus"], json!("ENABLE"));
    let zalo = settings_list(&db, "zalo", WS).await.expect("zalo");
    assert_eq!(zalo[0]["oaId"], json!("oa-1"));
    assert_eq!(zalo[0]["syncTagEnabledAt"], Value::Null);
}

/// Own: `disconnectZaloAction` removes the `TagChannel` rows and the
/// satellite, and disconnects the connection and inbox; another workspace's
/// row is not found.
#[tokio::test]
async fn disconnect_zalo_cleans_tag_channels_and_disconnects_the_connection() {
    let db = seeded(tiktok_auth()).await;
    let error = disconnect(&db, "zalo", OTHER_WS, 901)
        .await
        .expect_err("scoped");
    assert_eq!(error.message(), "Zalo channel not found");
    disconnect(&db, "zalo", WS, 901).await.expect("disconnect");
    assert_eq!(
        text(&db, r#"SELECT CAST(COUNT(*) AS TEXT) FROM "TagChannel""#).await,
        Some("0".into())
    );
    assert_eq!(
        text(
            &db,
            r#"SELECT CAST(COUNT(*) AS TEXT) FROM "IntegrationZalo""#
        )
        .await,
        Some("0".into())
    );
    assert_eq!(
        text(&db, r#"SELECT "status" FROM "Connection" WHERE "id" = 921"#).await,
        Some("disconnected".into())
    );
    assert_eq!(
        text(&db, r#"SELECT "status" FROM "Inbox" WHERE "id" = 911"#).await,
        Some("disconnected".into())
    );
    disconnect(&db, "tiktok", WS, 801).await.expect("tiktok");
    assert_eq!(
        text(&db, r#"SELECT "status" FROM "Connection" WHERE "id" = 821"#).await,
        Some("disconnected".into())
    );
}

/// Own: `refreshTiktokToken` stores the new tokens, clears the refresh
/// error, marks the connection connected and audits.
#[tokio::test]
async fn refresh_tiktok_token_stores_the_new_tokens_and_audits() {
    let db = seeded(tiktok_auth()).await;
    let (audit, jobs) = audit();
    let upstream = Script::with(vec![Ok((
        200,
        json!({ "code": 0, "data": {
            "access_token": "new-at", "refresh_token": "new-rt",
            "expires_in": 86400, "refresh_token_expires_in": 31536000,
        } })
        .to_string(),
    ))]);
    with_audit_context(
        actor(),
        refresh_tiktok_token(&db, &upstream, &audit, BASE, WS, 801),
    )
    .await
    .expect("refresh");
    let seen = upstream.seen.lock().expect("lock").clone();
    assert_eq!(seen[0].url, format!("{BASE}tt_user/oauth2/refresh_token/"));
    let body: Value = serde_json::from_str(seen[0].body.as_deref().unwrap_or("")).unwrap();
    assert_eq!(body["grant_type"], json!("refresh_token"));
    assert_eq!(body["refresh_token"], json!("old-rt"));
    let auth: Value = serde_json::from_str(
        &text(
            &db,
            r#"SELECT "auth" FROM "IntegrationTiktok" WHERE "id" = 801"#,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(auth["tokens"]["accessToken"], json!("new-at"));
    assert_eq!(auth["tokens"]["refreshToken"], json!("new-rt"));
    assert!(auth["tokens"]["expiresAt"].is_string());
    assert_eq!(auth["clientSecret"], json!("cs"));
    assert!(
        text(
            &db,
            r#"SELECT CAST("authExpiresAt" AS TEXT) FROM "Connection" WHERE "id" = 821"#
        )
        .await
        .is_some()
    );
    assert!(
        jobs.calls()
            .iter()
            .any(|c| c.data["data"]["detail"] == json!("refreshed the TikTok channel token"))
    );
}

/// Own: a transient refresh failure stores `tokenRefreshError` and degrades
/// the connection; a revoked token marks it `needs_reauth`.
#[tokio::test]
async fn refresh_tiktok_token_failures_mark_the_connection() {
    let db = seeded(tiktok_auth()).await;
    let (audit, _) = audit();
    let upstream = Script::with(vec![
        Ok((
            200,
            json!({ "code": 50002, "message": "server busy" }).to_string(),
        )),
        Ok((
            200,
            json!({ "code": 40105, "message": "access_token_invalid" }).to_string(),
        )),
    ]);
    let error = refresh_tiktok_token(&db, &upstream, &audit, BASE, WS, 801)
        .await
        .expect_err("transient");
    assert_eq!(error.message(), "Failed to refresh TikTok token");
    assert_eq!(
        text(
            &db,
            r#"SELECT "tokenRefreshError" FROM "IntegrationTiktok" WHERE "id" = 801"#
        )
        .await,
        Some("server busy".into())
    );
    assert_eq!(
        text(
            &db,
            r#"SELECT "status" || ':' || "statusReason" FROM "Connection" WHERE "id" = 821"#
        )
        .await,
        Some("degraded:refresh_failed".into())
    );
    refresh_tiktok_token(&db, &upstream, &audit, BASE, WS, 801)
        .await
        .expect_err("revoked");
    assert_eq!(
        text(
            &db,
            r#"SELECT "status" || ':' || "statusReason" FROM "Connection" WHERE "id" = 821"#
        )
        .await,
        Some("needs_reauth:token_revoked".into())
    );
}

/// Own: no stored refresh token is an error before any TikTok call.
#[tokio::test]
async fn refresh_tiktok_token_needs_a_refresh_token() {
    let db = seeded(json!({ "tokens": { "accessToken": "at" } })).await;
    let (audit, _) = audit();
    let upstream = Script::with(vec![]);
    let error = refresh_tiktok_token(&db, &upstream, &audit, BASE, WS, 801)
        .await
        .expect_err("no token");
    assert_eq!(error.message(), "TikTok refresh token not available");
    assert!(upstream.seen.lock().expect("lock").is_empty());
}
