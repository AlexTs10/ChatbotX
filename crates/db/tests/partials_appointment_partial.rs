//! Port of `packages/database/__tests__/appointment-partial.test.ts` (P3.2b, ported:
//! `safeParse(..).success` is `parse_..(..).is_ok()`).

use chatbotx_db::repo::partials::appointment::parse_appointment_schedule_window_config;
use serde_json::json;

#[test]
fn appointment_schedule_window_config_schema_allows_same_day_booking_with_zero_minimum_advance_days()
 {
    let configs = [
        json!({ "scheduleWindowType": "rollingDays", "rollingDays": 30, "minAdvanceDays": 0 }),
        json!({
            "scheduleWindowType": "dateRange",
            "startDate": "2026-08-09",
            "endDate": "2026-08-31",
            "minAdvanceDays": 0
        }),
        json!({ "scheduleWindowType": "specificDay", "date": "2026-08-09", "minAdvanceDays": 0 }),
        json!({ "scheduleWindowType": "anyFutureDate", "minAdvanceDays": 0 }),
    ];
    for config in &configs {
        let parsed = parse_appointment_schedule_window_config(config);
        assert!(parsed.is_ok(), "{config}: {parsed:?}");
    }
}
