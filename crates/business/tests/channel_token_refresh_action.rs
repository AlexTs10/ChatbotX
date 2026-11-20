//! Port of `apps/builder/__tests__/refresh-all-channel-tokens-action.test.ts`
//! (the provider branches): `refresh_for_workspaces` over a migrated database
//! and a scripted provider. The mocked `markTokenRefreshError` is the stored
//! `tokenRefreshError` and the connection status.
//! ponytail: the `logMessengerWelcomeProfile` cases are not ported (the
//! welcome-profile log is not in the Rust port; DECISIONS.md).

mod channel_fake;
mod common;
mod x1_support;

use channel_fake::{
    FakeHttp, auth_row, channel_row, connection_row, connection_status, graph_error, ok, status,
};
use chatbotx_business::audit::{AuditActor, AuditService, with_audit_context};
use chatbotx_business::channel_token_refresh::{
    RefreshDeps, RefreshSummary, refresh_for_workspaces,
};
use chatbotx_business::connection::credentials::UpstreamReply;
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::{Value, json};
use x1_support::RecordingEnqueuer;

fn auth() -> Value {
    json!({
        "authType": "oauth2",
        "clientId": "client-1",
        "clientSecret": "secret-1",
        "tokens": { "accessToken": "old-token", "refreshToken": "r" },
    })
}

async fn run(
    db: &chatbotx_db::Db,
    http: &FakeHttp,
    audit: &AuditService,
    workspaces: &[i64],
) -> RefreshSummary {
    let deps = RefreshDeps {
        db,
        http,
        audit,
        buses: None,
    };
    let actor = AuditActor {
        user_id: Some("1001".into()),
        ..AuditActor::default()
    };
    with_audit_context(actor, refresh_for_workspaces(deps, workspaces))
        .await
        .expect("refresh")
}

#[tokio::test]
async fn messenger_refreshes_and_audits_the_user_action() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        channel_row(tx, "IntegrationMessenger", 78_001, WS, "page-1", &auth())
    })
    .await;
    let http = FakeHttp::new(|_| ok(json!({ "access_token": "new-token" })));
    let jobs = RecordingEnqueuer::new();
    let summary = run(&db, &http, &AuditService::new(jobs.clone()), &[WS]).await;
    assert_eq!(
        summary,
        RefreshSummary {
            refreshed: 1,
            failed: 0
        }
    );
    assert_eq!(
        auth_row(&db, "IntegrationMessenger", 78_001).await.0["tokens"]["accessToken"],
        "new-token"
    );
    let audit = &jobs.calls()[0].data["data"];
    assert_eq!(audit["detail"], "refreshed the Messenger channel token");
    assert_eq!(audit["userId"], "1001");
    assert_eq!(audit["workspaceId"], WS.to_string());
}

#[tokio::test]
async fn messenger_reports_a_failed_refresh() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        channel_row(tx, "IntegrationMessenger", 78_101, WS, "page-1", &auth())
    })
    .await;
    let http = FakeHttp::new(|_| graph_error(400, "token expired", 1, None));
    let summary = run(&db, &http, &AuditService::disabled(), &[WS]).await;
    assert_eq!(
        summary,
        RefreshSummary {
            refreshed: 0,
            failed: 1
        }
    );
    assert_eq!(
        auth_row(&db, "IntegrationMessenger", 78_101)
            .await
            .1
            .as_deref(),
        Some("token expired")
    );
}

#[tokio::test]
async fn only_the_given_workspaces_are_refreshed() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        channel_row(
            tx,
            "IntegrationMessenger",
            78_201,
            OTHER_WS,
            "page-1",
            &auth(),
        )
    })
    .await;
    let http = FakeHttp::new(|_| ok(json!({ "access_token": "new-token" })));
    let summary = run(&db, &http, &AuditService::disabled(), &[WS]).await;
    assert_eq!(summary, RefreshSummary::default());
    assert!(http.seen().is_empty());
}

