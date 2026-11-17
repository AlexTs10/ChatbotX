//! `apps/builder/__tests__/working-hours-public-api.test.ts` and
//! `working-hours-actions.test.ts` (Q.D13): the TS tests check that each
//! route delegates to `workingHoursService`; here the routes run on a
//! migrated database and the responses are checked.

mod handlers_common;

use handlers_common::{OTHER_WS, WS, call, fixture};
use serde_json::json;

async fn seed_member(fx: &handlers_common::Fixture) {
    fx.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "WorkspaceMember" ("workspaceId", "userId", "role") VALUES (?1, 1001, 'owner')"#,
                chatbotx_db::rusqlite::params![WS],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn schedule_create_get_update_members_list_and_delete() {
    let fx = fixture().await;
    seed_member(&fx).await;
    // `create returns 201 with the single resource shape`.
    let (status, out) = call(
        &fx,
        "workingHours.create",
        WS,
        &[],
        None,
        Some(json!({
            "name": " Day shift ",
            "intervals": [
                { "weekday": 2, "startMinute": 540, "endMinute": 1020 },
                { "weekday": 1, "startMinute": 540, "endMinute": 1020 }
            ],
            "userIds": ["1001"]
        })),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    assert_eq!(out["name"], "Day shift");
    assert_eq!(out["intervals"][0]["weekday"], 1, "sorted by weekday");
    assert_eq!(out["memberUserIds"], json!(["1001"]));
    assert!(out.get("dateOverrides").is_none());
    let id = out["id"].as_str().unwrap().to_owned();

    // `get scopes the lookup to the caller's workspace`.
    let (status, got) = call(&fx, "workingHours.get", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{got}");
    assert_eq!(got["dateOverrides"], json!([]));
    let (status, got) = call(
        &fx,
        "workingHours.get",
        OTHER_WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(
        (status, got["message"].clone()),
        (404, json!("Working hours not found"))
    );

    // `update passes only the changed fields`: a date override is added.
    let (status, out) = call(
        &fx,
        "workingHours.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "dateOverrides": [{ "date": "2026-12-24", "name": "Eve", "ranges": [] }] })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["name"], "Day shift");
    assert_eq!(out["dateOverrides"][0]["date"], "2026-12-24");

    // The refine: none of the fields is a 422.
    let (status, _) = call(
        &fx,
        "workingHours.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 422);
    // Overlapping intervals are a 422.
    let (status, _) = call(
        &fx,
        "workingHours.update",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "intervals": [
            { "weekday": 1, "startMinute": 540, "endMinute": 700 },
            { "weekday": 1, "startMinute": 600, "endMinute": 800 }
        ] })),
    )
    .await;
    assert_eq!(status, 422);

    // A non-member is a 422 `validation` error; `[]` removes all.
    let (status, out) = call(
        &fx,
        "workingHours.setMembers",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "userIds": ["1002"] })),
    )
    .await;
    assert_eq!(
        (status, out["code"].clone()),
        (422, json!("validation")),
        "{out}"
    );
    let (status, out) = call(
        &fx,
        "workingHours.setMembers",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "userIds": [] })),
    )
    .await;
    assert_eq!(
        (status, out["memberUserIds"].clone()),
        (200, json!([])),
        "{out}"
    );

    let (status, list) = call(&fx, "workingHours.list", WS, &[], None, None).await;
    assert_eq!(
        (status, list["data"].as_array().unwrap().len()),
        (200, 1),
        "{list}"
    );

    let (status, _) = call(&fx, "workingHours.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 204);
    let (status, _) = call(&fx, "workingHours.delete", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn holidays_create_update_list_and_delete() {
    let fx = fixture().await;
    let body = json!({ "date": "2026-01-01", "name": "New Year", "ranges": [] });
    let (status, out) = call(
        &fx,
        "workingHours.createHoliday",
        WS,
        &[],
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    assert_eq!(
        (out["date"].clone(), out["name"].clone()),
        (json!("2026-01-01"), json!("New Year"))
    );
    let id = out["id"].as_str().unwrap().to_owned();

    // One holiday per date.
    let (status, out) = call(&fx, "workingHours.createHoliday", WS, &[], None, Some(body)).await;
    assert_eq!(
        (status, out["message"].clone()),
        (422, json!("A holiday already exists on 2026-01-01"))
    );
    // Not a real calendar date.
    let (status, _) = call(
        &fx,
        "workingHours.createHoliday",
        WS,
        &[],
        None,
        Some(json!({ "date": "2026-02-30", "ranges": [] })),
    )
    .await;
    assert_eq!(status, 422);

    let (status, out) = call(
        &fx,
        "workingHours.updateHoliday",
        WS,
        &[("id", &id)],
        None,
        Some(json!({ "name": null, "ranges": [{ "startMinute": 600, "endMinute": 720 }] })),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["name"], json!(null));
    assert_eq!(out["ranges"][0]["endMinute"], 720);

    let (status, list) = call(&fx, "workingHours.listHolidays", WS, &[], None, None).await;
    assert_eq!((status, list["data"].as_array().unwrap().len()), (200, 1));
    let (_, list) = call(&fx, "workingHours.listHolidays", OTHER_WS, &[], None, None).await;
    assert_eq!(list["data"], json!([]));

    let (status, _) = call(
        &fx,
        "workingHours.deleteHoliday",
        OTHER_WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = call(
        &fx,
        "workingHours.deleteHoliday",
        WS,
        &[("id", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
}
