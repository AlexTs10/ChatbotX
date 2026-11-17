//! Port of `apps/builder/__tests__/sequences-public-api.test.ts` and
//! `sequence-folders-public-api.test.ts` (Q.D5). Adapted: the mocked
//! `sequenceService` / folder service become `handlers::dispatch` over real
//! rows; "called with X" becomes the answer and the stored row.

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

/// Sequences 31 "Welcome" (active, folder 5), 32 "Alpha" (inactive), 39
/// (`OTHER_WS`); step 41 of 31; flow 61; sequence folder 5, tag folder 9,
/// sequence folder 7 (trash) under 5.
async fn seeded() -> Fixture {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Folder" ("id", "name", "workspaceId", "folderType", "paths") VALUES
                   (5, 'Seq', ?1, 'sequence', '[]'), (9, 'Tags', ?1, 'tag', '[]')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Folder" ("id", "name", "workspaceId", "folderType", "paths", "parentId", "isTrash")
                   VALUES (7, 'Old', ?1, 'sequence', '[5]', 5, 1)"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Sequence" ("id", "createdAt", "updatedAt", "name", "active", "folderId", "workspaceId") VALUES
                   (31, 1000, 1000, 'Welcome', 1, 5, ?1), (32, 2000, 2000, 'Alpha', 0, NULL, ?1), (39, 3000, 3000, 'Far', 1, NULL, ?2)"#,
                params![WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "SequenceStep" ("id", "sequenceId", "order", "delayDays", "delayMinutes", "delayUnit") VALUES (41, 31, 0, 1, 0, 'days')"#,
                [],
            )?;
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (61, ?1, 'f')"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    f
}

fn ids(body: &Value) -> Vec<String> {
    body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|r| r["id"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn registers_the_sequences_public_router_under_the_broadcasts_scope() {
    for route in [
        "sequences.list",
        "sequences.get",
        "sequences.create",
        "sequences.update",
        "sequences.delete",
        "sequences.upsertStep",
        "sequences.deleteStep",
        "sequences.listFolders",
        "sequences.createFolder",
        "sequences.updateFolder",
        "sequences.deleteFolder",
    ] {
        assert_eq!(
            route_by_id(route).expect("route").auth,
            RouteAuth::WorkspaceToken(&["broadcasts"])
        );
    }
}

// describe("GET /v1/sequences")

#[tokio::test]
async fn forwards_filters_and_keeps_newest_first_as_the_default_sort() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "sequences.list",
        WS,
        &[],
        Some("page=1&perPage=10&name=wel&folderId=5&active=true"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(ids(&body), vec!["31"]);
    assert_eq!(body["data"][0]["stepsCount"], 1);
    assert_eq!(body["pageCount"], 1);
    let (_, all) = call(
        &f,
        "sequences.list",
        WS,
        &[],
        Some("page=1&perPage=10"),
        None,
    )
    .await;
    assert_eq!(ids(&all), vec!["32", "31"]);
    let (_, sorted) = call(
        &f,
        "sequences.list",
        WS,
        &[],
        Some("page=1&perPage=10&sort[0][id]=name&sort[0][desc]=false"),
        None,
    )
    .await;
    assert_eq!(ids(&sorted), vec!["32", "31"]);
    let (_, sorted) = call(
        &f,
        "sequences.list",
        WS,
        &[],
        Some("sort[0][id]=name&sort[0][desc]=true"),
        None,
    )
    .await;
    assert_eq!(ids(&sorted), vec!["31", "32"]);
}

// describe("POST /v1/sequences")

#[tokio::test]
async fn sources_workspace_id_from_context() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "sequences.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "Welcome series" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let id: i64 = body["sequenceId"]
        .as_str()
        .expect("sequenceId")
        .parse()
        .expect("digits");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "workspaceId" FROM "Sequence" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(json!(WS))
    );
}

// describe("PATCH /v1/sequences/{id}")

#[tokio::test]
async fn update_delegates_scoped_to_the_context_workspace() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "sequences.update",
        WS,
        &[("id", "31")],
        None,
        Some(json!({ "active": false })),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(
        scalar(&f, r#"SELECT "active" FROM "Sequence" WHERE "id" = ?1"#, 31).await,
        Some(json!(0))
    );
    let (status, _) = call(
        &f,
        "sequences.update",
        WS,
        &[("id", "39")],
        None,
        Some(json!({ "active": false })),
    )
    .await;
    assert_eq!(status, 404);
}

// describe("DELETE /v1/sequences/{id}")

