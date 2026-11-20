//! Port of the appointment public-route tests
//! (`apps/builder/__tests__/appointment-calendars-public-api.test.ts`,
//! `appointments-public-api.test.ts`, `appointment-reminders-public-api.test.ts`,
//! `appointment-external-calendars-public-api.test.ts`).
//! Adapted: the mocked services become a real migrated database and the
//! routes are called through `handlers::dispatch`.

mod handlers_common;

use std::sync::Arc;

use chatbotx_api::routes::{RouteAuth, route_by_id};
use chatbotx_core::encryption::{Encryption, Keys};
use chatbotx_db::rusqlite::params;
use handlers_common::{Fixture, OTHER_WS, WS, call, fixture};
use serde_json::{Value, json};

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const CONTACT: i64 = 7001;

async fn setup() -> Fixture {
    let mut f = fixture().await;
    f.state = f
        .state
        .clone()
        .with_encryption(Arc::new(Encryption::new(Keys::new(KEY).expect("keys"))));
    f.state
        .db
        .write(|tx| {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (?1, 'Ann', ?2)"#,
                params![CONTACT, WS],
            )?;
            Ok(())
        })
        .await
        .expect("contact");
    f
}

fn id_of(value: &Value) -> String {
    value["id"].as_str().expect("id").to_string()
}

