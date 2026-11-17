//! Q.D4 slice 2: `templates.create` / `update`, `listSelectableResources`
//! and `previewDependencies` on real rows (snapshot, hard dependencies,
//! referenced tags, versions, ownership and token rules).

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
    query: Option<&str>,
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
            query,
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

/// Flow 11 (draft) adds tag 31 by id; keyword 21 fires flow 11; flow 12 is
/// another workspace's.
async fn seed(fixture: &Fixture) {
    fixture
        .state
        .db
        .write(|tx| {
            tx.execute(r#"INSERT INTO "Tag" ("id", "name", "workspaceId") VALUES (31, 'vip', ?1)"#, params![WS])?;
            tx.execute(r#"INSERT INTO "Flow" ("id", "name", "workspaceId") VALUES (11, 'Welcome', ?1), (12, 'Foreign', ?2)"#, params![WS, OTHER_WS])?;
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
}

fn body(selection: Value) -> Value {
    json!({
        "name": "Starter",
        "selection": selection,
        "defaultPermissions": { "allowEdit": true },
        "createInstallFolder": false,
        "defaultAutoUpdate": true,
    })
}

async fn stored(fixture: &Fixture, id: &str) -> (Value, i64) {
    let id: i64 = id.parse().unwrap();
    fixture
        .state
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "payload", "version" FROM "Template" WHERE "id" = ?1"#,
                [id],
                |r| {
                    Ok((
                        serde_json::from_str(&r.get::<_, String>(0)?).unwrap(),
                        r.get(1)?,
                    ))
                },
            )?)
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn create_snapshots_the_selection_with_its_dependencies_and_tags() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (status, out) = run(
        &fixture,
        "templates.create",
        false,
        &[],
        None,
        Some(body(
            json!({ "keywords": { "mode": "ids", "ids": ["21"] } }),
        )),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["categoryCounts"]["keywords"], 1);
    assert_eq!(out["categoryCounts"]["flows"], 1);
    assert_eq!(out["categoryCounts"]["tags"], 1);
    assert_eq!(
        out["selection"],
        json!({ "keywords": { "mode": "ids", "ids": ["21"] } })
    );
    let (payload, version) = stored(&fixture, out["id"].as_str().unwrap()).await;
    assert_eq!(version, 1);
    assert_eq!(payload["formatVersion"], 1);
    assert_eq!(payload["resources"]["flows"][0]["sourceId"], "11");
    assert_eq!(payload["resources"]["flows"][0]["published"], false);
    assert_eq!(payload["resources"]["keywords"][0]["flowId"], "11");
    assert_eq!(payload["manifests"]["tags"]["31"], json!({ "name": "vip" }));
    assert_eq!(payload["source"]["workspaceId"], WS.to_string());
}

#[tokio::test]
async fn update_advances_the_version_only_when_the_content_changes() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let selection = json!({ "flows": { "mode": "all" } });
    let (_, created) = run(
        &fixture,
        "templates.create",
        false,
        &[],
        None,
        Some(body(selection.clone())),
    )
    .await;
    let id = created["id"].as_str().unwrap().to_string();
    let (status, _) = run(
        &fixture,
        "templates.update",
        false,
        &[("id", &id)],
        None,
        Some(body(selection.clone())),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(stored(&fixture, &id).await.1, 1);
    let mut renamed = body(selection);
    renamed["selection"]["keywords"] = json!({ "mode": "all" });
    let (status, _) = run(
        &fixture,
        "templates.update",
        false,
        &[("id", &id)],
        None,
        Some(renamed),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(stored(&fixture, &id).await.1, 2);
    let (_, versions) = run(
        &fixture,
        "templates.listVersions",
        false,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(versions["items"][0]["changes"][0]["category"], "keywords");
}

#[tokio::test]
async fn create_refuses_another_workspaces_ids_and_read_only_tokens() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (status, out) = run(
        &fixture,
        "templates.create",
        false,
        &[],
        None,
        Some(body(json!({ "flows": { "mode": "ids", "ids": ["12"] } }))),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(out["code"], "templateSaveInvalidSelection", "{out}");
    let (status, _) = run(
        &fixture,
        "templates.create",
        true,
        &[],
        None,
        Some(body(json!({}))),
    )
    .await;
    assert_eq!(status, 403);
    let (status, out) = run(
        &fixture,
        "templates.create",
        false,
        &[],
        None,
        Some(body(json!({ "nope": { "mode": "all" } }))),
    )
    .await;
    assert_eq!(status, 400, "{out}");
}

#[tokio::test]
async fn preview_lists_pulled_in_dependencies_and_manifests() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (status, out) = run(
        &fixture,
        "templates.previewDependencies",
        true,
        &[],
        None,
        Some(json!({ "selection": { "keywords": { "mode": "ids", "ids": ["21"] } } })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(
        out["includedDependencies"],
        json!([{ "category": "flows", "sourceId": "11", "name": "Welcome" }])
    );
    assert_eq!(
        out["manifests"],
        json!([{ "kind": "tags", "count": 1, "names": ["vip"] }])
    );
    assert_eq!(out["droppedReferenceCount"], 0);
}

#[tokio::test]
async fn selectable_resources_page_the_workspace_rows() {
    let fixture = fixture().await;
    seed(&fixture).await;
    let (status, out) = run(
        &fixture,
        "templates.listSelectableResources",
        false,
        &[],
        Some("category=flows&limit=1"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["items"], json!([{ "id": "11", "name": "Welcome" }]));
    assert_eq!(out["total"], 1);
    assert_eq!(out["allIds"], json!(["11"]));
    let (status, _) = run(
        &fixture,
        "templates.listSelectableResources",
        true,
        &[],
        Some("category=flows"),
        None,
    )
    .await;
    assert_eq!(status, 403);
}
