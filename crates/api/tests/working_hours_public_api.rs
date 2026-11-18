//! Port of `apps/builder/__tests__/working-hours-public-api.test.ts` (adapted:
//! a real migrated database instead of the mocked service).

mod handlers_common;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use chatbotx_db::rusqlite::params;
use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

async fn member(f: &handlers_common::Fixture, user: i64) {
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("id", "workspaceId", "userId", "role") VALUES (?1, ?2, ?3, 'agent')"#,
                params![user + 50_000, WS, user],
            )?;
            Ok(())
        })
        .await
        .expect("member");
}

fn id_of(v: &Value) -> String {
    v["id"].as_str().expect("id").to_string()
}

#[tokio::test]
async fn working_hours_routes_use_the_inbox_scope() {
    let route = route_by_id("workingHours.list").expect("route");
    assert_eq!(route.auth, RouteAuth::WorkspaceToken(&["inbox"]));
}

#[tokio::test]
async fn create_get_update_and_delete_a_schedule() {
    let f = fixture().await;
    member(&f, 1001).await;
    let (status, created) = call(
        &f,
        "workingHours.create",
        WS,
        &[],
        None,
        Some(json!({
            "name": " Support ",
            "intervals": [
                { "weekday": 2, "startMinute": 780, "endMinute": 1020 },
                { "weekday": 1, "startMinute": 540, "endMinute": 720 },
            ],
            "userIds": ["1001"],
        })),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    assert_eq!(created["name"], "Support");
    assert_eq!(created["intervals"][0]["weekday"], 1, "sorted by weekday");
    assert_eq!(created["memberUserIds"], json!(["1001"]));
    assert!(created.get("dateOverrides").is_none());
    let id = id_of(&created);

    let (status, updated) = call(
        &f,
        "workingHours.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "dateOverrides": [{ "date": "2026-12-24", "ranges": [] }] })),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["dateOverrides"][0]["date"], "2026-12-24");

    let (status, _) = call(&f, "workingHours.get", OTHER_WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);

    let (status, _) = call(&f, "workingHours.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    let (status, list) = call(&f, "workingHours.list", WS, &[], None, None).await;
    assert_eq!(status, 200);
    assert_eq!(list, json!({ "data": [], "pageCount": 1 }));
}

#[tokio::test]
async fn overlapping_intervals_and_empty_updates_are_rejected() {
    let f = fixture().await;
    let (status, body) = call(
        &f,
        "workingHours.create",
        WS,
        &[],
        None,
        Some(json!({
            "name": "Bad",
            "intervals": [
                { "weekday": 1, "startMinute": 540, "endMinute": 720 },
                { "weekday": 1, "startMinute": 700, "endMinute": 800 },
            ],
        })),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(
        body.to_string().contains("Intervals overlap on weekday 1"),
        "{body}"
    );

    let (_, created) = call(
        &f,
        "workingHours.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "Ok" })),
    )
    .await;
    let id = id_of(&created);
    let (status, _) = call(
        &f,
        "workingHours.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn members_must_belong_to_the_workspace_and_move_between_schedules() {
    let f = fixture().await;
    member(&f, 1001).await;
    let (_, a) = call(
        &f,
        "workingHours.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "A", "userIds": ["1001"] })),
    )
    .await;
    let (_, b) = call(
        &f,
        "workingHours.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "B" })),
    )
    .await;
    let (status, moved) = call(
        &f,
        "workingHours.setMembers",
        WS,
        &[("id", &id_of(&b))],
        None,
        Some(json!({ "userIds": ["1001"] })),
    )
    .await;
    assert_eq!(status, 200, "{moved}");
    assert_eq!(moved["memberUserIds"], json!(["1001"]));
    let (_, a_now) = call(
        &f,
        "workingHours.get",
        WS,
        &[("id", &id_of(&a))],
        None,
        None,
    )
    .await;
    assert_eq!(a_now["memberUserIds"], json!([]));

    let (status, body) = call(
        &f,
        "workingHours.setMembers",
        WS,
        &[("id", &id_of(&b))],
        None,
        Some(json!({ "userIds": ["1002"] })),
    )
    .await;
    assert_eq!(status, 422, "{body}");
}

#[tokio::test]
async fn holidays_create_update_duplicate_and_delete() {
    let f = fixture().await;
    let (status, holiday) = call(
        &f,
        "workingHours.createHoliday",
        WS,
        &[],
        None,
        Some(json!({ "date": "2026-01-01", "name": "New Year", "ranges": [] })),
    )
    .await;
    assert_eq!(status, 201, "{holiday}");
    let id = id_of(&holiday);
    let (status, dup) = call(
        &f,
        "workingHours.createHoliday",
        WS,
        &[],
        None,
        Some(json!({ "date": "2026-01-01", "ranges": [] })),
    )
    .await;
    assert_eq!(status, 422, "{dup}");

    let (status, updated) = call(
        &f,
        "workingHours.updateHoliday",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": null, "ranges": [{ "startMinute": 540, "endMinute": 720 }] })),
    )
    .await;
    assert_eq!(status, 200, "{updated}");
    assert_eq!(updated["name"], Value::Null);
    assert_eq!(updated["ranges"][0]["startMinute"], 540);

    let (status, bad_date) = call(
        &f,
        "workingHours.createHoliday",
        WS,
        &[],
        None,
        Some(json!({ "date": "2026-02-30", "ranges": [] })),
    )
    .await;
    assert_eq!(status, 422, "{bad_date}");

    let (status, _) = call(
        &f,
        "workingHours.deleteHoliday",
        WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (_, list) = call(&f, "workingHours.listHolidays", WS, &[], None, None).await;
    assert_eq!(list["data"], json!([]));
}