async fn new_calendar(f: &Fixture, name: &str) -> String {
    let (status, body) = call(
        f,
        "appointmentCalendars.create",
        WS,
        &[],
        None,
        Some(json!({ "name": name })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    id_of(&body)
}

/// An update body: open every day 00:00-23:59, no limits.
fn open_calendar(overrides: Value) -> Value {
    let mut body = json!({
        "name": "Consult",
        "active": true,
        "timezone": "UTC",
        "durationMinutes": 30,
        "bufferAfterMinutes": null,
        "locationType": "onlineMeeting",
        "scheduleWindowConfig": { "scheduleWindowType": "rollingDays", "rollingDays": 30 },
        "maxAppointmentsPerUser": null,
        "dailyLimitEnabled": false,
        "allowGroupMeeting": false,
        "availability": (0..7).map(|d| json!({ "weekday": d, "startMinute": 0, "endMinute": 1439 })).collect::<Vec<_>>(),
        "reminders": [],
    });
    if let (Some(base), Value::Object(extra)) = (body.as_object_mut(), overrides) {
        base.extend(extra);
    }
    body
}

async fn first_slot(f: &Fixture, calendar: &str) -> String {
    let now = chatbotx_db::repo::now_ms();
    let start = chatbotx_db::repo::iso(now + 86_400_000);
    let end = chatbotx_db::repo::iso(now + 2 * 86_400_000);
    let query = format!("startDate={start}&endDate={end}&contactId={CONTACT}");
    let (status, body) = call(
        f,
        "appointmentCalendars.getAvailability",
        WS,
        &[("id", calendar)],
        Some(&query),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    body["slots"][0]["startAt"]
        .as_str()
        .expect("a free slot")
        .to_string()
}

#[tokio::test]
async fn appointment_routes_use_the_appointments_scope() {
    for id in [
        "appointmentCalendars.list",
        "appointmentExternalCalendars.list",
        "appointmentReminders.list",
        "appointments.book",
    ] {
        let route = route_by_id(id).expect("route");
        assert_eq!(
            route.auth,
            RouteAuth::WorkspaceToken(&["appointments"]),
            "{id}"
        );
    }
}

#[tokio::test]
async fn create_then_get_returns_the_defaults_and_the_public_url() {
    let f = setup().await;
    let calendar = new_calendar(&f, "Intro call").await;
    let (status, body) = call(
        &f,
        "appointmentCalendars.get",
        WS,
        &[("id", &calendar)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["name"], "Intro call");
    let slug = body["publicLinkSlug"].as_str().expect("slug");
    assert_eq!(body["publicUrl"], format!("http://app/booking/{slug}"));
    // The default reminders (10 minutes, 1 hour, 1 day) reach the edit resource.
    assert_eq!(
        body["reminders"].as_array().map(Vec::len),
        Some(3),
        "{body}"
    );

    // Another workspace does not see it.
    let (status, _) = call(
        &f,
        "appointmentCalendars.get",
        OTHER_WS,
        &[("id", &calendar)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn create_rejects_a_duplicate_name() {
    let f = setup().await;
    new_calendar(&f, "Same").await;
    let (status, body) = call(
        &f,
        "appointmentCalendars.create",
        WS,
        &[],
        None,
        Some(json!({ "name": "Same" })),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["code"], "nameAlreadyExists");
}

#[tokio::test]
async fn update_requires_max_per_day_when_the_daily_limit_is_on() {
    let f = setup().await;
    let calendar = new_calendar(&f, "Limits").await;
    let (status, body) = call(
        &f,
        "appointmentCalendars.update",
        WS,
        &[("id", &calendar)],
        None,
        Some(open_calendar(json!({ "dailyLimitEnabled": true }))),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(body.to_string().contains("maxPerDay"), "{body}");
}

#[tokio::test]
async fn a_new_calendar_keeps_its_empty_window_config_and_bad_configs_name_the_key() {
    let f = setup().await;
    let calendar = new_calendar(&f, "Window").await;
    let params = [("id", calendar.as_str())];
    let get = || call(&f, "appointmentCalendars.get", WS, &params, None, None);
    let (status, body) = get().await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["scheduleWindowConfig"], json!({}), "{body}");
    assert_eq!(body["scheduleWindowType"], json!("rollingDays"));

    for (config, path) in [
        (
            json!({ "scheduleWindowType": "dateRange", "startDate": "2030-01-08", "endDate": "2030-01-07" }),
            json!(["scheduleWindowConfig", "endDate"]),
        ),
        (
            json!({ "scheduleWindowType": "rollingDays", "rollingDays": 366 }),
            json!(["scheduleWindowConfig", "rollingDays"]),
        ),
    ] {
        let update = open_calendar(json!({ "scheduleWindowConfig": config }));
        let (status, body) = call(
            &f,
            "appointmentCalendars.update",
            WS,
            &[("id", &calendar)],
            None,
            Some(update),
        )
        .await;
        assert_eq!(status, 422, "{body}");
        let paths: Vec<Value> = body["data"]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|issue| issue["path"].clone())
            .collect();
        assert!(paths.contains(&path), "{path}: {body}");
    }

    // A saved config is returned with its type and defaults.
    let update = open_calendar(json!({}));
    let (status, body) = call(
        &f,
        "appointmentCalendars.update",
        WS,
        &[("id", &calendar)],
        None,
        Some(update),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    let (_, body) = get().await;
    assert_eq!(
        body["scheduleWindowConfig"],
        json!({ "scheduleWindowType": "rollingDays", "rollingDays": 30, "minAdvanceDays": 0 })
    );
}

#[tokio::test]
async fn update_rejects_an_invalid_duration_and_overlapping_end() {
    let f = setup().await;
    let calendar = new_calendar(&f, "Bad").await;
    let body = open_calendar(json!({
        "durationMinutes": 7,
        "availability": [{ "weekday": 1, "startMinute": 600, "endMinute": 540 }],
    }));
    let (status, body) = call(
        &f,
        "appointmentCalendars.update",
        WS,
        &[("id", &calendar)],
        None,
        Some(body),
    )
    .await;
    assert_eq!(status, 422, "{body}");
}

#[tokio::test]
async fn list_paginates_and_searches() {
    let f = setup().await;
    new_calendar(&f, "Alpha").await;
    new_calendar(&f, "Beta").await;
    let (status, body) = call(
        &f,
        "appointmentCalendars.list",
        WS,
        &[],
        Some("search=alp&perPage=1"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["data"][0]["name"], "Alpha");
}

#[tokio::test]
async fn duplicate_makes_an_inactive_copy() {
    let f = setup().await;
    let calendar = new_calendar(&f, "Orig").await;
    let (status, body) = call(
        &f,
        "appointmentCalendars.duplicate",
        WS,
        &[("id", &calendar)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let copy = id_of(&body);
    let (_, got) = call(
        &f,
        "appointmentCalendars.get",
        WS,
        &[("id", &copy)],
        None,
        None,
    )
    .await;
    assert_eq!(got["active"], false);
    assert_ne!(got["name"], "Orig");
}

#[tokio::test]
async fn book_list_cancel_and_delete_an_appointment() {
    let f = setup().await;
    let calendar = new_calendar(&f, "Booking").await;
    let (status, body) = call(
        &f,
        "appointmentCalendars.update",
        WS,
        &[("id", &calendar)],
        None,
        Some(open_calendar(json!({}))),
    )
    .await;
    assert_eq!(status, 204, "{body}");
    let slot = first_slot(&f, &calendar).await;

    let (status, booked) = call(
        &f,
        "appointments.book",
        WS,
        &[],
        None,
        Some(json!({ "calendarId": calendar, "contactId": CONTACT.to_string(), "startAt": slot })),
    )
    .await;
    assert_eq!(status, 201, "{booked}");
    assert_eq!(booked["status"], "scheduled");
    let appointment = id_of(&booked);

    // The same start again is not free any more.
    let (status, again) = call(
        &f,
        "appointments.book",
        WS,
        &[],
        None,
        Some(json!({ "calendarId": calendar, "contactId": CONTACT.to_string(), "startAt": slot })),
    )
    .await;
    assert_eq!(status, 409, "{again}");

    let (status, list) = call(&f, "appointments.list", WS, &[], Some("tab=next"), None).await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["data"][0]["id"], appointment);
    assert_eq!(list["data"][0]["calendarName"], "Consult");
    assert_eq!(list["data"][0]["cancellable"], true);
    assert!(
        list["data"][0]["scheduleUrl"]
            .as_str()
            .unwrap_or("")
            .contains("/booking/schedule")
    );

    let (status, cancelled) = call(
        &f,
        "appointments.cancel",
        WS,
        &[("id", &appointment)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 200, "{cancelled}");
    assert_eq!(cancelled["status"], "cancelled");

    // A cancelled appointment cannot be cancelled again.
    let (status, _) = call(
        &f,
        "appointments.cancel",
        WS,
        &[("id", &appointment)],
        None,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, 409);

    let (status, _) = call(
        &f,
        "appointments.delete",
        WS,
        &[("id", &appointment)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (status, _) = call(
        &f,
        "appointments.get",
        WS,
        &[("id", &appointment)],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn book_requires_a_conversation_for_an_inbox() {
    let f = setup().await;
    let calendar = new_calendar(&f, "Inbox").await;
    let (status, body) = call(
        &f,
        "appointments.book",
        WS,
        &[],
        None,
        Some(json!({
            "calendarId": calendar,
            "contactId": CONTACT.to_string(),
            "inboxId": "5",
            "startAt": "2030-01-01T10:00:00Z",
        })),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(
        body.to_string().contains("inboxId requires conversationId"),
        "{body}"
    );
}

#[tokio::test]
async fn reminders_and_external_calendars_lists_start_empty() {
    let f = setup().await;
    let (status, body) = call(&f, "appointmentReminders.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["data"], json!([]));
    let (status, body) = call(&f, "appointmentExternalCalendars.list", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, json!({ "data": [], "pageCount": 1 }));
}

#[tokio::test]
async fn disconnect_an_unknown_connection_is_not_found() {
    let f = setup().await;
    let (status, _) = call(
        &f,
        "appointmentExternalCalendars.disconnect",
        WS,
        &[("integrationId", "999")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 404);
}
