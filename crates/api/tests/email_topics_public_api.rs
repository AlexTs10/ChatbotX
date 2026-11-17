//! Port of `apps/builder/__tests__/email-topics-public-api.test.ts` and the
//! handler cases of `email-topics-public-scope.test.ts` (Q.D5; the scope
//! cases are in `routes_public_scope.rs`). Adapted: the mocked
//! `emailTopicService` becomes `handlers::dispatch` over real `"EmailTopic"`
//! rows; "the service was called with X" becomes the answer / the stored row.

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

/// Topic 1 "News" in folder 50, topic 2 "Promo" (root), topic 3 in `OTHER_WS`.
async fn seeded() -> Fixture {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Folder" ("id", "name", "workspaceId", "folderType", "paths") VALUES (50, 'f', ?1, 'emailTopic', '[]')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "EmailTopic" ("id", "createdAt", "updatedAt", "name", "workspaceId", "folderId")
                   VALUES (1, 1000, 1000, 'News', ?1, 50), (2, 2000, 2000, 'Promo', ?1, NULL), (3, 3000, 3000, 'Other', ?2, NULL)"#,
                params![WS, OTHER_WS],
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
fn registers_the_email_topics_public_router_under_the_broadcasts_scope() {
    for route in [
        "emailTopics.list",
        "emailTopics.get",
        "emailTopics.create",
        "emailTopics.update",
        "emailTopics.delete",
    ] {
        assert_eq!(
            route_by_id(route).expect("route").auth,
            RouteAuth::WorkspaceToken(&["broadcasts"])
        );
    }
}

#[test]
fn registers_the_expected_public_crud_routes() {
    let routes: Vec<(&str, &str)> = [
        "emailTopics.list",
        "emailTopics.get",
        "emailTopics.create",
        "emailTopics.update",
        "emailTopics.delete",
    ]
    .iter()
    .map(|id| {
        let route = route_by_id(id).expect("route");
        (route.method, route.path)
    })
    .collect();
    assert_eq!(
        routes,
        vec![
            ("GET", "/v1/email-topics"),
            ("GET", "/v1/email-topics/{id}"),
            ("POST", "/v1/email-topics"),
            ("PUT", "/v1/email-topics/{id}"),
            ("DELETE", "/v1/email-topics/{id}"),
        ]
    );
}

// describe("GET /v1/email-topics")

#[tokio::test]
async fn lists_email_topics_in_the_token_workspace() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "emailTopics.list",
        WS,
        &[],
        Some("page=1&perPage=10&name=new&folderId=50"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "data": [{
                "id": "1",
                "createdAt": "1970-01-01T00:00:01.000Z",
                "updatedAt": "1970-01-01T00:00:01.000Z",
                "name": "News",
                "folderId": "50",
                "sendsTotal": 0,
                "deliveredsTotal": 0,
                "seensTotal": 0,
                "clicksTotal": 0,
            }],
            "pageCount": 1,
        })
    );
    // No sort = newest first; "0" = the root.
    let (_, all) = call(&f, "emailTopics.list", WS, &[], None, None).await;
    assert_eq!(ids(&all), vec!["2", "1"]);
    let (_, root) = call(&f, "emailTopics.list", WS, &[], Some("folderId=0"), None).await;
    assert_eq!(ids(&root), vec!["2"]);
    let (_, sorted) = call(
        &f,
        "emailTopics.list",
        WS,
        &[],
        Some("sort[0][id]=name&sort[0][desc]=false"),
        None,
    )
    .await;
    assert_eq!(ids(&sorted), vec!["1", "2"]);
}

// describe("GET /v1/email-topics/{id}")

#[tokio::test]
async fn gets_an_email_topic_in_the_token_workspace() {
    let f = seeded().await;
    let (status, body) = call(&f, "emailTopics.get", WS, &[("id", "2")], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["name"], "Promo");
    assert!(body.get("workspaceId").is_none());
    let (status, body) = call(&f, "emailTopics.get", WS, &[("id", "3")], None, None).await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], "Email topic not found");
}

// describe("POST /v1/email-topics")

#[tokio::test]
async fn creates_an_email_topic_in_the_token_workspace() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "emailTopics.create",
        WS,
        &[],
        None,
        Some(json!({ "name": " Weekly ", "folderId": "50" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let id: i64 = body["id"].as_str().expect("id").parse().expect("digits");
    assert_eq!(
        scalar(&f, r#"SELECT "name" FROM "EmailTopic" WHERE "id" = ?1"#, id).await,
        Some(json!("Weekly"))
    );
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "workspaceId" FROM "EmailTopic" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(json!(WS))
    );
    // Taken name; unknown folder; the root folder id.
    let (status, body) = call(
        &f,
        "emailTopics.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "News" })),
    )
    .await;
    assert_eq!((status, body["code"].clone()), (400, json!("nameTaken")));
    assert_eq!(body["defined"], true);
    let (status, _) = call(
        &f,
        "emailTopics.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "X", "folderId": "999" })),
    )
    .await;
    assert_eq!(status, 404);
    let (status, body) = call(
        &f,
        "emailTopics.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "Y", "folderId": "0" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
}

// describe("PUT /v1/email-topics/{id}")

#[tokio::test]
async fn updates_an_email_topic_in_the_token_workspace() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "emailTopics.update",
        WS,
        &[("id", "1")],
        None,
        Some(json!({ "name": "Updates", "workspaceId": OTHER_WS.to_string() })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["name"], "Updates");
    // `passes workspaceId from the token, not from input`; `strips workspaceId`.
    assert!(body.get("workspaceId").is_none());
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "workspaceId" FROM "EmailTopic" WHERE "id" = ?1"#,
            1
        )
        .await,
        Some(json!(WS))
    );
}

#[tokio::test]
async fn propagates_a_not_found_error_from_the_service() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "emailTopics.update",
        WS,
        &[("id", "3")],
        None,
        Some(json!({ "name": "Updates" })),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], "Email topic not found");
}

// describe("DELETE /v1/email-topics/{id}")

#[tokio::test]
async fn deletes_an_email_topic_in_the_token_workspace() {
    let f = seeded().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "AnalyticsEmailTopic" ("id", "topicId", "workspaceId", "email", "token")
                   VALUES (70, 1, ?1, 'a@b.c', 'tok')"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed tracking row");
    let (status, body) = call(&f, "emailTopics.delete", WS, &[("id", "1")], None, None).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(body, Value::Null);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "EmailTopic" WHERE "id" = ?1"#,
            1
        )
        .await,
        Some(json!(0))
    );
    // The tracking row keeps the topic as its reference.
    assert_eq!(
        scalar(
            &f,
            r#"SELECT json_array("topicId", "topicReferenceId") FROM "AnalyticsEmailTopic" WHERE "id" = ?1"#,
            70
        )
        .await,
        Some(json!("[null,1]"))
    );
}

#[tokio::test]
async fn rejects_with_not_found_when_nothing_was_deleted() {
    let f = seeded().await;
    let (status, body) = call(&f, "emailTopics.delete", WS, &[("id", "3")], None, None).await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], "Email topic not found");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "EmailTopic" WHERE "id" = ?1"#,
            3
        )
        .await,
        Some(json!(1))
    );
}
