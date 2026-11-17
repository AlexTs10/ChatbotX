//! Q.D4 slice 3: `templatePreviews.get`, `templateInstallations.install`, the
//! install job, `templateInstallations.get` and `listResources` on real rows.

mod handlers_common;

use chatbotx_api::handlers::{Call, Principal, dispatch_as};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, fixture};
use serde_json::{Value, json};

async fn run(
    fixture: &Fixture,
    route: &str,
    read_only: bool,
    params: &[(&'static str, &str)],
    body: Option<Value>,
) -> (u16, Value) {
    let params: Vec<(&'static str, String)> =
        params.iter().map(|(k, v)| (*k, (*v).to_string())).collect();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let principal = Principal {
        read_only,
        ..Principal::default()
    };
    let response = dispatch_as(
        &fixture.state,
        route,
        Call {
            workspace_id: WS,
            params: &params,
            query: None,
            body: body.as_bytes(),
        },
        &principal,
    )
    .await
    .expect("ported route");
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("json")
    };
    (status, json)
}

async fn count(fixture: &Fixture, sql: &'static str) -> i64 {
    fixture
        .state
        .db
        .read(move |conn| Ok(conn.query_row(sql, params![WS], |r| r.get(0))?))
        .await
        .unwrap()
}

/// Saves keyword 21 (fires flow 11, which tags `vip`) as a template, then
/// moves the template to the other workspace and shares it.
async fn shared_template(fixture: &Fixture) -> String {
    fixture
        .state
        .db
        .write(|tx| {
            tx.execute(r#"INSERT INTO "Tag" ("id", "name", "workspaceId") VALUES (31, 'vip', ?1)"#, params![WS])?;
            tx.execute(r#"INSERT INTO "Flow" ("id", "name", "workspaceId") VALUES (11, 'Welcome', ?1)"#, params![WS])?;
            let nodes = json!([{ "id": "1", "type": "message", "data": { "details": { "steps": [
                { "id": "s1", "stepType": "addContactTag", "tags": ["vip"] }
            ] } } }]);
            tx.execute(
                r#"INSERT INTO "FlowVersion" ("id", "workspaceId", "flowId", "nodes", "edges", "isDraft", "startNodeId")
                   VALUES (111, ?1, 11, ?2, '[]', 1, 1)"#,
                params![WS, nodes.to_string()],
            )?;
            tx.execute(
                r#"INSERT INTO "AutomatedResponse" ("id", "workspaceId", "type", "keywords", "status", "flowId")
                   VALUES (21, ?1, 'inbound', '["hi"]', 1, 11)"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    let (status, out) = run(
        fixture,
        "templates.create",
        false,
        &[],
        Some(json!({
            "name": "Starter",
            "selection": { "keywords": { "mode": "ids", "ids": ["21"] } },
            "defaultPermissions": { "allowEdit": true },
            "createInstallFolder": false,
            "defaultAutoUpdate": false,
        })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    let id: i64 = out["id"].as_str().unwrap().parse().unwrap();
    fixture
        .state
        .db
        .write(move |tx| {
            tx.execute(
                r#"UPDATE "Template" SET "workspaceId" = ?1, "shareEnabled" = 1 WHERE "id" = ?2"#,
                params![OTHER_WS, id],
            )?;
            Ok(tx.query_row(
                r#"SELECT "shareToken" FROM "Template" WHERE "id" = ?1"#,
                [id],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn preview_install_and_detail_copy_the_template_into_the_workspace() {
    let fixture = fixture().await;
    let token = shared_template(&fixture).await;

    let (status, preview) = run(
        &fixture,
        "templatePreviews.get",
        false,
        &[("id", &token)],
        None,
    )
    .await;
    assert_eq!(status, 200, "{preview}");
    assert_eq!(preview["installable"], true, "{preview}");
    let (_, read_only) = run(
        &fixture,
        "templatePreviews.get",
        true,
        &[("id", &token)],
        None,
    )
    .await;
    assert_eq!(read_only["installable"], false, "{read_only}");
    let (status, denied) = run(
        &fixture,
        "templateInstallations.install",
        true,
        &[],
        Some(json!({ "shareToken": token })),
    )
    .await;
    assert_eq!(status, 403, "{denied}");

    let (status, out) = run(
        &fixture,
        "templateInstallations.install",
        false,
        &[],
        Some(json!({ "shareToken": token })),
    )
    .await;
    assert_eq!(status, 202, "{out}");
    let installation = out["installationId"].as_str().unwrap().to_string();
    chatbotx_business::template::request::run_install_job(
        &fixture.state.db,
        WS,
        installation.parse().unwrap(),
        "http://localhost",
    )
    .await
    .expect("install job");

    let (status, detail) = run(
        &fixture,
        "templateInstallations.get",
        false,
        &[("id", &installation)],
        None,
    )
    .await;
    assert_eq!(status, 200, "{detail}");
    assert!(
        matches!(detail["status"].as_str(), Some("completed" | "partial")),
        "{detail}"
    );
    assert_eq!(
        count(
            &fixture,
            r#"SELECT COUNT(*) FROM "AutomatedResponse" WHERE "workspaceId" = ?1"#
        )
        .await,
        2
    );
    assert_eq!(
        count(
            &fixture,
            r#"SELECT COUNT(*) FROM "Flow" WHERE "workspaceId" = ?1"#
        )
        .await,
        2
    );
    // The tag is reused by name, not copied.
    assert_eq!(
        count(
            &fixture,
            r#"SELECT COUNT(*) FROM "Tag" WHERE "workspaceId" = ?1"#
        )
        .await,
        1
    );
    let (status, resources) = run(
        &fixture,
        "templateInstallations.listResources",
        false,
        &[("id", &installation)],
        None,
    )
    .await;
    assert_eq!(status, 200, "{resources}");
    assert!(
        resources["items"].as_array().unwrap().len() >= 2,
        "{resources}"
    );
}
