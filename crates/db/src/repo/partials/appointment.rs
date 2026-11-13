//! Port of `partials/appointment.ts` (`appointmentScheduleWindowConfigSchema`).

use serde::{Deserialize, Serialize};

use crate::repo::{Error, Result};

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
    let config: AppointmentScheduleWindowConfig =
        serde_json::from_value(value.clone()).map_err(|e| Error::msg(e.to_string()))?;
    let min_advance_days = match &config {
        AppointmentScheduleWindowConfig::RollingDays {
            rolling_days,
            min_advance_days,
        } => {
            if !(1..=365).contains(rolling_days) {
                return Err(Error::msg("rollingDays: must be between 1 and 365"));
            }
            *min_advance_days
        }
        AppointmentScheduleWindowConfig::DateRange {
            start_date,
            end_date,
            min_advance_days,
        } => {
            iso_date("startDate", start_date)?;
            iso_date("endDate", end_date)?;
            if end_date < start_date {
                return Err(Error::msg("endDate: endDate must be on or after startDate"));
            }
            *min_advance_days
        }
        AppointmentScheduleWindowConfig::SpecificDay {
            date,
            min_advance_days,
        } => {
            iso_date("date", date)?;
            *min_advance_days
        }
        AppointmentScheduleWindowConfig::AnyFutureDate { min_advance_days } => *min_advance_days,
    };
    if min_advance_days < 0 {
        return Err(Error::msg(
            "minAdvanceDays: Too small: expected number to be >=0",
        ));
    }
    Ok(config)
}

/// `z.iso.date()`: `YYYY-MM-DD`, a real calendar date.
fn iso_date(path: &str, value: &str) -> Result<()> {
    let ok = value.len() == 10 && chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok();
    if ok {
        Ok(())
    } else {
        Err(Error::msg(format!("{path}: Invalid ISO date")))
    }
}
