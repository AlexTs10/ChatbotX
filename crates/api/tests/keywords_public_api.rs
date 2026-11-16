//! Port of `apps/builder/__tests__/keywords-public-api.test.ts` (P3.6-D3,
//! slice keywords). Adapted: the captured oRPC procedures and the mocked
//! `automatedResponseService` become `handlers::dispatch` calls over real
//! `AutomatedResponse` rows; "the service was called with X" becomes the
//! answer / the stored row; `input.safeParse` becomes the route's 422.

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

/// Inbound rule 1 ("hello"), outbound rule 2 ("bye"), flow 10.
async fn seeded() -> Fixture {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (10, ?1, 'Reply')"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "AutomatedResponse" ("id", "createdAt", "updatedAt", "workspaceId", "keywords", "conditions", "status", "text", "type")
                   VALUES (1, 1000, 1000, ?1, '["hello"]', '[{"operator":"contains","values":["hello"]}]', 1, 'Hi', 'inbound'),
                          (2, 2000, 2000, ?1, '["bye"]', NULL, 1, 'Bye', 'outbound')"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    f
}

async fn get(f: &Fixture, id: &str, query: Option<&str>) -> (u16, Value) {
    call(f, "keywords.get", WS, &[("id", id)], query, None).await
}

async fn patch(f: &Fixture, route: &str, id: &str, body: Value) -> (u16, Value) {
    call(f, route, WS, &[("id", id)], None, Some(body)).await
}

fn issue_paths(body: &Value) -> Vec<String> {
    body["data"]["issues"]
        .as_array()
        .map(|issues| {
            issues
                .iter()
                .map(|issue| {
                    issue["path"]
                        .as_array()
                        .map(|p| {
                            p.iter()
                                .map(|s| s.as_str().map_or_else(|| s.to_string(), str::to_string))
                                .collect::<Vec<_>>()
                                .join(".")
                        })
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn registers_the_keywords_public_router_under_the_automation_scope() {
    for route_id in [
        "keywords.list",
        "keywords.get",
        "keywords.create",
        "keywords.update",
        "keywords.updateStatus",
        "keywords.delete",
    ] {
        let route = route_by_id(route_id).expect("route");
        assert_eq!(
            route.auth,
            RouteAuth::WorkspaceToken(&["automation"]),
            "{route_id}"
        );
    }
}

// describe("GET /v1/keywords")

#[tokio::test]
async fn keeps_the_type_filter_in_the_where_clause_instead_of_dropping_it() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "keywords.list",
        WS,
        &[],
        Some("page=1&perPage=50&type=outbound"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["pageCount"], json!(1));
    let ids: Vec<&Value> = body["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|r| &r["id"])
        .collect();
    assert_eq!(ids, vec![&json!("2")]);
}

#[tokio::test]
async fn defaults_type_to_inbound_when_omitted() {
    let f = seeded().await;
    let (status, body) = call(&f, "keywords.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({
            "data": [{
                "id": "1",
                "keywords": ["hello"],
                "conditions": [{ "operator": "contains", "values": ["hello"] }],
                "status": true,
                "text": "Hi",
                "flowId": null,
                "type": "inbound",
                "createdAt": "1970-01-01T00:00:01.000Z",
                "updatedAt": "1970-01-01T00:00:01.000Z",
            }],
            "pageCount": 1,
        })
    );
    let (_, other) = call(&f, "keywords.list", OTHER_WS, &[], None, None).await;
    assert_eq!(other, json!({ "data": [], "pageCount": 0 }));
}

// describe("GET /v1/keywords/{id}")

#[tokio::test]
async fn delegates_to_automated_response_service_find_or_fail() {
    let f = seeded().await;
    let (status, body) = get(&f, "1", None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("1"));
    let (status, body) = get(&f, "2", None).await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["code"], json!("notFound"));
    assert_eq!(body["message"], json!("Automated response not found"));
}

#[tokio::test]
async fn threads_type_outbound_through_so_an_outbound_id_doesn_t_404() {
    let f = seeded().await;
    let (status, body) = get(&f, "2", Some("type=outbound")).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], json!("2"));
    assert_eq!(body["conditions"], Value::Null);
    assert_eq!(body["type"], json!("outbound"));
}

// describe("POST /v1/keywords")

