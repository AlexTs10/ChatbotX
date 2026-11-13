//! Port of `packages/business/src/ecommerce-open-hours/__tests__/ecommerce-open-hours-service.test.ts`.
//! Adapted: the mocked repository becomes real rows of a migrated SQLite file;
//! the cache invalidation assertion is dropped (no read cache in the Rust port).

mod common;

use chatbotx_business::ecommerce_open_hours::OpenHoursRange;
use chatbotx_business::ecommerce_open_hours::service;
use common::{OTHER_WS, WS, scalar_i64, two_workspaces};
use serde_json::{Map, Value, json};

const COUNT_WS: &str = r#"SELECT count(*) FROM "EcommerceOpenHours" WHERE "workspaceId" = ?1"#;

fn body(value: Value) -> Map<String, Value> {
    value.as_object().expect("object").clone()
}

#[tokio::test]
async fn ecommerce_open_hours_service_creates_a_schedule_and_drops_the_cached_settings() {
    let db = two_workspaces().await;
    let row = service::create(
        &db,
        WS,
        &body(
            json!({ "name": "Weekdays", "ranges": [{ "day": 1, "from": "09:00", "to": "17:00" }] }),
        ),
    )
    .await
    .expect("create");
    assert_eq!((row.workspace_id, row.name.as_str()), (WS, "Weekdays"));
    assert_eq!(
        row.ranges,
        vec![OpenHoursRange {
            day: 1,
            from: "09:00".to_string(),
            to: "17:00".to_string()
        }]
    );
    assert_eq!(service::list(&db, WS).await.expect("list"), vec![row]);
}

#[tokio::test]
async fn ecommerce_open_hours_service_refuses_a_schedule_without_hours_or_with_a_bad_time() {
    let db = two_workspaces().await;
    let empty = service::create(&db, WS, &body(json!({ "name": "Empty", "ranges": [] }))).await;
    assert!(empty.is_err());
    let bad = service::create(
        &db,
        WS,
        &body(json!({ "name": "Bad", "ranges": [{ "day": 1, "from": "9am", "to": "17:00" }] })),
    )
    .await;
    assert!(bad.is_err());
    assert_eq!(scalar_i64(&db, COUNT_WS, WS).await, Some(0));
}

#[tokio::test]
async fn ecommerce_open_hours_service_is_a_404_for_a_schedule_of_another_workspace() {
    let db = two_workspaces().await;
    let foreign = service::create(
        &db,
        OTHER_WS,
        &body(
            json!({ "name": "Foreign", "ranges": [{ "day": 1, "from": "09:00", "to": "17:00" }] }),
        ),
    )
    .await
    .expect("create");
    let error = service::update(&db, WS, foreign.id, &body(json!({ "name": "X" })))
        .await
        .expect_err("foreign");
    assert_eq!(error.code(), Some("notFound"));
}