#[tokio::test]
async fn delete_delegates_scoped_to_the_context_workspace() {
    let f = seeded().await;
    let (status, _) = call(&f, "sequences.delete", WS, &[("id", "39")], None, None).await;
    assert_eq!(status, 404);
    let (status, _) = call(&f, "sequences.delete", WS, &[("id", "31")], None, None).await;
    assert_eq!(status, 204);
    assert_eq!(
        scalar(&f, r#"SELECT count(*) FROM "Sequence" WHERE "id" = ?1"#, 31).await,
        Some(json!(0))
    );
}

// describe("PUT /v1/sequences/{id}/steps")

#[tokio::test]
async fn asserts_ownership_before_upserting_using_the_id_path_segment_as_sequence_id() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "sequences.upsertStep",
        WS,
        &[("id", "31")],
        None,
        Some(json!({ "order": 1, "flowId": "61" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let id: i64 = body["stepId"]
        .as_str()
        .expect("stepId")
        .parse()
        .expect("digits");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "sequenceId" FROM "SequenceStep" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(json!(31))
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "flowId" FROM "SequenceStep" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(json!(61))
    );
}

#[tokio::test]
async fn throws_before_calling_upsert_step_when_the_sequence_is_not_owned_by_this_workspace() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "sequences.upsertStep",
        WS,
        &[("id", "39")],
        None,
        Some(json!({ "order": 0 })),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], "Sequence not found");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "SequenceStep" WHERE "sequenceId" = ?1"#,
            39
        )
        .await,
        Some(json!(0))
    );
}

// describe("DELETE /v1/sequences/{id}/steps/{stepId}")

#[tokio::test]
async fn asserts_ownership_before_deleting_the_step() {
    let f = seeded().await;
    // The step belongs to 31, not 32: the `{id}` segment is not decorative.
    let (status, _) = call(
        &f,
        "sequences.deleteStep",
        WS,
        &[("id", "32"), ("stepId", "41")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = call(
        &f,
        "sequences.deleteStep",
        WS,
        &[("id", "39"), ("stepId", "41")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = call(
        &f,
        "sequences.deleteStep",
        WS,
        &[("id", "31"), ("stepId", "41")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "SequenceStep" WHERE "id" = ?1"#,
            41
        )
        .await,
        Some(json!(0))
    );
}

// describe("GET /v1/sequences/{id} input")

#[tokio::test]
async fn get_accepts_a_numeric_id_and_rejects_a_non_numeric_one() {
    let f = seeded().await;
    let (status, body) = call(&f, "sequences.get", WS, &[("id", "31")], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["steps"][0]["id"], "41");
    let (status, _) = call(
        &f,
        "sequences.get",
        WS,
        &[("id", "11714399357876729")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    // A non-numeric id fails the input schema (422 `invalidRequestData`).
    let (status, _) = call(&f, "sequences.get", WS, &[("id", "abc")], None, None).await;
    assert_eq!(status, 422);
}

// `sequence-folders-public-api.test.ts`

#[tokio::test]
async fn list_and_create_are_pinned_to_the_sequence_folder_type_and_workspace() {
    let f = seeded().await;
    let (status, body) = call(&f, "sequences.listFolders", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(ids(&body), vec!["5"]);
    let (status, body) = call(
        &f,
        "sequences.createFolder",
        WS,
        &[],
        None,
        Some(json!({ "name": "Promo", "parentId": "0" })),
    )
    .await;
    // create answers 201 like sequences.create
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["folderType"], "sequence");
    assert_eq!(body["parentId"], Value::Null);
    assert!(body.get("workspaceId").is_none());
}

#[tokio::test]
async fn rename_and_delete_refuse_a_folder_of_another_type_as_404() {
    let f = seeded().await;
    let (status, _) = call(
        &f,
        "sequences.updateFolder",
        WS,
        &[("id", "9")],
        None,
        Some(json!({ "name": "x" })),
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = call(&f, "sequences.deleteFolder", WS, &[("id", "9")], None, None).await;
    assert_eq!(status, 404);
    assert_eq!(
        scalar(&f, r#"SELECT "name" FROM "Folder" WHERE "id" = ?1"#, 9).await,
        Some(json!("Tags"))
    );
}

#[tokio::test]
async fn rename_and_delete_work_on_a_sequence_folder() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "sequences.updateFolder",
        WS,
        &[("id", "5")],
        None,
        Some(json!({ "name": "New" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["name"], "New");
    let (status, _) = call(&f, "sequences.deleteFolder", WS, &[("id", "5")], None, None).await;
    assert_eq!(status, 204);
}

#[tokio::test]
async fn list_forwards_is_trash_and_keeps_the_workspace_type_and_parent_scope() {
    let f = seeded().await;
    let (_, live) = call(
        &f,
        "sequences.listFolders",
        WS,
        &[],
        Some("parentId=5&isTrash=false"),
        None,
    )
    .await;
    assert_eq!(ids(&live), Vec::<String>::new());
    let (_, trash) = call(
        &f,
        "sequences.listFolders",
        WS,
        &[],
        Some("parentId=5&isTrash=true"),
        None,
    )
    .await;
    assert_eq!(ids(&trash), vec!["7"]);
}
