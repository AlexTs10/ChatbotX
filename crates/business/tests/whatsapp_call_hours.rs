//! The call-hours form of `features/integration-whatsapp/calling` (`callHoursFormSchema`,
//! `toMetaCallHours`, `toCallHoursSnapshot`, `buildCallHoursFormValues`,
//! `upcomingHolidays`), as `whatsapp-call-hours-action.test.ts` drives them.
use chatbotx_business::whatsapp_voip_actions::{
    call_hours_form_values, call_hours_issue_message, parse_call_hours, to_call_hours_snapshot,
    to_meta_call_hours, upcoming_holidays,
};
use serde_json::{Value, json};

fn form(monday: Value) -> Value {
    let days: Vec<Value> = ["MONDAY", "TUESDAY", "WEDNESDAY", "THURSDAY", "FRIDAY", "SATURDAY", "SUNDAY"]
        .iter()
        .enumerate()
        .map(|(i, d)| json!({ "dayOfWeek": d, "ranges": if i == 0 { monday.clone() } else { json!([]) } }))
        .collect();
    json!({ "enabled": false, "timezoneId": "Europe/Berlin", "days": days })
}

#[test]
fn turning_call_hours_off_still_sends_the_schedule() {
    let parsed = parse_call_hours(&form(json!([{ "openMinute": 780, "closeMinute": 1020 }, { "openMinute": 540, "closeMinute": 720 }]))).unwrap();
    let meta = to_meta_call_hours(&parsed, None);
    assert_eq!(meta["status"], "DISABLED");
    assert_eq!(
        meta["weekly_operating_hours"],
        json!([
            { "day_of_week": "MONDAY", "open_time": "0900", "close_time": "1200" },
            { "day_of_week": "MONDAY", "open_time": "1300", "close_time": "1700" },
        ])
    );
    assert!(meta.get("holiday_schedule").is_none());
    let snapshot = to_call_hours_snapshot(&meta);
    assert_eq!(
        snapshot["weeklyOperatingHours"][0],
        json!({ "dayOfWeek": "MONDAY", "openTime": "0900", "closeTime": "1200" })
    );
    // The snapshot reads back as the same form.
    let back = call_hours_form_values(Some(&snapshot), "UTC");
    assert_eq!(
        back["days"][0]["ranges"],
        json!([{ "openMinute": 540, "closeMinute": 720 }, { "openMinute": 780, "closeMinute": 1020 }])
    );
    assert_eq!(back["timezoneId"], "Europe/Berlin");
}

#[test]
fn rejects_overlaps_days_out_of_order_and_unknown_zones() {
    let overlap = parse_call_hours(&form(
        json!([{ "openMinute": 540, "closeMinute": 720 }, { "openMinute": 600, "closeMinute": 800 }]),
    ));
    assert_eq!(
        call_hours_issue_message(&overlap.unwrap_err()),
        "days.0.ranges: The two ranges of a day must not overlap."
    );
    let mut swapped = form(json!([{ "openMinute": 540, "closeMinute": 720 }]));
    swapped["days"][1]["dayOfWeek"] = json!("SUNDAY");
    assert_eq!(
        call_hours_issue_message(&parse_call_hours(&swapped).unwrap_err()),
        "days.1.dayOfWeek: Days must be listed in week order"
    );
    let mut zone = form(json!([{ "openMinute": 540, "closeMinute": 720 }]));
    zone["timezoneId"] = json!("Mars/Olympus");
    assert!(parse_call_hours(&zone).is_err());
    let late = form(json!([{ "openMinute": 540, "closeMinute": 1440 }]));
    assert!(parse_call_hours(&late).is_err());
}

#[test]
fn keeps_only_the_holidays_not_yet_past() {
    let holidays = json!([{ "date": "2026-01-01" }, { "date": "2026-06-01" }]);
    // 2026-03-01T00:00:00Z
    let kept = upcoming_holidays(Some(&holidays), "Europe/Berlin", 1_772_323_200_000).unwrap();
    assert_eq!(kept, vec![json!({ "date": "2026-06-01" })]);
    assert_eq!(upcoming_holidays(None, "UTC", 0), None);
}

#[test]
fn the_default_form_is_weekdays_nine_to_five_and_off() {
    let values = call_hours_form_values(None, "Not/AZone");
    assert_eq!(values["enabled"], false);
    assert_eq!(values["timezoneId"], "Etc/UTC");
    assert_eq!(
        values["days"][4]["ranges"],
        json!([{ "openMinute": 540, "closeMinute": 1020 }])
    );
    assert_eq!(values["days"][5]["ranges"], json!([]));
}
