//! `syncExternalCalendarEvent` create: the `events.insert` body leaves out
//! an absent description, location and attendee list (parity p1-04 `04`).

use chatbotx_worker::appointment::google_event;
use serde_json::json;

#[test]
fn absent_fields_are_left_out_of_the_event() {
    let start = 1_894_006_800_000; // 2030-01-07T09:00:00.000Z
    let event = google_event(
        "abe1",
        (
            "Appointment: Synced".into(),
            None,
            Some(vec![json!({ "email": "ada@parity.test" })]),
        ),
        None,
        (start, start + 30 * 60_000),
        "UTC",
    );
    let mut keys: Vec<&str> = event
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["attendees", "end", "id", "start", "summary"]);
    assert_eq!(
        event["start"],
        json!({ "dateTime": "2030-01-07T09:00:00.000Z", "timeZone": "UTC" })
    );

    let full = google_event(
        "abe2",
        ("T".into(), Some("D".into()), None),
        Some("Room 1"),
        (start, start),
        "UTC",
    );
    assert_eq!(
        (full["description"].clone(), full["location"].clone()),
        (json!("D"), json!("Room 1"))
    );
    assert!(full.get("attendees").is_none());
}
