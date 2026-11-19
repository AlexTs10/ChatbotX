//! Port of `apps/builder/__tests__/whatsapp-calling-public-api.test.ts`,
//! `whatsapp-calling-settings-action.test.ts` and `whatsapp-call-hours-action.test.ts`
//! (the operations under `lib/calling-operations.ts`). Adapted: the mocked
//! Graph calls become a local Graph server (`ApiState.graph_url`) and the
//! mocked service writes become the stored row. The settings cache is not
//! ported, so its invalidation cases have no Rust twin.
mod handlers_common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture, scalar};
use serde_json::{Value, json};

#[derive(Default)]
struct Meta {
    /// `GET /settings` fails.
    fail_read: bool,
    /// `POST /settings` is refused.
    refuse_write: bool,
    posts: Vec<Value>,
}

fn reply(status: StatusCode, body: Value) -> Response {
    (status, body.to_string()).into_response()
}

async fn graph(meta: Arc<Mutex<Meta>>, request: Request<Body>) -> Response {
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let body = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .unwrap();
    let mut m = meta.lock().unwrap();
    match (method.as_str(), path.as_str()) {
        ("GET", "/v23.0/PH/settings") if m.fail_read => reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": { "code": 1, "message": "down" } }),
        ),
        ("GET", "/v23.0/PH/settings") => reply(
            StatusCode::OK,
            json!({ "calling": { "status": "ENABLED", "call_hours": { "holiday_schedule": [
                { "date": "2000-01-01", "start_time": "0000", "end_time": "2359" },
                { "date": "2999-12-24", "start_time": "0000", "end_time": "2359" },
            ] } } }),
        ),
        ("POST", "/v23.0/PH/settings") if m.refuse_write => reply(
            StatusCode::BAD_REQUEST,
            json!({ "error": { "code": 100, "message": "Invalid", "error_user_msg": "Calling is not available for this number." } }),
        ),
        ("POST", "/v23.0/PH/settings") => {
            m.posts
                .push(serde_json::from_slice(&body).unwrap_or(Value::Null));
            reply(StatusCode::OK, json!({ "success": true }))
        }
        _ => reply(
            StatusCode::NOT_FOUND,
            json!({ "error": { "message": "no route" } }),
        ),
    }
}

async fn setup() -> (Fixture, Arc<Mutex<Meta>>) {
    let mut f = fixture().await;
    let meta = Arc::new(Mutex::new(Meta::default()));
    let state = Arc::clone(&meta);
    let app = Router::new().fallback(move |r: Request<Body>| graph(Arc::clone(&state), r));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    f.state.graph_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    f.state
        .db
        .write(|tx| {
            let auth = json!({ "tokens": { "accessToken": "tok" }, "metadata": { "phoneNumber": { "id": "PH" } } }).to_string();
            tx.execute(
                r#"UPDATE "Workspace" SET "timezone" = 'Europe/Berlin' WHERE "id" = ?1"#,
                params![WS],
            )?;
            tx.execute(
                r#"INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId")
                   VALUES (71, 'Number', 'whatsapp', 's71', ?1), (81, 'Far', 'whatsapp', 's81', ?2)"#,
                params![WS, OTHER_WS],
            )?;
            tx.execute(
                r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "workspaceId", "inboxId")
                   VALUES (72, ?1, 'PH', 'waba-1', 'b', 'Number', ?2, 71), (82, ?1, 'PH-FAR', 'waba-2', 'b', 'Far', ?3, 81)"#,
                params![auth, WS, OTHER_WS],
            )?;
            Ok(())
        })
        .await
        .expect("seed");
    (f, meta)
}

fn week(open: i64, close: i64) -> Value {
    let days: Vec<Value> = [
        "MONDAY",
        "TUESDAY",
        "WEDNESDAY",
        "THURSDAY",
        "FRIDAY",
        "SATURDAY",
        "SUNDAY",
    ]
    .iter()
    .enumerate()
    .map(|(i, d)| {
        let ranges = if i == 0 {
            json!([{ "openMinute": open, "closeMinute": close }])
        } else {
            json!([])
        };
        json!({ "dayOfWeek": d, "ranges": ranges })
    })
    .collect();
    json!({ "enabled": true, "timezoneId": "Europe/Berlin", "days": days })
}

