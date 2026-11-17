//! Port of the "repeating broadcasts" routes of
//! `apps/builder/__tests__/broadcasts-public-scope.test.ts` (Q.D5): run
//! groups, occurrences, skip and restore. Adapted: the mocked recurrence
//! service becomes `handlers::dispatch` over a real repeating broadcast.

mod handlers_common;

use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

/// Flow 61 and a repeating broadcast (daily 09:00 UTC from 2030-01-01); its id.
async fn repeating() -> (Fixture, String) {
    let f = fixture().await;
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Flow" ("id", "workspaceId", "name") VALUES (61, ?1, 'Promo flow')"#,
                params![WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    let (status, out) = call(
        &f,
        "broadcasts.create",
        WS,
        &[],
        None,
        Some(json!({
            "channel": "omnichannel",
            "subaction": "allContacts",
            "flowId": "61",
            "schedulesType": "recurring",
            "schedulesAt": null,
            "recurrence": {
                "frequency": "daily",
                "interval": 1,
                "timeOfDay": "09:00",
                "timezone": "UTC",
                "startsOn": "2030-01-01",
                "end": { "kind": "never" },
            },
            "contactFilter": { "operator": "and", "conditions": [] },
        })),
    )
    .await;
    assert_eq!(status, 201, "{out}");
    let id = out["id"].as_str().expect("id").to_owned();
    (f, id)
}

#[tokio::test]
async fn list_run_groups_scopes_to_the_tokens_workspace_and_defaults_to_day_groups() {
    let (f, id) = repeating().await;
    let (status, out) = call(
        &f,
        "broadcasts.listRunGroups",
        WS,
        &[("idOrName", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(
        out,
        json!({
            "data": [],
            "nextCursor": null,
            "totalSlots": 0,
            "totalRuns": 0,
            "timezone": "UTC",
            "weekStartsOn": "monday",
        })
    );
    let (status, _) = call(
        &f,
        "broadcasts.listRunGroups",
        OTHER_WS,
        &[("idOrName", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = call(
        &f,
        "broadcasts.listRunGroups",
        WS,
        &[("idOrName", &id)],
        Some("groupBy=year"),
        None,
    )
    .await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn list_occurrences_scopes_to_the_tokens_workspace_and_passes_the_window() {
    let (f, id) = repeating().await;
    let (status, out) = call(
        &f,
        "broadcasts.listOccurrences",
        WS,
        &[("idOrName", &id)],
        Some("from=2026-10-01T00:00:00.000Z&limit=10"),
        None,
    )
    .await;
    assert_eq!(
        (status, out),
        (200, json!({ "data": [], "nextCursor": null }))
    );
    let (status, _) = call(
        &f,
        "broadcasts.listOccurrences",
        OTHER_WS,
        &[("idOrName", &id)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = call(
        &f,
        "broadcasts.listOccurrences",
        WS,
        &[("idOrName", &id)],
        Some("limit=101"),
        None,
    )
    .await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn skip_and_restore_name_the_date_and_the_instant() {
    let (f, id) = repeating().await;
    let body = json!({ "dates": ["2030-12-25"], "occurrenceAts": ["2030-12-26T09:00:00.000Z"] });
    let (status, out) = call(
        &f,
        "broadcasts.skipOccurrence",
        WS,
        &[("id", &id)],
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["excludeDates"], json!(["2030-12-25", "2030-12-26"]));
    assert!(out["nextSendAt"].is_string());
    assert_eq!(out["upcomingRuns"].as_array().map(Vec::len), Some(3));
    let (status, _) = call(
        &f,
        "broadcasts.skipOccurrence",
        OTHER_WS,
        &[("id", &id)],
        None,
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, 404);
    let (status, out) = call(
        &f,
        "broadcasts.restoreOccurrence",
        WS,
        &[("id", &id)],
        None,
        Some(body),
    )
    .await;
    assert_eq!(status, 200, "{out}");
    assert_eq!(out["excludeDates"], json!([]));
}

#[tokio::test]
async fn skip_rejects_a_date_not_on_the_calendar_and_needs_a_date_or_time() {
    let (f, id) = repeating().await;
    for body in [
        json!({ "dates": ["2030-02-30"] }),
        json!({}),
        json!({ "dates": [] }),
        json!({ "occurrenceAts": ["not a date"] }),
    ] {
        let (status, out) = call(
            &f,
            "broadcasts.skipOccurrence",
            WS,
            &[("id", &id)],
            None,
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, 422, "{body} -> {out}");
    }
    let (_, out) = call(
        &f,
        "broadcasts.skipOccurrence",
        WS,
        &[("id", &id)],
        None,
        Some(json!({})),
    )
    .await;
    let messages: Vec<Value> = out["data"]["issues"]
        .as_array()
        .map(|i| i.iter().map(|x| x["message"].clone()).collect())
        .unwrap_or_default();
    assert!(
        messages.contains(&json!("Give at least one date or occurrenceAt.")),
        "{out}"
    );
}
