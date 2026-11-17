//! `/developer/queues` (Q.D11 slice 9), the jobs admin page that replaces
//! Bull Board: 404 for anyone but a super admin (as the builder route), the
//! counts per queue and state, the failed list with its reason (escaped),
//! retry back to waiting and remove.

use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::jobs_admin::{JobsAdminState, router};
use chatbotx_api::mail::NoMailer;
use chatbotx_api::session::{AuthSettings, AuthState, TrustedOriginSettings, sign_cookie_value};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::{JobStore, SystemClock};
use tower::ServiceExt as _;

const SECRET: &str = "jobs-admin-secret";
const NOW_MS: i64 = 1_800_000_000_000;

fn now_ms() -> i64 {
    NOW_MS
}

async fn app() -> (axum::Router, Arc<Db>) {
    let db = Db::open_in_memory().expect("open db");
    db.migrate().await.expect("migrate");
    db.write(|tx| {
        for (id, email, token) in [(1, "admin@jobs.test", "admintoken"), (2, "user@jobs.test", "usertoken")] {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email", "emailVerified") VALUES (?1, ?2, 1)"#,
                params![id, email],
            )?;
            tx.execute(
                r#"INSERT INTO "Session" ("id", "token", "userId", "expiresAt") VALUES (?1, ?2, ?1, ?3)"#,
                params![id, token, NOW_MS + 60_000],
            )?;
        }
        tx.execute(r#"INSERT INTO "PlatformAdmin" ("id", "userId") VALUES (1, 1)"#, [])?;
        for (seq, job_id, state, reason) in [
            (1, "j-ok", "completed", None),
            (2, "j-bad", "failed", Some("boom <script>")),
            (3, "j-wait", "waiting", None),
        ] {
            tx.execute(
                "INSERT INTO jobs (queue, name, job_id, data, opts, state, ready_seq, run_at_ms, created_at_ms, \
                 attempts_made, finished_at_ms, failed_reason) VALUES ('default', 'sendEmail', ?1, '{}', '{}', ?2, ?3, 0, 0, 3, ?4, ?5)",
                params![job_id, state, seq, (state != "waiting").then_some(NOW_MS), reason],
            )?;
        }
        Ok(())
    })
    .await
    .expect("seed");
    let db = Arc::new(db);
    let auth = Arc::new(AuthState {
        db: Arc::clone(&db),
        settings: AuthSettings {
            secret: SECRET.to_string(),
            base_url: "http://app".to_string(),
            sign_up_disabled: false,
            trusted_origins: TrustedOriginSettings {
                broker_url: None,
                builder_url: "http://app".to_string(),
                next_phase: None,
            },
            extra_trusted_origins: None,
        },
        mailer: Arc::new(NoMailer),
        now_ms,
    });
    let store = Arc::new(
        JobStore::open(Arc::clone(&db), Arc::new(SystemClock))
            .await
            .expect("store"),
    );
    (router(Arc::new(JobsAdminState { auth, store })), db)
}

async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    origin: Option<&str>,
) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "app");
    if let Some(token) = token {
        builder = builder.header(
            header::COOKIE,
            format!(
                "better-auth.session_token={}",
                sign_cookie_value(token, SECRET)
            ),
        );
    }
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

async fn job_state(db: &Db, job_id: &'static str) -> Option<String> {
    db.read(move |conn| {
        Ok(conn
            .query_row(
                "SELECT state FROM jobs WHERE queue = 'default' AND job_id = ?1",
                [job_id],
                |r| r.get(0),
            )
            .ok())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn only_a_super_admin_sees_the_page() {
    let (app, _) = app().await;
    for token in [None, Some("usertoken"), Some("missing")] {
        for (method, path) in [
            ("GET", "/developer/queues"),
            ("POST", "/developer/queues/default/j-bad/retry"),
            ("POST", "/developer/queues/default/j-bad/remove"),
        ] {
            let (status, body) = send(&app, method, path, token, None).await;
            assert_eq!(
                (status, body.as_str()),
                (StatusCode::NOT_FOUND, "Not found"),
                "{token:?} {method} {path}"
            );
        }
    }
}

#[tokio::test]
async fn lists_counts_and_the_failed_jobs_with_their_reason() {
    let (app, _) = app().await;
    let (status, body) = send(
        &app,
        "GET",
        "/developer/queues?queue=default",
        Some("admintoken"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    // default: 1 waiting, 1 completed, 1 failed.
    assert!(
        body.contains(r#"<a href="/developer/queues?queue=default">default</a></td><td class="n">1</td><td class="n">0</td><td class="n">0</td><td class="n">0</td><td class="n">1</td><td class="n">1</td>"#),
        "{body}"
    );
    assert!(body.contains("<td>j-bad</td><td>sendEmail</td>"), "{body}");
    assert!(body.contains("boom &lt;script&gt;"), "{body}");
    assert!(!body.contains("<td>j-ok</td>"), "{body}");
    assert!(
        body.contains(r#"action="/developer/queues/default/j-bad/retry""#),
        "{body}"
    );
}

#[tokio::test]
async fn retry_moves_a_failed_job_back_to_waiting_and_remove_deletes_it() {
    let (app, db) = app().await;
    // A cross-site post is refused.
    let (status, _) = send(
        &app,
        "POST",
        "/developer/queues/default/j-bad/retry",
        Some("admintoken"),
        Some("https://evil.test"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(job_state(&db, "j-bad").await.as_deref(), Some("failed"));

    let (status, _) = send(
        &app,
        "POST",
        "/developer/queues/default/j-bad/retry",
        Some("admintoken"),
        Some("http://app"),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(job_state(&db, "j-bad").await.as_deref(), Some("waiting"));
    let reason = db
        .read(|conn| {
            Ok(conn.query_row(
                "SELECT failed_reason IS NULL AND finished_at_ms IS NULL AND attempts_made = 3 FROM jobs WHERE job_id = 'j-bad'",
                [],
                |r| r.get::<_, bool>(0),
            )?)
        })
        .await
        .unwrap();
    assert!(reason);

    let (status, _) = send(
        &app,
        "POST",
        "/developer/queues/default/j-ok/remove",
        Some("admintoken"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(job_state(&db, "j-ok").await, None);
    // An unknown queue is 404.
    let (status, _) = send(
        &app,
        "POST",
        "/developer/queues/nope/j-ok/remove",
        Some("admintoken"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
