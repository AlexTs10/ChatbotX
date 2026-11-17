//! Port of the `POST /v1/flows/import` cases of
//! `apps/builder/__tests__/flows-public-api.test.ts` (P3.6-D2 slice flow
//! import). Adapted: the captured handler runs through `handlers::dispatch`
//! over a real database; the mocked `startFlowImport` becomes a seeded file,
//! and "queues the import job" = the import row is run in the request (no
//! queue). Plus one own case for the input errors.

mod handlers_common;

use std::sync::Arc;

use chatbotx_db::rusqlite::params;
use chatbotx_files::{PutOptions, Uploader};
use handlers_common::{Fixture, WS, call, fixture};
use serde_json::{Value, json};

/// A fixture with a file store and an uploaded flow export (file 31).
async fn uploaded(body: &Value) -> Fixture {
    let mut f = fixture().await;
    let files = Arc::new(Uploader::in_memory());
    files
        .put_object(
            "p/31.json",
            serde_json::to_vec(body).expect("json"),
            PutOptions::default(),
            None,
        )
        .await
        .expect("put");
    f.state = f.state.clone().with_files(files);
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType")
                   VALUES (31, ?1, 'import', 'flow', 'p/31.json', 'flow.json', 'application/json')"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    f
}

fn export() -> Value {
    json!({
        "formatVersion": 2,
        "exportedAt": "2026-11-16T10:00:00.000Z",
        "source": { "workspaceId": "source-workspace", "flowId": "source-flow" },
        "flows": [{
            "name": "Imported",
            "active": true,
            "enableInInbox": true,
            "startNodeId": "1",
            "nodes": [{
                "id": "1",
                "position": { "x": 0, "y": 0 },
                "measured": { "width": 288, "height": 100 },
                "type": "sendMessage",
                "data": {
                    "name": "Send Message",
                    "isStartNode": true,
                    "details": {
                        "beforeStep": { "id": "2", "stepType": "chooseChannel", "channel": "omnichannel" },
                        "steps": [],
                        "quickReplies": [],
                    },
                },
            }],
            "edges": [],
        }],
        "customFields": {},
    })
}

#[tokio::test]
async fn post_v1_flows_import_delegates_to_start_flow_import_with_a_null_user_id_and_queues_the_import_job()
 {
    let f = uploaded(&export()).await;
    let (status, body) = call(
        &f,
        "flows.import",
        WS,
        &[],
        None,
        Some(json!({ "fileId": "31", "folderId": null })),
    )
    .await;
    assert_eq!(status, 202, "{body}");
    let import_id: i64 = body["importId"]
        .as_str()
        .expect("importId")
        .parse()
        .expect("id");
    let row = f
        .state
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "userId", "status", "workspaceId", "errorMessage" FROM "Import" WHERE "id" = ?1"#,
                [import_id],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, Option<String>>(3)?,
                    ))
                },
            )?)
        })
        .await
        .expect("import row");
    // `userId: null`: a token-initiated import is never attributed to the owner.
    assert_eq!(
        (row.0, row.1.as_str(), row.2),
        (None, "completed", WS),
        "{:?}",
        row.3
    );
    let flows = f
        .state
        .db
        .read(|conn| {
            Ok(conn.query_row(
                r#"SELECT COUNT(*) FROM "Flow" WHERE "workspaceId" = ?1 AND "name" = 'Imported'"#,
                [WS],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .await
        .expect("flows");
    assert_eq!(flows, 1);
}

#[tokio::test]
async fn post_v1_flows_import_throws_a_validation_exception_when_the_file_is_not_found() {
    let f = uploaded(&export()).await;
    let (status, body) = call(
        &f,
        "flows.import",
        WS,
        &[],
        None,
        Some(json!({ "fileId": "999", "folderId": null })),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["message"], json!("File not found"));
    let imports = f
        .state
        .db
        .read(|conn| {
            Ok(conn.query_row(r#"SELECT COUNT(*) FROM "Import""#, [], |r| {
                r.get::<_, i64>(0)
            })?)
        })
        .await
        .expect("count");
    assert_eq!(imports, 0);
}

#[tokio::test]
async fn post_v1_flows_import_input_errors() {
    let f = uploaded(&export()).await;
    for (body, path) in [
        (json!({ "folderId": null }), "fileId"),
        (json!({ "fileId": "31" }), "folderId"),
        (json!({ "fileId": "x", "folderId": null }), "fileId"),
    ] {
        let (status, out) = call(&f, "flows.import", WS, &[], None, Some(body.clone())).await;
        assert_eq!(status, 422, "{body}: {out}");
        let paths: Vec<String> = out["data"]["issues"]
            .as_array()
            .expect("issues")
            .iter()
            .map(|i| i["path"][0].as_str().unwrap_or_default().to_string())
            .collect();
        assert_eq!(paths, vec![path.to_string()], "{body}");
    }
}