struct Case {
    name: &'static str,
    table: &'static str,
    provider: &'static str,
    ig_type: Option<&'static str>,
    revoked: fn() -> UpstreamReply,
    transient: fn() -> UpstreamReply,
}

const CASES: [Case; 5] = [
    Case {
        name: "Zalo",
        table: "IntegrationZalo",
        provider: "zalo",
        ig_type: None,
        revoked: || ok(json!({ "error": -124, "message": "Zalo provider rejected the refresh" })),
        transient: || ok(json!({ "error": -32, "message": "transient failure" })),
    },
    Case {
        name: "TikTok",
        table: "IntegrationTiktok",
        provider: "tiktok",
        ig_type: None,
        revoked: || status(401, json!({})),
        transient: || ok(json!({ "code": 40100, "message": "transient failure" })),
    },
    Case {
        name: "Instagram",
        table: "IntegrationInstagram",
        provider: "instagram",
        ig_type: Some("instagram"),
        revoked: || {
            graph_error(
                400,
                "Instagram provider rejected the refresh",
                190,
                Some(460),
            )
        },
        transient: || graph_error(500, "transient failure", 2, None),
    },
    Case {
        name: "Instagram (Facebook-linked)",
        table: "IntegrationInstagram",
        provider: "instagramFacebook",
        ig_type: Some("facebook"),
        revoked: || {
            graph_error(
                400,
                "Instagram provider rejected the refresh",
                190,
                Some(463),
            )
        },
        transient: || graph_error(500, "transient failure", 2, None),
    },
    Case {
        name: "WhatsApp",
        table: "IntegrationWhatsapp",
        provider: "whatsapp",
        ig_type: None,
        revoked: || graph_error(400, "WhatsApp provider rejected the refresh", 190, None),
        transient: || graph_error(500, "transient failure", 2, None),
    },
];

async fn seed(case: &Case, id: i64) -> common::TestDb {
    let db = two_workspaces().await;
    let (table, provider, ig_type) = (case.table, case.provider, case.ig_type);
    setup(&db, move |tx| {
        channel_row(tx, table, id, WS, "src-1", &auth())?;
        if let Some(kind) = ig_type {
            tx.execute(
                r#"UPDATE "IntegrationInstagram" SET "type" = ?2 WHERE "id" = ?1"#,
                chatbotx_db::rusqlite::params![id, kind],
            )?;
        }
        connection_row(tx, id, WS, provider, id, "src-1")
    })
    .await;
    db
}

#[tokio::test]
async fn reports_a_failure_and_records_the_token_refresh_error_with_the_revoked_signal() {
    for (i, case) in CASES.iter().enumerate() {
        let id = 78_300 + i as i64;
        let db = seed(case, id).await;
        let revoked = case.revoked;
        let http = FakeHttp::new(move |_| revoked());
        let summary = run(&db, &http, &AuditService::disabled(), &[WS]).await;
        assert_eq!(
            summary,
            RefreshSummary {
                refreshed: 0,
                failed: 1
            },
            "{}",
            case.name
        );
        assert!(
            auth_row(&db, case.table, id).await.1.is_some(),
            "{}",
            case.name
        );
        assert_eq!(
            connection_status(&db, id).await.0,
            "needs_reauth",
            "{}",
            case.name
        );
    }
}

#[tokio::test]
async fn reports_is_revoked_false_when_the_revocation_check_says_so() {
    for (i, case) in CASES.iter().enumerate() {
        let id = 78_400 + i as i64;
        let db = seed(case, id).await;
        let transient = case.transient;
        let http = FakeHttp::new(move |_| transient());
        run(&db, &http, &AuditService::disabled(), &[WS]).await;
        assert_eq!(
            auth_row(&db, case.table, id).await.1.as_deref(),
            Some("transient failure"),
            "{}",
            case.name
        );
        assert_eq!(
            connection_status(&db, id).await,
            ("degraded".into(), Some("refresh_failed".into())),
            "{}",
            case.name
        );
    }
}
