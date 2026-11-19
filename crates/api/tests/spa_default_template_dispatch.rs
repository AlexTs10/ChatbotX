//! Q.D4 follow-up: the SPA's first workspace (`pages.home`,
//! `createFirstWorkspaceForUser`) stages the root tenant's default template
//! and dispatches its install job after the commit (`dispatchStagedInstalls`).

use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use chatbotx_api::handlers::ApiState;
use chatbotx_api::mail::NoMailer;
use chatbotx_api::rpc::rpc_router;
use chatbotx_api::session::{AuthSettings, AuthState, TrustedOriginSettings, sign_cookie_value};
use chatbotx_api::spa::spa_router_with;
use chatbotx_business::template::tenant_default::{self, Scope};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_jobs::{AddResult, BoxFuture, Enqueuer, JobOptions, JobsError, NewJob, QueueName};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const SECRET: &str = "spa-d4-secret";
const NOW_MS: i64 = 1_800_000_000_000;
const ANN: i64 = 101;
const PUBLISHER: i64 = 102;

fn now_ms() -> i64 {
    NOW_MS
}

/// `queue.add` spy.
#[derive(Default)]
struct Jobs {
    calls: Mutex<Vec<(String, Value)>>,
}

impl Enqueuer for Jobs {
    fn add(
        &self,
        _queue: QueueName,
        name: &str,
        data: Value,
        opts: JobOptions,
    ) -> BoxFuture<'_, Result<AddResult, JobsError>> {
        self.calls
            .lock()
            .expect("lock")
            .push((name.to_owned(), data));
        let job_id = opts.job_id.unwrap_or_default();
        Box::pin(async move {
            Ok(AddResult {
                job_id,
                created: true,
            })
        })
    }

    fn add_bulk(
        &self,
        _queue: QueueName,
        jobs: Vec<NewJob>,
    ) -> BoxFuture<'_, Result<Vec<AddResult>, JobsError>> {
        let out = jobs
            .into_iter()
            .map(|job| AddResult {
                job_id: job.opts.job_id.unwrap_or_default(),
                created: true,
            })
            .collect();
        Box::pin(async move { Ok(out) })
    }
}

fn encryption() -> chatbotx_core::encryption::Encryption {
    chatbotx_core::encryption::Encryption::new(
        chatbotx_core::encryption::Keys::new(&"b2".repeat(32)).expect("test key"),
    )
}

/// `ANN` (signed in, no workspace) and a publisher workspace whose shared
/// template is the root tenant's default template.
async fn setup() -> (Arc<AuthState>, Arc<ApiState>, Arc<Jobs>) {
    let db = Arc::new(Db::open_in_memory().expect("open db"));
    db.migrate().await.expect("migrate");
    db.write(|tx| {
        tx.execute(
            r#"INSERT INTO "User" ("id", "name", "email", "emailVerified") VALUES (?1, 'Ann', 'ann@d4.test', 1), (?2, 'Pub', 'pub@d4.test', 1)"#,
            params![ANN, PUBLISHER],
        )?;
        tx.execute(
            r#"INSERT INTO "Session" ("id", "token", "userId", "expiresAt") VALUES (301, 'anntoken', ?1, ?2)"#,
            params![ANN, NOW_MS + 60_000],
        )?;
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "name", "ownerId", "tenantId") VALUES (900, 'Publisher', ?1, 1)"#,
            params![PUBLISHER],
        )?;
        tx.execute(
            r#"INSERT INTO "Template" ("id", "workspaceId", "tenantId", "name", "selection", "payload",
                 "categoryCounts", "formatVersion", "shareToken", "shareEnabled", "defaultPermissions")
               VALUES (901, 900, 1, 'Starter', '{}', '{}', '{}', 1, 'live-key', 1, '{"allowEdit":true}')"#,
            [],
        )?;
        Ok(())
    })
    .await
    .expect("seed");
    let encryption = Arc::new(encryption());
    tenant_default::set(
        &db,
        &encryption,
        Scope::Root,
        901,
        "live-key".to_string(),
        PUBLISHER,
    )
    .await
    .expect("default template");

    let jobs = Arc::new(Jobs::default());
    let api = Arc::new(
        ApiState::new(Arc::clone(&db), "http://app")
            .with_encryption(encryption)
            .with_jobs(Arc::clone(&jobs) as Arc<dyn Enqueuer>),
    );
    let auth = Arc::new(AuthState {
        db,
        settings: AuthSettings {
            secret: SECRET.to_string(),
            base_url: "http://app".to_string(),
            sign_up_disabled: true,
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
    (auth, api, jobs)
}

async fn home(auth: &Arc<AuthState>, api: &Arc<ApiState>) -> (StatusCode, Value) {
    let app = spa_router_with(Arc::clone(auth), Some(Arc::clone(api)))
        .merge(rpc_router(Arc::clone(auth)));
    let request = Request::builder()
        .method("POST")
        .uri("/rpc/pages/home")
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            header::COOKIE,
            format!(
                "better-auth.session_token={}",
                sign_cookie_value("anntoken", SECRET)
            ),
        )
        .body(Body::from(json!({ "json": null, "meta": [] }).to_string()))
        .expect("request");
    let response = app.oneshot(request).await.expect("response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value: Value = serde_json::from_slice(&bytes).expect("json body");
    (status, value["json"].clone())
}

fn install_jobs(jobs: &Jobs) -> Vec<Value> {
    jobs.calls
        .lock()
        .expect("lock")
        .iter()
        .filter(|(name, _)| name == "installTemplate")
        .map(|(_, data)| data.clone())
        .collect()
}

#[tokio::test]
async fn first_workspace_stages_and_dispatches_the_default_template() {
    let (auth, api, jobs) = setup().await;
    let (status, page) = home(&auth, &api).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    let workspace: i64 = page["createdWorkspaceId"]
        .as_str()
        .and_then(|id| id.parse().ok())
        .expect("created workspace");

    let staged: Vec<(i64, String, String, i64)> = auth
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(
                r#"SELECT "id", "status", "installSource", "templateId" FROM "TemplateInstallation" WHERE "workspaceId" = ?1"#,
            )?;
            let rows = stmt
                .query_map([workspace], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .await
        .expect("installations");
    assert_eq!(staged.len(), 1, "{staged:?}");
    let (installation, status, source, template) = staged[0].clone();
    assert_eq!(
        (status.as_str(), source.as_str(), template),
        ("pending", "tenantDefault", 901)
    );

    let dispatched = install_jobs(&jobs);
    assert_eq!(dispatched.len(), 1, "{dispatched:?}");
    assert_eq!(
        dispatched[0]["data"]["installationId"],
        json!(installation.to_string())
    );

    // A second visit creates nothing and dispatches nothing more.
    let (_, again) = home(&auth, &api).await;
    assert_eq!(again["createdWorkspaceId"], Value::Null);
    assert_eq!(install_jobs(&jobs).len(), 1);
}