#[tokio::test]
async fn get_calling_settings_returns_the_stored_settings_and_the_default_hours_input() {
    let (f, _) = setup().await;
    let (status, body) = call(
        &f,
        "whatsappChannels.getCallingSettings",
        WS,
        &[("id", "72")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["callingEnabled"], Value::Null);
    assert_eq!(body["callRecordingRetentionDays"], 90);
    assert_eq!(body["callHours"], Value::Null);
    let input = &body["callHoursInput"];
    assert_eq!(input["enabled"], false);
    assert_eq!(input["timezoneId"], "Europe/Berlin");
    assert_eq!(
        input["days"][0]["ranges"],
        json!([{ "openMinute": 540, "closeMinute": 1020 }])
    );
    assert_eq!(input["days"][6]["ranges"], json!([]));
}

#[tokio::test]
async fn reports_a_number_that_does_not_belong_to_the_workspace() {
    let (f, _) = setup().await;
    let (status, body) = call(
        &f,
        "whatsappChannels.getCallingSettings",
        WS,
        &[("id", "82")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404, "{body}");
    let (status, _) = call(
        &f,
        "whatsappChannels.updateCallHours",
        WS,
        &[("id", "82")],
        None,
        Some(week(540, 600)),
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn sends_meta_fields_to_meta_first_then_mirrors_calling_enabled() {
    let (f, meta) = setup().await;
    let (status, body) = call(
        &f,
        "whatsappChannels.updateCallingSettings",
        WS,
        &[("id", "72")],
        None,
        Some(json!({ "status": "ENABLED", "inboundCallsEnabled": false })),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    assert_eq!(
        meta.lock().unwrap().posts,
        vec![json!({ "calling": { "status": "ENABLED" } })]
    );
    let enabled = scalar(
        &f,
        r#"SELECT "callingEnabled" FROM "IntegrationWhatsapp" WHERE "id" = ?1"#,
        72,
    )
    .await;
    assert_eq!(enabled, Some(json!(1)));
    let inbound = scalar(
        &f,
        r#"SELECT "inboundCallsEnabled" FROM "IntegrationWhatsapp" WHERE "id" = ?1"#,
        72,
    )
    .await;
    assert_eq!(inbound, Some(json!(0)));
}

#[tokio::test]
async fn a_pure_local_toggle_skips_meta() {
    let (f, meta) = setup().await;
    let (status, _) = call(
        &f,
        "whatsappChannels.updateCallingSettings",
        WS,
        &[("id", "72")],
        None,
        Some(json!({ "recordingEnabled": true, "callRecordingRetentionDays": 30 })),
    )
    .await;
    assert_eq!(status, 204);
    assert!(meta.lock().unwrap().posts.is_empty());
    let days = scalar(
        &f,
        r#"SELECT "callRecordingRetentionDays" FROM "IntegrationWhatsapp" WHERE "id" = ?1"#,
        72,
    )
    .await;
    assert_eq!(days, Some(json!(30)));
}

#[tokio::test]
async fn transcription_needs_recording_on() {
    let (f, _) = setup().await;
    let (status, body) = call(
        &f,
        "whatsappChannels.updateCallingSettings",
        WS,
        &[("id", "72")],
        None,
        Some(json!({ "callTranscriptionEnabled": true })),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(
        body["message"],
        "Turn on call recording before turning on transcription."
    );
}

#[tokio::test]
async fn surfaces_metas_refusal_and_writes_nothing() {
    let (f, meta) = setup().await;
    meta.lock().unwrap().refuse_write = true;
    let (status, body) = call(
        &f,
        "whatsappChannels.updateCallingSettings",
        WS,
        &[("id", "72")],
        None,
        Some(json!({ "status": "ENABLED" })),
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["message"], "Calling is not available for this number.");
    let enabled = scalar(
        &f,
        r#"SELECT "callingEnabled" FROM "IntegrationWhatsapp" WHERE "id" = ?1"#,
        72,
    )
    .await;
    assert_eq!(enabled, Some(Value::Null));
}

#[tokio::test]
async fn sends_the_whole_call_hours_keeping_upcoming_holidays_and_mirrors_them() {
    let (f, meta) = setup().await;
    let (status, body) = call(
        &f,
        "whatsappChannels.updateCallHours",
        WS,
        &[("id", "72")],
        None,
        Some(week(540, 1020)),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    let posts = meta.lock().unwrap().posts.clone();
    let hours = &posts[0]["calling"]["call_hours"];
    assert_eq!(hours["status"], "ENABLED");
    assert_eq!(hours["timezone_id"], "Europe/Berlin");
    assert_eq!(
        hours["weekly_operating_hours"],
        json!([{ "day_of_week": "MONDAY", "open_time": "0900", "close_time": "1700" }])
    );
    assert_eq!(
        hours["holiday_schedule"],
        json!([{ "date": "2999-12-24", "start_time": "0000", "end_time": "2359" }])
    );
    let stored = scalar(
        &f,
        r#"SELECT "callHours" FROM "IntegrationWhatsapp" WHERE "id" = ?1"#,
        72,
    )
    .await;
    let stored: Value = serde_json::from_str(stored.unwrap().as_str().unwrap()).unwrap();
    assert_eq!(stored["weeklyOperatingHours"][0]["openTime"], "0900");
    assert_eq!(stored["holidaySchedule"][0]["date"], "2999-12-24");
}

#[tokio::test]
async fn never_writes_when_metas_current_settings_cannot_be_read() {
    let (f, meta) = setup().await;
    meta.lock().unwrap().fail_read = true;
    let (status, body) = call(
        &f,
        "whatsappChannels.updateCallHours",
        WS,
        &[("id", "72")],
        None,
        Some(week(540, 1020)),
    )
    .await;
    assert_eq!(status, 502, "{body}");
    assert!(meta.lock().unwrap().posts.is_empty());
}

#[tokio::test]
async fn rejects_invalid_ranges_with_the_path_of_the_issue() {
    let (f, meta) = setup().await;
    let (status, body) = call(
        &f,
        "whatsappChannels.updateCallHours",
        WS,
        &[("id", "72")],
        None,
        Some(week(600, 540)),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(
        body["message"],
        "days.0.ranges.0: A range must close after it opens."
    );
    let mut closed = week(540, 600);
    closed["days"][0]["ranges"] = json!([]);
    let (status, body) = call(
        &f,
        "whatsappChannels.updateCallHours",
        WS,
        &[("id", "72")],
        None,
        Some(closed),
    )
    .await;
    assert_eq!(status, 422);
    assert_eq!(
        body["message"],
        "days: At least one day needs an open range."
    );
    assert!(meta.lock().unwrap().posts.is_empty());
}
