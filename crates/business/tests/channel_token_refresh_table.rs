//! Port of `apps/worker/__tests__/refresh-channel-tokens-table.test.ts`: the
//! scheduled per-row refresh of Messenger, Instagram (both logins) and Zalo
//! over a migrated database and a scripted provider. The mocked
//! `markTokenRefreshError` / `updateAuth` are the stored
//! `tokenRefreshError` / `auth` columns; the `runExclusive` failure is a
//! held `auth:refresh:*` lock.
//! ponytail: the "markError throw" case has no Rust seam (the mark is one
//! SQL write whose error is logged); row isolation is the same path as the
//! row-1 failure test.

mod channel_fake;
mod common;

use channel_fake::{
    FakeHttp, auth_row, channel_row, connection_row, connection_status, graph_error, ok, status,
    text,
};
use chatbotx_business::audit::AuditService;
use chatbotx_business::channel_token_refresh::{RefreshDeps, refresh_locks, refresh_scheduled};
use chatbotx_business::connection::credentials::{UpstreamReply, UpstreamRequest};
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    schedule: &'static str,
    table: &'static str,
    provider: &'static str,
    lock: &'static str,
    ig_type: Option<&'static str>,
    zalo: bool,
}

const CASES: [Case; 4] = [
    Case {
        name: "Messenger",
        schedule: "messenger",
        table: "IntegrationMessenger",
        provider: "messenger",
        lock: "messenger",
        ig_type: None,
        zalo: false,
    },
    Case {
        name: "Instagram",
        schedule: "instagram",
        table: "IntegrationInstagram",
        provider: "instagram",
        lock: "instagram",
        ig_type: Some("instagram"),
        zalo: false,
    },
    Case {
        name: "Instagram (Facebook-linked)",
        schedule: "instagram",
        table: "IntegrationInstagram",
        provider: "instagramFacebook",
        lock: "instagramFacebook",
        ig_type: Some("facebook"),
        zalo: false,
    },
    Case {
        name: "Zalo",
        schedule: "zalo",
        table: "IntegrationZalo",
        provider: "zalo",
        lock: "zalo",
        ig_type: None,
        zalo: true,
    },
];

fn auth(n: i64) -> Value {
    json!({
        "authType": "oauth2",
        "clientId": "client",
        "clientSecret": "secret",
        "tokens": { "accessToken": format!("old-{n}"), "refreshToken": format!("old-{n}") },
    })
}

impl Case {
    fn revoked(self) -> UpstreamReply {
        if self.zalo {
            ok(json!({ "error": -124, "message": "revoked" }))
        } else {
            graph_error(400, "revoked", 190, Some(460))
        }
    }

    fn transient(self, message: &str) -> UpstreamReply {
        if self.zalo {
            ok(json!({ "error": -32, "message": message }))
        } else {
            status(500, json!({ "error": { "message": message, "code": 4 } }))
        }
    }
}

fn success(n: i64) -> UpstreamReply {
    ok(json!({
        "access_token": format!("new-{n}"),
        "refresh_token": format!("new-refresh-{n}"),
        "expires_in": 3600,
    }))
}

/// Rows `base + 1` (workspace `WS`) and `base + 2` (`OTHER_WS`), each with a
/// connected `Connection` of the same id. The ids are unique per test: the
/// refresh locks are process-wide.
async fn seed(case: Case, base: i64) -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, move |tx| {
        for (n, ws) in [(1, WS), (2, OTHER_WS)] {
            let id = base + n;
            let source = format!("src-{id}");
            channel_row(tx, case.table, id, ws, &source, &auth(n))?;
            if let Some(kind) = case.ig_type {
                tx.execute(
                    r#"UPDATE "IntegrationInstagram" SET "type" = ?2 WHERE "id" = ?1"#,
                    chatbotx_db::rusqlite::params![id, kind],
                )?;
            }
            connection_row(tx, id, ws, case.provider, id, &source)?;
        }
        Ok(())
    })
    .await;
    db
}

async fn run(db: &chatbotx_db::Db, http: &FakeHttp, case: Case) {
    let audit = AuditService::disabled();
    let deps = RefreshDeps {
        db,
        http,
        audit: &audit,
        buses: None,
    };
    refresh_scheduled(deps, Some(&[case.schedule.to_owned()])).await;
}

fn row(request: &UpstreamRequest) -> i64 {
    if text(request).contains("old-1") {
        1
    } else {
        2
    }
}

