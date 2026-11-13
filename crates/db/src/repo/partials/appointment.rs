//! Port of `partials/appointment.ts` (`appointmentScheduleWindowConfigSchema`).

use serde::{Deserialize, Serialize};

use crate::repo::Result;

/// `AppointmentScheduleWindowConfig` (tag `scheduleWindowType`; `minAdvanceDays` and
/// `rollingDays` take their zod defaults).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scheduleWindowType", rename_all = "camelCase")]
pub enum AppointmentScheduleWindowConfig {
    #[serde(rename_all = "camelCase")]
    RollingDays {
        #[serde(default = "default_rolling_days")]
        rolling_days: i64,
        #[serde(default)]
        min_advance_days: i64,
    },
    #[serde(rename_all = "camelCase")]
    DateRange {
        start_date: String,
        end_date: String,
        #[serde(default)]
        min_advance_days: i64,
    },
    #[serde(rename_all = "camelCase")]
    SpecificDay {
        date: String,
        #[serde(default)]
        min_advance_days: i64,
    },
    #[serde(rename_all = "camelCase")]
    AnyFutureDate {
        #[serde(default)]
        min_advance_days: i64,
    },
}

fn default_rolling_days() -> i64 {
    30
}

/// `appointmentScheduleWindowConfigSchema.safeParse`: the config, or the first issue
/// (`rollingDays` 1-365, `minAdvanceDays` >= 0, ISO dates, `endDate >= startDate`).
pub fn parse_appointment_schedule_window_config(
    value: &serde_json::Value,
) -> Result<AppointmentScheduleWindowConfig> {
    let _ = value;
    todo!("P3.2b")
}
