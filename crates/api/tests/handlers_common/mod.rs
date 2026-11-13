//! Shared fixtures of the ported public-handler tests (P3.6-D1): a migrated
//! SQLite file, a workspace, and a direct call of `handlers::dispatch`.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chatbotx_api::handlers::{ApiState, Call, dispatch};
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use serde_json::Value;

pub const WS: i64 = 2001;
pub const OTHER_WS: i64 = 2002;

/// The handler state over a temp database (directory removed on drop).
pub struct Fixture {
    pub state: ApiState,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A migrated database with workspaces `WS` and `OTHER_WS`.
pub async fn fixture() -> Fixture {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "chatbotx-api-handler-test-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let db = Db::open(dir.join("test.db")).expect("Db::open");
    db.migrate().await.expect("migrate");
    db.write(|tx| {
        for (user, ws) in [(1001_i64, WS), (1002, OTHER_WS)] {
            tx.execute(
                r#"INSERT INTO "User" ("id", "email") VALUES (?1, ?2)"#,
                params![user, format!("user-{user}@example.test")],
            )?;
            tx.execute(
                r#"INSERT INTO "Workspace" ("id", "name", "ownerId") VALUES (?1, ?2, ?3)"#,
                params![ws, format!("Workspace {ws}"), user],
            )?;
        }
        Ok(())
    })
    .await
    .expect("seed workspaces");
    Fixture {
        state: ApiState {
            db: Arc::new(db),
            app_url: "http://app".to_string(),
        },
        dir,
    }
}

/// The status and JSON body (`Value::Null` for an empty body) of one handler call.
pub async fn call(
    fixture: &Fixture,
    route_id: &str,
    workspace_id: i64,
    params: &[(&'static str, &str)],
    query: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let params: Vec<(&'static str, String)> =
        params.iter().map(|(k, v)| (*k, (*v).to_string())).collect();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let response = dispatch(
        &fixture.state,
        route_id,
        Call {
            workspace_id,
            params: &params,
            query,
            body: body.as_bytes(),
        },
    )
    .await
    .unwrap_or_else(|| panic!("{route_id} has no handler"));
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("JSON body")
    };
    (status, json)
}