#[tokio::test]
async fn delegates_to_automated_response_service_create() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "keywords.create",
        WS,
        &[],
        None,
        Some(json!({ "type": "inbound", "keywords": ["hi"], "text": "Hey" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["keywords"], json!(["hi"]));
    assert_eq!(
        body["conditions"],
        json!([{ "operator": "contains", "values": ["hi"] }])
    );
    assert_eq!(body["status"], json!(true));
    let id: i64 = body["id"].as_str().expect("id").parse().expect("bigint");
    assert_eq!(
        scalar(
            &f,
            r#"SELECT "text" FROM "AutomatedResponse" WHERE "id" = ?1"#,
            id
        )
        .await,
        Some(json!("Hey"))
    );
}

// describe("PATCH /v1/keywords/{id}")

#[tokio::test]
async fn delegates_to_automated_response_service_update_without_a_redundant_pre_check() {
    let f = seeded().await;
    let (status, body) = patch(
        &f,
        "keywords.update",
        "1",
        json!({ "keywords": ["hello", "hey"] }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["keywords"], json!(["hello", "hey"]));
    let (status, body) = patch(&f, "keywords.update", "999", json!({ "text": "x" })).await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Automated response not found"));
}

#[tokio::test]
async fn threads_type_outbound_through_to_the_service_call() {
    let f = seeded().await;
    let (status, body) = patch(
        &f,
        "keywords.update",
        "2",
        json!({ "type": "outbound", "keywords": ["hi"] }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["keywords"], json!(["hi"]));
    assert_eq!(body["conditions"], Value::Null);
    let (status, _) = patch(&f, "keywords.update", "2", json!({ "keywords": ["hi"] })).await;
    assert_eq!(status, 404);
}

// describe("PATCH /v1/keywords/{id}/status")

#[tokio::test]
async fn verifies_existence_then_delegates_to_automated_response_service_set_status() {
    let f = seeded().await;
    let (status, body) = patch(&f, "keywords.updateStatus", "1", json!({ "status": false })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!(false));
    let (status, body) = patch(
        &f,
        "keywords.updateStatus",
        "999",
        json!({ "status": false }),
    )
    .await;
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["message"], json!("Automated response not found"));
}

#[tokio::test]
async fn threads_type_outbound_through_to_find_or_fail_and_set_status() {
    let f = seeded().await;
    let (status, _) = patch(&f, "keywords.updateStatus", "2", json!({ "status": false })).await;
    assert_eq!(status, 404);
    let (status, body) = patch(
        &f,
        "keywords.updateStatus",
        "2",
        json!({ "status": false, "type": "outbound" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], json!(false));
}

// describe("DELETE /v1/keywords/{id}")

#[tokio::test]
async fn delegates_to_automated_response_service_delete_many() {
    let f = seeded().await;
    let (status, body) = call(&f, "keywords.delete", WS, &[("id", "1")], None, None).await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(body, Value::Null);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "AutomatedResponse" WHERE "id" = ?1"#,
            1
        )
        .await,
        Some(json!(0))
    );
}

#[tokio::test]
async fn threads_type_outbound_through_to_delete_many() {
    let f = seeded().await;
    let (status, _) = call(&f, "keywords.delete", WS, &[("id", "2")], None, None).await;
    assert_eq!(status, 204);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "AutomatedResponse" WHERE "id" = ?1"#,
            2
        )
        .await,
        Some(json!(1))
    );
    let (status, _) = call(
        &f,
        "keywords.delete",
        WS,
        &[("id", "2")],
        None,
        Some(json!({ "type": "outbound" })),
    )
    .await;
    assert_eq!(status, 204);
    assert_eq!(
        scalar(
            &f,
            r#"SELECT count(*) FROM "AutomatedResponse" WHERE "id" = ?1"#,
            2
        )
        .await,
        Some(json!(0))
    );
}

// describe("keyword conditions on create/update")

fn conditions() -> Value {
    json!([
        { "operator": "contains", "values": ["muốn", "quan tâm"] },
        { "operator": "containsImage", "values": [] },
    ])
}

#[tokio::test]
async fn create_accepts_conditions_instead_of_keywords() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "keywords.create",
        WS,
        &[],
        None,
        Some(json!({ "type": "inbound", "conditions": conditions(), "text": "Hi" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["conditions"], conditions());
    assert_eq!(body["keywords"], json!(["muốn", "quan tâm"]));
}

#[tokio::test]
async fn create_rejects_keywords_and_conditions_together_or_neither() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "keywords.create",
        WS,
        &[],
        None,
        Some(json!({ "type": "inbound", "keywords": ["hi"], "conditions": conditions(), "text": "Hi" })),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], json!("invalidRequestData"));
    assert_eq!(issue_paths(&body), vec!["conditions"]);
    let (status, body) = call(
        &f,
        "keywords.create",
        WS,
        &[],
        None,
        Some(json!({ "type": "inbound", "text": "Hi" })),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(issue_paths(&body), vec!["keywords"]);
}

#[tokio::test]
async fn create_rejects_an_invalid_regex_condition() {
    let f = seeded().await;
    let (status, body) = call(
        &f,
        "keywords.create",
        WS,
        &[],
        None,
        Some(json!({
            "type": "inbound",
            "conditions": [{ "operator": "matchesRegex", "values": ["(unclosed"] }],
            "text": "Hi",
        })),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(issue_paths(&body), vec!["conditions.0.values.0"]);
}

#[tokio::test]
async fn update_rejects_a_body_with_nothing_to_change() {
    let f = seeded().await;
    for body in [
        json!({ "type": "inbound" }),
        json!({ "type": "inbound", "status": false }),
        json!({ "type": "inbound", "name": "x" }),
    ] {
        let (status, answer) = patch(&f, "keywords.update", "1", body).await;
        assert_eq!(status, 422, "{answer}");
        assert_eq!(issue_paths(&answer), vec![""]);
        assert!(
            answer["data"]["issues"]
                .to_string()
                .contains("keywords.updateStatus")
        );
    }
    let (status, body) = patch(
        &f,
        "keywords.update",
        "1",
        json!({ "type": "inbound", "text": "hi" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let (status, body) = patch(
        &f,
        "keywords.update",
        "1",
        json!({ "type": "inbound", "folderId": null }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn update_rejects_keywords_and_conditions_together() {
    let f = seeded().await;
    let (status, body) = patch(
        &f,
        "keywords.update",
        "1",
        json!({ "type": "inbound", "keywords": ["hi"], "conditions": conditions() }),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(issue_paths(&body), vec!["conditions"]);
    let (status, body) = patch(
        &f,
        "keywords.update",
        "1",
        json!({ "type": "inbound", "conditions": conditions() }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn update_forwards_conditions_to_the_service_untouched() {
    let f = seeded().await;
    let (status, body) = patch(
        &f,
        "keywords.update",
        "1",
        json!({ "type": "inbound", "conditions": conditions() }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["conditions"], conditions());
    assert_eq!(body["text"], json!("Hi"));
}
