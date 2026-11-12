//! Port of `packages/database/__tests__/appointment-calendar-repository.test.ts` (P3.2).
//! The mocked transaction becomes real reminder rows of a migrated SQLite file.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::appointment_calendar::{ReminderInput, replace_reminders};
use chatbotx_db::repo::{self};
use common::{fresh_db, seed};
use repo_common::{insert_flow, ok};

/// Port of: packages/database/__tests__/appointment-calendar-repository.test.ts:85 "appointmentCalendarRepository.replaceReminders > keeps unchanged reminders so dispatch history is not cascaded"
#[tokio::test]
async fn appointment_calendar_repository_replace_reminders_keeps_unchanged_reminders_so_dispatch_history_is_not_cascaded()
 {
    let db = fresh_db().await;
    seed(&db).await;
    repo_common::setup(&db, |tx| {
        for flow in [601, 602, 603] {
            insert_flow(tx, flow, 2001)?;
        }
        ins!(tx, "AppointmentCalendar", { "id" => 50_i64, "workspaceId" => 2001_i64, "name" => "Cal", "timezone" => "UTC", "locationType" => "inPerson", "publicLinkSlug" => "cal" })?;
        ins!(tx, "AppointmentCalendarReminder", { "id" => 1_i64, "calendarId" => 50_i64, "flowId" => 601_i64, "timingValue" => 1_i64, "timingUnit" => "hours" })?;
        ins!(tx, "AppointmentCalendarReminder", { "id" => 2_i64, "calendarId" => 50_i64, "flowId" => 602_i64, "timingValue" => 2_i64, "timingUnit" => "days" })?;
        Ok(())
    })
    .await;
    let wanted = vec![
        ReminderInput {
            flow_id: 601,
            timing_value: 1,
            timing_unit: "hours".into(),
        },
        ReminderInput {
            flow_id: 603,
            timing_value: 30,
            timing_unit: "minutes".into(),
        },
    ];
    let result = ok(repo::write(&db, move |tx| replace_reminders(tx, 50, &wanted)).await);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].id, 1);
    assert_eq!(
        (
            result[1].flow_id,
            result[1].timing_value,
            result[1].timing_unit.as_str()
        ),
        (603, 30, "minutes")
    );
    let ids = repo_common::rows(
        &db,
        r#"SELECT "id" FROM "AppointmentCalendarReminder" ORDER BY "id""#,
        vec![],
    )
    .await;
    // The kept reminder keeps its id; the dropped one (2) is gone.
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0][0], repo_common::int(1));
}