#[tokio::test]
async fn marks_the_row_unhealthy_with_is_revoked_true_when_the_provider_confirms_the_token_was_revoked()
 {
    for (i, case) in CASES.into_iter().enumerate() {
        let base = 71_000 + 10 * i as i64;
        let db = seed(case, base).await;
        let http = FakeHttp::new(move |r| {
            if row(r) == 1 {
                case.revoked()
            } else {
                success(2)
            }
        });
        run(&db, &http, case).await;
        assert_eq!(
            auth_row(&db, case.table, base + 1).await.1.as_deref(),
            Some("revoked"),
            "{}",
            case.name
        );
        assert_eq!(
            connection_status(&db, base + 1).await,
            ("needs_reauth".into(), Some("token_revoked".into())),
            "{}",
            case.name
        );
    }
}

#[tokio::test]
async fn degrades_with_is_revoked_false_on_a_transient_refresh_failure() {
    for (i, case) in CASES.into_iter().enumerate() {
        let base = 72_000 + 10 * i as i64;
        let db = seed(case, base).await;
        let http = FakeHttp::new(move |r| {
            if row(r) == 1 {
                case.transient("rate limited")
            } else {
                success(2)
            }
        });
        run(&db, &http, case).await;
        assert_eq!(
            auth_row(&db, case.table, base + 1).await.1.as_deref(),
            Some("rate limited"),
            "{}",
            case.name
        );
        assert_eq!(
            connection_status(&db, base + 1).await,
            ("degraded".into(), Some("refresh_failed".into())),
            "{}",
            case.name
        );
    }
}

#[tokio::test]
async fn isolates_a_row_1_failure_row_2_still_refreshes_in_the_same_batch() {
    for (i, case) in CASES.into_iter().enumerate() {
        let base = 73_000 + 10 * i as i64;
        let db = seed(case, base).await;
        let http = FakeHttp::new(move |r| {
            if row(r) == 1 {
                case.transient("boom")
            } else {
                success(2)
            }
        });
        run(&db, &http, case).await;
        let (_, first_error) = auth_row(&db, case.table, base + 1).await;
        let (second_auth, second_error) = auth_row(&db, case.table, base + 2).await;
        assert_eq!(first_error.as_deref(), Some("boom"), "{}", case.name);
        assert_eq!(second_error, None, "{}", case.name);
        assert_eq!(
            second_auth["tokens"]["accessToken"], "new-2",
            "{}",
            case.name
        );
        assert_eq!(
            connection_status(&db, base + 2).await.0,
            "connected",
            "{}",
            case.name
        );
    }
}

#[tokio::test]
async fn a_lock_acquisition_failure_on_row_1_does_not_stop_row_2_from_refreshing() {
    for (i, case) in CASES.into_iter().enumerate() {
        let base = 74_000 + 10 * i as i64;
        let db = seed(case, base).await;
        let held = refresh_locks()
            .try_acquire(&format!("auth:refresh:{}:{}", case.lock, base + 1))
            .expect("hold row 1's lock");
        let http = FakeHttp::new(|r| success(row(r)));
        run(&db, &http, case).await;
        drop(held);
        let (first_auth, first_error) = auth_row(&db, case.table, base + 1).await;
        let (second_auth, _) = auth_row(&db, case.table, base + 2).await;
        assert_eq!(first_error, None, "{}", case.name);
        assert_eq!(
            first_auth["tokens"]["accessToken"], "old-1",
            "{}",
            case.name
        );
        assert_eq!(
            second_auth["tokens"]["accessToken"], "new-2",
            "{}",
            case.name
        );
        assert!(
            http.seen().iter().all(|r| !text(r).contains("old-1")),
            "{}: row 1 must not call the provider",
            case.name
        );
    }
}

#[tokio::test]
async fn a_refreshed_zalo_row_stores_both_new_tokens_and_an_expiry() {
    let case = CASES[3];
    let base = 75_000;
    let db = seed(case, base).await;
    let http = FakeHttp::new(|r| success(row(r)));
    run(&db, &http, case).await;
    let (auth, error) = auth_row(&db, case.table, base + 1).await;
    assert_eq!(error, None);
    assert_eq!(auth["tokens"]["accessToken"], "new-1");
    assert_eq!(auth["tokens"]["refreshToken"], "new-refresh-1");
    assert!(auth["tokens"]["expiresAt"].is_string());
}
