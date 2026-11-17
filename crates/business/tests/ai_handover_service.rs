//! Ports of `packages/business/__tests__/ai-handover-settings-policy.test.ts`,
//! `ai-handover-settings-service.test.ts` and
//! `ai-handover-bulk-run-service.test.ts` (Q.D6).
//! Adapted: the mocked repositories, inbox / flow / workspace services and
//! the transaction become a real migrated database; the mocked
//! `integrationQueue.add` becomes a recording job enqueuer.
//! Not applicable: the settings cache tests (no read caches, P3.6-D1), the
//! template lock / `assertReferencable` assertions (P3.6-D1), the
//! "between the two locks" / "re-check under the lock" races (one SQLite
//! writer: the request sees one state), and the worker engine / sweeper
//! surface (`refundIfPreviousChunkQueued`, `findRunInbox`, the page / claim
//! helpers: unit Q.W2).

mod common;
mod x1_support;

use std::sync::Arc;

use chatbotx_business::ai_handover::{
    self as service, SaveSettingsInput, SetApplyToAllInput, SettingsPatch, TimeRange, codes,
};
use chatbotx_business::tx;
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::queries::ai_handover_bulk_eligibility::AiHandoverBulkAction;
use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_jobs::{Enqueuer, Retention};
use common::{OTHER_WS, WS, setup, two_workspaces};
use serde_json::json;
use x1_support::RecordingEnqueuer;

const INBOX: i64 = 3001;
const WHATSAPP: i64 = 3002;
const FOREIGN: i64 = 3003;
const USER: i64 = 1001;
const FLOW: i64 = 4001;

fn at(iso: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(iso)
        .expect("iso")
        .timestamp_millis()
}

fn ranges(pairs: &[(u32, u32)]) -> Vec<TimeRange> {
    pairs
        .iter()
        .map(|&(from, to)| TimeRange { from, to })
        .collect()
}

// ------------------------------------------------------------ policy

const UTC: &str = "UTC";
const VIETNAM: &str = "Asia/Ho_Chi_Minh";

#[test]
fn is_inclusive_at_the_start_hour_and_exclusive_at_the_end_hour() {
    let office = ranges(&[(8, 17)]);
    let within = |iso| service::is_within_hour_ranges(&office, UTC, at(iso));
    assert!(within("2026-10-01T08:00:00Z"));
    assert!(within("2026-10-01T16:59:59Z"));
    assert!(!within("2026-10-01T17:00:00Z"));
    assert!(!within("2026-10-01T07:59:59Z"));
}

#[test]
fn a_range_wrapping_past_midnight_matches_both_sides_of_midnight() {
    let night = ranges(&[(22, 6)]);
    let within = |iso| service::is_within_hour_ranges(&night, UTC, at(iso));
    assert!(within("2026-10-01T23:30:00Z"));
    assert!(within("2026-10-01T05:59:00Z"));
    assert!(!within("2026-10-01T06:00:00Z"));
    assert!(!within("2026-10-01T12:00:00Z"));
}

#[test]
fn zero_to_24_is_all_day() {
    let all_day = ranges(&[(0, 24)]);
    assert!(service::is_within_hour_ranges(
        &all_day,
        UTC,
        at("2026-10-01T00:00:00Z")
    ));
    assert!(service::is_within_hour_ranges(
        &all_day,
        UTC,
        at("2026-10-01T23:59:59Z")
    ));
}

#[test]
fn a_range_with_equal_bounds_never_matches() {
    let empty = ranges(&[(0, 0), (9, 9)]);
    assert!(!service::is_within_hour_ranges(
        &empty,
        UTC,
        at("2026-10-01T00:00:00Z")
    ));
    assert!(!service::is_within_hour_ranges(
        &empty,
        UTC,
        at("2026-10-01T09:30:00Z")
    ));
}

#[test]
fn multiple_ranges_are_or_ed() {
    let split = ranges(&[(8, 12), (14, 18)]);
    assert!(service::is_within_hour_ranges(
        &split,
        UTC,
        at("2026-10-01T09:00:00Z")
    ));
    assert!(!service::is_within_hour_ranges(
        &split,
        UTC,
        at("2026-10-01T13:00:00Z")
    ));
    assert!(service::is_within_hour_ranges(
        &split,
        UTC,
        at("2026-10-01T15:00:00Z")
    ));
}

#[test]
fn no_ranges_never_matches() {
    assert!(!service::is_within_hour_ranges(
        &[],
        UTC,
        at("2026-10-01T09:00:00Z")
    ));
}

#[test]
fn evaluates_the_hour_in_the_given_timezone_not_utc() {
    let office = ranges(&[(8, 17)]);
    let now = at("2026-10-01T02:00:00Z");
    assert!(!service::is_within_hour_ranges(&office, UTC, now));
    assert!(service::is_within_hour_ranges(&office, VIETNAM, now));
}

#[test]
fn the_hour_is_taken_across_a_utc_date_boundary() {
    let night = ranges(&[(0, 2)]);
    assert!(service::is_within_hour_ranges(
        &night,
        VIETNAM,
        at("2026-09-30T18:00:00Z")
    ));
}

fn office() -> Vec<TimeRange> {
    ranges(&[(8, 17)])
}

#[test]
fn a_disabled_integration_is_never_active_schedule_or_not() {
    let noon = at("2026-10-01T12:00:00Z");
    assert!(!service::is_ai_handover_active(
        false,
        false,
        &office(),
        UTC,
        noon
    ));
    assert!(!service::is_ai_handover_active(
        false,
        true,
        &office(),
        UTC,
        noon
    ));
}

#[test]
fn enabled_without_a_schedule_is_always_active() {
    assert!(service::is_ai_handover_active(
        true,
        false,
        &office(),
        UTC,
        at("2026-10-01T00:00:00Z")
    ));
}

#[test]
fn enabled_with_a_schedule_is_active_only_inside_a_window() {
    assert!(service::is_ai_handover_active(
        true,
        true,
        &office(),
        UTC,
        at("2026-10-01T12:00:00Z")
    ));
    assert!(!service::is_ai_handover_active(
        true,
        true,
        &office(),
        UTC,
        at("2026-10-01T00:00:00Z")
    ));
}

#[test]
fn a_schedule_with_no_ranges_is_never_active() {
    assert!(!service::is_ai_handover_active(
        true,
        true,
        &[],
        UTC,
        at("2026-10-01T12:00:00Z")
    ));
}

// ------------------------------------------------------------ fixtures

/// Two workspaces; `WS` has a Messenger Page, a WhatsApp inbox and an
/// active flow; `OTHER_WS` has a Messenger Page.
async fn seeded() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        common::inbox(tx, INBOX, WS, "messenger")?;
        common::inbox(tx, WHATSAPP, WS, "whatsapp")?;
        common::inbox(tx, FOREIGN, OTHER_WS, "messenger")?;
        common::flow(tx, FLOW, WS)?;
        Ok(())
    })
    .await;
    db
}

/// The Page's settings row.
#[derive(Clone, Copy)]
struct Row {
    enabled: bool,
    schedule: Option<(u32, u32)>,
    apply: bool,
    revision: i64,
}

const ON: Row = Row {
    enabled: true,
    schedule: None,
    apply: false,
    revision: 0,
};

async fn with_settings(db: &common::TestDb, row: Row) {
    setup(db, move |tx| insert_settings(tx, row)).await;
}

fn insert_settings(tx: &Connection, row: Row) -> Result<(), chatbotx_db::DbError> {
    let ranges = row
        .schedule
        .map_or(json!([]), |(from, to)| json!([{ "from": from, "to": to }]));
    tx.execute(
        r#"INSERT INTO "AIHandoverSettings" ("id", "workspaceId", "inboxId", "channel", "enabled", "scheduleEnabled",
             "timeRanges", "applyToAllCustomers", "applyToAllRevision", "applyToAllMessage", "applyToAllRequestedByUserId")
           VALUES (9001, ?1, ?2, 'messenger', ?3, ?4, ?5, ?6, ?7, 'bye', ?8)"#,
        params![
            WS,
            INBOX,
            row.enabled,
            row.schedule.is_some(),
            ranges.to_string(),
            row.apply,
            row.revision,
            USER
        ],
    )?;
    Ok(())
}

fn insert_run(
    tx: &Connection,
    id: i64,
    revision: i64,
    status: &str,
    claim: Option<&str>,
) -> Result<(), chatbotx_db::DbError> {
    tx.execute(
        r#"INSERT INTO "AIHandoverBulkRun" ("id", "workspaceId", "inboxId", "channel", "revision", "action", "status",
             "requestedAt", "claimToken", "attempts", "chunkSeq", "processedCount", "skippedCount", "failedCount")
           VALUES (?1, ?2, ?3, 'messenger', ?4, 'enable', ?5, 0, ?6, 0, 0, 0, 0, 0)"#,
        params![id, WS, INBOX, revision, status, claim],
    )?;
    Ok(())
}

async fn with_run(
    db: &common::TestDb,
    id: i64,
    revision: i64,
    status: &'static str,
    claim: Option<&'static str>,
) {
    setup(db, move |tx| insert_run(tx, id, revision, status, claim)).await;
}

/// `n` threads of the Page that wrote in the last hour (eligible for an ON).
async fn with_recent_threads(db: &common::TestDb, n: i64) {
    setup(db, move |tx| {
        for i in 0..n {
            let contact = 5000 + i;
            common::contact(tx, contact, WS)?;
            common::contact_inbox(
                tx,
                6000 + i,
                contact,
                INBOX,
                "messenger",
                &format!("psid-{i}"),
            )?;
            common::conversation(tx, 7000 + i, WS, contact)?;
            tx.execute(
                r#"UPDATE "ContactInbox" SET "lastIncomingMessageAt" = ?1 WHERE "id" = ?2"#,
                params![now_ms() - 3_600_000, 6000 + i],
            )?;
        }
        Ok(())
    })
    .await;
}

async fn disconnect(db: &common::TestDb) {
    setup(db, |tx| {
        tx.execute(
            r#"UPDATE "Inbox" SET "status" = 'disconnected' WHERE "id" = ?1"#,
            [INBOX],
        )?;
        Ok(())
    })
    .await;
}

async fn read<F, R>(db: &common::TestDb, f: F) -> R
where
    F: FnOnce(&Connection) -> chatbotx_business::errors::Result<R> + Send + 'static,
    R: Send + 'static,
{
    tx::read(db, f).await.expect("read")
}

async fn settings_row(db: &common::TestDb) -> Option<(bool, i64, Option<String>, Option<i64>)> {
    read(db, |c| {
        Ok(service::find_in(c, WS, INBOX)?.map(|s| {
            (
                s.apply_to_all_customers,
                s.apply_to_all_revision,
                s.apply_to_all_message,
                s.apply_to_all_requested_by_user_id,
            )
        }))
    })
    .await
}

async fn runs(db: &common::TestDb) -> Vec<(i64, String, String, Option<String>)> {
    db.read(|c| {
        let mut stmt = c.prepare(
            r#"SELECT "revision", "action", "status", "message" FROM "AIHandoverBulkRun" ORDER BY "revision""#,
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .await
    .expect("runs")
}

fn code(error: &chatbotx_business::BusinessError) -> &str {
    error.code().unwrap_or("")
}

// ------------------------------------------------------------ settings service

#[tokio::test]
async fn returns_the_pages_inbox_when_it_belongs_to_the_workspace_and_its_channel_has_an_ai_hand_off()
 {
    let db = seeded().await;
    let inbox = read(&db, |c| service::require_inbox(c, WS, INBOX)).await;
    assert_eq!(
        (inbox.id, inbox.status.as_str(), inbox.channel.as_str()),
        (INBOX, "connected", "messenger")
    );
}

#[tokio::test]
async fn is_not_found_for_an_inbox_that_is_not_in_this_workspace_or_a_channel_without_an_ai_hand_off()
 {
    let db = seeded().await;
    for inbox in [FOREIGN, WHATSAPP] {
        let error = tx::read(&db, move |c| service::require_inbox(c, WS, inbox))
            .await
            .expect_err("not found");
        assert_eq!(code(&error), "notFound");
    }
}

#[tokio::test]
async fn returns_the_stored_row() {
    let db = seeded().await;
    assert!(
        read(&db, |c| service::find_in(c, WS, INBOX))
            .await
            .is_none()
    );
    with_settings(&db, ON).await;
    let row = read(&db, |c| service::find_in(c, WS, INBOX))
        .await
        .expect("row");
    assert_eq!((row.inbox_id, row.enabled), (INBOX, true));
}

#[tokio::test]
async fn a_disconnected_page_is_never_active_even_with_the_automation_switched_on() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    disconnect(&db).await;
    assert!(
        read(&db, |c| service::find_active_in(c, WS, INBOX, now_ms()))
            .await
            .is_none()
    );
    // The row itself is still readable (the card shows it).
    assert!(
        read(&db, |c| service::find_in(c, WS, INBOX))
            .await
            .is_some()
    );
}

#[tokio::test]
async fn a_workspace_that_does_not_own_the_page_cannot_read_its_settings() {
    // Adapted: no cache entry to poison; the read is scoped by the workspace.
    let db = seeded().await;
    with_settings(&db, ON).await;
    assert!(
        read(&db, |c| service::find_in(c, OTHER_WS, INBOX))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn returns_the_settings_while_the_automation_applies_null_otherwise() {
    let db = seeded().await;
    let now = at("2026-10-01T12:00:00Z");
    assert!(
        read(&db, move |c| service::find_active_in(c, WS, INBOX, now))
            .await
            .is_none()
    );
    with_settings(&db, ON).await;
    assert!(
        read(&db, move |c| service::find_active_in(c, WS, INBOX, now))
            .await
            .is_some()
    );
    setup(&db, |tx| {
        tx.execute(r#"UPDATE "AIHandoverSettings" SET "enabled" = 0"#, [])?;
        Ok(())
    })
    .await;
    assert!(
        read(&db, move |c| service::find_active_in(c, WS, INBOX, now))
            .await
            .is_none()
    );
}

async fn stop_reason(db: &common::TestDb, requires_automation: bool) -> Option<&'static str> {
    let now = at("2026-10-01T12:00:00Z");
    read(db, move |c| {
        service::find_stop_reason(c, WS, INBOX, requires_automation, now)
    })
    .await
}

#[tokio::test]
async fn a_disconnected_page_stops_any_run_whether_or_not_it_needs_the_automation() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    disconnect(&db).await;
    assert_eq!(stop_reason(&db, true).await, Some("pageDisconnected"));
    assert_eq!(stop_reason(&db, false).await, Some("pageDisconnected"));
}

#[tokio::test]
async fn a_hand_over_stops_when_the_automation_is_off_or_outside_its_schedule_a_take_back_does_not_need_it()
 {
    let db = seeded().await;
    with_settings(
        &db,
        Row {
            enabled: false,
            ..ON
        },
    )
    .await;
    assert_eq!(stop_reason(&db, true).await, Some("automationStopped"));
    assert_eq!(stop_reason(&db, false).await, None);

    let db = seeded().await;
    with_settings(
        &db,
        Row {
            schedule: Some((18, 20)),
            ..ON
        },
    )
    .await;
    assert_eq!(stop_reason(&db, true).await, Some("automationStopped"));
}

#[tokio::test]
async fn a_running_automation_on_a_connected_page_does_not_stop_anything() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    assert_eq!(stop_reason(&db, true).await, None);
}

#[tokio::test]
async fn a_connected_page_with_no_saved_settings_has_no_automation_a_hand_over_stops_a_take_back_carries_on()
 {
    let db = seeded().await;
    assert_eq!(stop_reason(&db, true).await, Some("automationStopped"));
    assert_eq!(stop_reason(&db, false).await, None);
}

#[tokio::test]
async fn a_page_whose_inbox_is_gone_counts_as_disconnected() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    setup(&db, |tx| {
        tx.execute(r#"DELETE FROM "Inbox" WHERE "id" = ?1"#, [INBOX])?;
        Ok(())
    })
    .await;
    assert_eq!(stop_reason(&db, true).await, Some("pageDisconnected"));
}

#[tokio::test]
async fn the_connection_state_is_read_fresh_never_from_the_settings_cache() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    assert_eq!(stop_reason(&db, true).await, None);
    disconnect(&db).await;
    assert_eq!(stop_reason(&db, true).await, Some("pageDisconnected"));
}

async fn configured_and_inactive(db: &common::TestDb, now: i64) -> bool {
    read(db, move |c| {
        service::is_configured_and_inactive(c, WS, INBOX, now)
    })
    .await
}

#[tokio::test]
async fn nothing_saved_is_not_inactive_no_automation_exists_to_take_back_for() {
    let db = seeded().await;
    assert!(!configured_and_inactive(&db, at("2026-10-01T12:00:00Z")).await);
}

#[tokio::test]
async fn saved_and_switched_off_is_inactive() {
    let db = seeded().await;
    with_settings(
        &db,
        Row {
            enabled: false,
            ..ON
        },
    )
    .await;
    assert!(configured_and_inactive(&db, at("2026-10-01T12:00:00Z")).await);
}

#[tokio::test]
async fn saved_and_active_is_not_inactive() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    assert!(!configured_and_inactive(&db, at("2026-10-01T12:00:00Z")).await);
}

#[tokio::test]
async fn saved_with_a_schedule_is_inactive_outside_its_windows_only() {
    let db = seeded().await;
    with_settings(
        &db,
        Row {
            schedule: Some((18, 20)),
            ..ON
        },
    )
    .await;
    assert!(configured_and_inactive(&db, at("2026-10-01T12:00:00Z")).await);
    assert!(!configured_and_inactive(&db, at("2026-10-01T19:00:00Z")).await);
}

fn save_input() -> SaveSettingsInput {
    SaveSettingsInput {
        workspace_id: WS,
        inbox_id: INBOX,
        enabled: true,
        schedule_enabled: false,
        time_ranges: json!([]),
        goto_flow_id: None,
        return_message: None,
        pause_bot_waiting_for_staff: false,
    }
}

async fn save(
    db: &common::TestDb,
    input: SaveSettingsInput,
) -> chatbotx_business::errors::Result<chatbotx_db::repo::models::AiHandoverSettings> {
    tx::write(db, move |c| service::save_in(c, &input)).await
}

#[tokio::test]
async fn stores_the_settings() {
    let db = seeded().await;
    let row = save(
        &db,
        SaveSettingsInput {
            pause_bot_waiting_for_staff: true,
            ..save_input()
        },
    )
    .await
    .expect("saved");
    assert_eq!(
        (
            row.channel.as_str(),
            row.enabled,
            row.pause_bot_waiting_for_staff
        ),
        ("messenger", true, true)
    );
    assert_eq!(
        (
            row.time_ranges.0.clone(),
            row.goto_flow_id,
            row.return_message
        ),
        (json!([]), None, None)
    );
}

#[tokio::test]
async fn trims_the_return_message_and_stores_blank_as_null() {
    let db = seeded().await;
    let row = save(
        &db,
        SaveSettingsInput {
            return_message: Some("  Back with you  ".into()),
            ..save_input()
        },
    )
    .await
    .expect("saved");
    assert_eq!(row.return_message.as_deref(), Some("Back with you"));
    let row = save(
        &db,
        SaveSettingsInput {
            return_message: Some("   ".into()),
            ..save_input()
        },
    )
    .await
    .expect("saved");
    assert_eq!(row.return_message, None);
}

#[tokio::test]
async fn saves_nothing_for_an_inbox_that_is_not_a_page_of_this_workspace() {
    let db = seeded().await;
    let error = save(
        &db,
        SaveSettingsInput {
            inbox_id: FOREIGN,
            ..save_input()
        },
    )
    .await
    .expect_err("refused");
    assert_eq!(code(&error), "notFound");
    assert!(
        read(&db, |c| service::find_in(c, OTHER_WS, FOREIGN))
            .await
            .is_none()
    );
}

fn field(error: &chatbotx_business::BusinessError) -> Option<String> {
    match error {
        chatbotx_business::BusinessError::Exception { field, .. } => field.clone(),
        _ => None,
    }
}

#[tokio::test]
async fn rejects_a_return_message_over_the_limit_without_writing() {
    let db = seeded().await;
    let error = save(
        &db,
        SaveSettingsInput {
            return_message: Some("x".repeat(2001)),
            ..save_input()
        },
    )
    .await
    .expect_err("too long");
    assert_eq!(field(&error).as_deref(), Some("returnMessage"));
    assert!(
        read(&db, |c| service::find_in(c, WS, INBOX))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn requires_at_least_one_range_when_the_schedule_is_on() {
    let db = seeded().await;
    let error = save(
        &db,
        SaveSettingsInput {
            schedule_enabled: true,
            ..save_input()
        },
    )
    .await
    .expect_err("no range");
    assert_eq!(field(&error).as_deref(), Some("timeRanges"));
}

#[tokio::test]
async fn rejects_an_invalid_range() {
    let db = seeded().await;
    for ranges in [
        json!([{ "from": 9, "to": 9 }]),
        json!([{ "from": 0, "to": 25 }]),
        json!([{ "from": 8.5, "to": 17 }]),
    ] {
        let error = save(
            &db,
            SaveSettingsInput {
                time_ranges: ranges,
                ..save_input()
            },
        )
        .await
        .expect_err("invalid");
        assert_eq!(field(&error).as_deref(), Some("timeRanges"));
    }
    assert!(
        read(&db, |c| service::find_in(c, WS, INBOX))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn keeps_valid_ranges_including_a_midnight_wrap() {
    let db = seeded().await;
    let ranges = json!([{ "from": 8, "to": 12 }, { "from": 22, "to": 6 }]);
    let row = save(
        &db,
        SaveSettingsInput {
            schedule_enabled: true,
            time_ranges: ranges.clone(),
            ..save_input()
        },
    )
    .await
    .expect("saved");
    assert_eq!(row.time_ranges.0, ranges);
}

#[tokio::test]
async fn rejects_a_goto_flow_that_is_not_an_active_flow_of_the_workspace() {
    let db = seeded().await;
    setup(&db, |tx| {
        common::flow(tx, 4002, OTHER_WS)?;
        common::flow(tx, 4003, WS)?;
        tx.execute(r#"UPDATE "Flow" SET "active" = 0 WHERE "id" = 4003"#, [])?;
        Ok(())
    })
    .await;
    for flow in [4002, 4003, 4999] {
        let error = save(
            &db,
            SaveSettingsInput {
                goto_flow_id: Some(flow),
                ..save_input()
            },
        )
        .await
        .expect_err("flow");
        assert_eq!(
            (code(&error), error.message().as_str()),
            ("notFound", "Flow not found")
        );
    }
    assert!(
        read(&db, |c| service::find_in(c, WS, INBOX))
            .await
            .is_none()
    );
}

#[tokio::test]
async fn accepts_an_active_goto_flow() {
    let db = seeded().await;
    let row = save(
        &db,
        SaveSettingsInput {
            goto_flow_id: Some(FLOW),
            ..save_input()
        },
    )
    .await
    .expect("saved");
    assert_eq!(row.goto_flow_id, Some(FLOW));
}

#[tokio::test]
async fn does_not_look_up_a_flow_when_none_is_selected() {
    // Adapted: no flow at all in the workspace, and the save still succeeds.
    let db = seeded().await;
    setup(&db, |tx| {
        tx.execute(r#"DELETE FROM "Flow""#, [])?;
        Ok(())
    })
    .await;
    assert!(save(&db, save_input()).await.is_ok());
}

// ------------------------------------------------------------ bulk run

fn request(apply: bool, message: Option<&str>) -> SetApplyToAllInput {
    SetApplyToAllInput {
        workspace_id: WS,
        inbox_id: INBOX,
        user_id: Some(USER),
        apply_to_all_customers: apply,
        message: message.map(str::to_owned),
        confirm_max_eligible: None,
    }
}

fn jobs() -> (Arc<RecordingEnqueuer>, Arc<dyn Enqueuer>) {
    let recorder = RecordingEnqueuer::new();
    let dynamic: Arc<dyn Enqueuer> = recorder.clone();
    (recorder, dynamic)
}

#[tokio::test]
async fn bumps_the_revision_creates_the_enable_run_and_dispatches_it() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    let (recorder, jobs) = jobs();
    let change =
        service::set_apply_to_all(&db, Some(&jobs), request(true, Some("ignored for an ON")))
            .await
            .expect("changed");
    let run = change.run.expect("run");
    assert!(change.is_changed);
    // A message never rides along with an ON.
    assert_eq!(settings_row(&db).await, Some((true, 1, None, Some(USER))));
    assert_eq!(
        (
            run.revision,
            run.action.as_str(),
            run.status.as_str(),
            run.message
        ),
        (1, "enable", "pending", None)
    );
    assert_eq!(run.requested_by_user_id, Some(USER));
    let calls = recorder.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "aiHandoverBulkToggle");
    assert_eq!(
        calls[0].data,
        json!({
            "type": "aiHandoverBulkToggle",
            "data": { "runId": run.id.to_string(), "workspaceId": WS.to_string() },
        })
    );
    assert_eq!(
        calls[0].opts.job_id,
        Some(format!("ai-handover-bulk-{}-chunk-0", run.id))
    );
}

#[tokio::test]
async fn is_refused_while_the_pages_automation_is_not_running_and_writes_nothing() {
    let db = seeded().await;
    with_settings(
        &db,
        Row {
            enabled: false,
            ..ON
        },
    )
    .await;
    let (recorder, jobs) = jobs();
    let error = service::set_apply_to_all(&db, Some(&jobs), request(true, None))
        .await
        .expect_err("refused");
    assert_eq!(code(&error), codes::AUTOMATION_NOT_ACTIVE);
    assert_eq!(
        settings_row(&db).await,
        Some((false, 0, Some("bye".into()), Some(USER)))
    );
    assert!(runs(&db).await.is_empty());
    assert!(recorder.calls().is_empty());
}

const APPLIED: Row = Row {
    apply: true,
    revision: 1,
    ..ON
};

#[tokio::test]
async fn trims_the_tagged_message_and_does_not_need_the_automation_to_be_running() {
    let db = seeded().await;
    with_settings(
        &db,
        Row {
            enabled: false,
            ..APPLIED
        },
    )
    .await;
    service::set_apply_to_all(&db, None, request(false, Some("  We are here now  ")))
        .await
        .expect("off");
    assert_eq!(
        settings_row(&db).await,
        Some((false, 2, Some("We are here now".into()), Some(USER)))
    );
    assert_eq!(
        runs(&db).await,
        vec![(
            2,
            "disable".into(),
            "pending".into(),
            Some("We are here now".into())
        )]
    );
}

#[tokio::test]
async fn rejects_a_missing_null_or_blank_message_before_anything_is_locked() {
    let db = seeded().await;
    with_settings(&db, APPLIED).await;
    for message in [None, Some("   \n ")] {
        let error = service::set_apply_to_all(&db, None, request(false, message))
            .await
            .expect_err("required");
        assert_eq!(code(&error), codes::MESSAGE_REQUIRED);
    }
    assert_eq!(settings_row(&db).await.map(|s| s.1), Some(1));
}

#[tokio::test]
async fn rejects_a_message_over_2000_characters_accepts_exactly_2000() {
    let db = seeded().await;
    with_settings(&db, APPLIED).await;
    let long = "a".repeat(2001);
    let error = service::set_apply_to_all(&db, None, request(false, Some(&long)))
        .await
        .expect_err("long");
    assert_eq!(code(&error), codes::MESSAGE_TOO_LONG);
    let exact = "a".repeat(2000);
    service::set_apply_to_all(&db, None, request(false, Some(&exact)))
        .await
        .expect("ok");
    assert_eq!(settings_row(&db).await.map(|s| s.1), Some(2));
}

#[tokio::test]
async fn the_same_state_again_changes_nothing_no_revision_no_cancel_no_run() {
    for desired in [true, false] {
        let db = seeded().await;
        with_settings(
            &db,
            Row {
                apply: desired,
                revision: 3,
                ..ON
            },
        )
        .await;
        with_run(&db, 8001, 3, "running", Some("token")).await;
        let (recorder, jobs) = jobs();
        let change = service::set_apply_to_all(&db, Some(&jobs), request(desired, Some("hi")))
            .await
            .expect("noop");
        assert_eq!((change.is_changed, change.run), (false, None));
        assert_eq!(settings_row(&db).await.map(|s| s.1), Some(3));
        assert_eq!(runs(&db).await[0].2, "running");
        assert!(recorder.calls().is_empty());
    }
}

#[tokio::test]
async fn an_off_leaves_no_trace_no_settings_row_is_created() {
    let db = seeded().await;
    let change = service::set_apply_to_all(&db, None, request(false, Some("hi")))
        .await
        .expect("noop");
    assert!(!change.is_changed);
    assert_eq!(settings_row(&db).await, None);
}

#[tokio::test]
async fn an_on_for_a_page_nobody_configured_is_refused_and_leaves_no_row() {
    // Adapted: the TS test mocks `isActiveNow` true for the freshly created
    // (switched-off) row; with real rows that row has no running automation,
    // so the ON is refused and the created row is rolled back.
    let db = seeded().await;
    let error = service::set_apply_to_all(&db, None, request(true, None))
        .await
        .expect_err("refused");
    assert_eq!(code(&error), codes::AUTOMATION_NOT_ACTIVE);
    assert_eq!(settings_row(&db).await, None);
}

#[tokio::test]
async fn cancels_the_older_revisions_run_in_the_same_transaction_before_creating_the_new_one() {
    let db = seeded().await;
    with_settings(&db, Row { revision: 1, ..ON }).await;
    with_run(&db, 8001, 1, "pending", None).await;
    let change = service::set_apply_to_all(&db, None, request(true, None))
        .await
        .expect("changed");
    assert!(change.run.is_some());
    let all = runs(&db).await;
    assert_eq!((all[0].0, all[0].2.as_str()), (1, "cancelled"));
    assert_eq!((all[1].0, all[1].2.as_str()), (2, "pending"));
}

#[tokio::test]
async fn a_run_still_winding_down_keeps_the_page_nothing_is_created_now() {
    let db = seeded().await;
    with_settings(&db, Row { revision: 1, ..ON }).await;
    with_run(&db, 8001, 1, "running", Some("token")).await;
    let (recorder, jobs) = jobs();
    let change = service::set_apply_to_all(&db, Some(&jobs), request(true, None))
        .await
        .expect("changed");
    assert_eq!((change.is_changed, change.run), (true, None));
    assert_eq!(runs(&db).await.len(), 1);
    assert_eq!(runs(&db).await[0].2, "cancelling");
    assert_eq!(settings_row(&db).await.map(|s| (s.0, s.1)), Some((true, 2)));
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn a_failed_first_enqueue_still_returns_the_run() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    let (recorder, jobs) = jobs();
    recorder.fail_next("redis down");
    let change = service::set_apply_to_all(&db, Some(&jobs), request(true, None))
        .await
        .expect("changed");
    assert!(change.run.is_some());
}

#[tokio::test]
async fn a_disconnected_page_is_refused_and_nothing_is_written() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    disconnect(&db).await;
    let error = service::set_apply_to_all(&db, None, request(true, None))
        .await
        .expect_err("refused");
    assert_eq!(code(&error), codes::PAGE_NOT_CONNECTED);
    assert_eq!(settings_row(&db).await.map(|s| s.1), Some(0));
    assert!(runs(&db).await.is_empty());
}

#[tokio::test]
async fn a_foreign_or_unsupported_inbox_is_refused_before_any_lock() {
    let db = seeded().await;
    for inbox in [FOREIGN, WHATSAPP] {
        let error = service::set_apply_to_all(
            &db,
            None,
            SetApplyToAllInput {
                inbox_id: inbox,
                ..request(true, None)
            },
        )
        .await
        .expect_err("not found");
        assert_eq!(code(&error), "notFound");
    }
}

#[tokio::test]
async fn reconcile_creates_the_run_the_latest_revision_is_waiting_for() {
    let db = seeded().await;
    with_settings(&db, Row { revision: 4, ..ON }).await;
    let (recorder, jobs) = jobs();
    service::reconcile(&db, Some(&jobs), WS, INBOX)
        .await
        .expect("reconciled");
    assert_eq!(
        runs(&db).await,
        vec![(4, "disable".into(), "pending".into(), Some("bye".into()))]
    );
    assert_eq!(recorder.calls().len(), 1);
}

#[tokio::test]
async fn reconcile_does_nothing_at_revision_0_or_when_the_revision_already_has_a_run() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    service::reconcile(&db, None, WS, INBOX)
        .await
        .expect("noop");
    assert!(runs(&db).await.is_empty());

    let db = seeded().await;
    with_settings(&db, Row { revision: 2, ..ON }).await;
    with_run(&db, 8001, 2, "failed", None).await;
    let (recorder, jobs) = jobs();
    service::reconcile(&db, Some(&jobs), WS, INBOX)
        .await
        .expect("noop");
    assert_eq!(runs(&db).await.len(), 1);
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn reconcile_an_on_waits_while_the_pages_automation_is_not_running() {
    let db = seeded().await;
    with_settings(
        &db,
        Row {
            enabled: false,
            apply: true,
            revision: 2,
            ..ON
        },
    )
    .await;
    service::reconcile(&db, None, WS, INBOX)
        .await
        .expect("noop");
    assert!(runs(&db).await.is_empty());
}

#[tokio::test]
async fn reconcile_a_disconnected_page_gets_no_run() {
    let db = seeded().await;
    with_settings(&db, Row { revision: 2, ..ON }).await;
    disconnect(&db).await;
    service::reconcile(&db, None, WS, INBOX)
        .await
        .expect("noop");
    assert!(runs(&db).await.is_empty());
}

#[tokio::test]
async fn reconcile_a_page_with_no_settings_row_has_nothing_to_reconcile() {
    let db = seeded().await;
    service::reconcile(&db, None, WS, INBOX)
        .await
        .expect("noop");
    assert_eq!(settings_row(&db).await, None);
}

#[tokio::test]
async fn two_reconcilers_cannot_both_create_the_second_finds_the_unique_index_taken() {
    let db = seeded().await;
    with_settings(&db, Row { revision: 2, ..ON }).await;
    let (recorder, jobs) = jobs();
    service::reconcile(&db, Some(&jobs), WS, INBOX)
        .await
        .expect("first");
    service::reconcile(&db, Some(&jobs), WS, INBOX)
        .await
        .expect("second");
    assert_eq!(recorder.calls().len(), 1);
}

#[tokio::test]
async fn retry_starts_the_same_desired_state_again_under_a_new_revision() {
    for status in ["failed", "cancelled"] {
        let db = seeded().await;
        with_settings(
            &db,
            Row {
                apply: true,
                revision: 2,
                ..ON
            },
        )
        .await;
        with_run(&db, 8001, 2, status, None).await;
        let change = service::retry(&db, None, WS, INBOX, Some(USER))
            .await
            .expect("retried");
        assert!(change.is_changed);
        assert_eq!(change.run.map(|r| r.revision), Some(3));
        assert_eq!(
            settings_row(&db).await.map(|s| (s.0, s.1, s.3)),
            Some((true, 3, Some(USER)))
        );
    }
}

#[tokio::test]
async fn retry_refuses_when_the_latest_run_is_live_completed_or_missing() {
    for latest in [Some("running"), Some("completed"), None] {
        let db = seeded().await;
        with_settings(&db, Row { revision: 2, ..ON }).await;
        if let Some(status) = latest {
            with_run(&db, 8001, 2, status, None).await;
        }
        let error = service::retry(&db, None, WS, INBOX, Some(USER))
            .await
            .expect_err("refused");
        assert_eq!(code(&error), codes::NOTHING_TO_RETRY);
        assert_eq!(settings_row(&db).await.map(|s| s.1), Some(2));
    }
}

#[tokio::test]
async fn retry_an_on_is_not_retried_while_the_automation_is_not_running() {
    let db = seeded().await;
    with_settings(
        &db,
        Row {
            enabled: false,
            apply: true,
            revision: 2,
            ..ON
        },
    )
    .await;
    with_run(&db, 8001, 2, "failed", None).await;
    let error = service::retry(&db, None, WS, INBOX, Some(USER))
        .await
        .expect_err("refused");
    assert_eq!(code(&error), codes::AUTOMATION_NOT_ACTIVE);
    assert_eq!(settings_row(&db).await.map(|s| s.1), Some(2));
}

#[tokio::test]
async fn retry_a_disconnected_page_is_not_retried() {
    let db = seeded().await;
    with_settings(
        &db,
        Row {
            apply: true,
            revision: 2,
            ..ON
        },
    )
    .await;
    with_run(&db, 8001, 2, "failed", None).await;
    disconnect(&db).await;
    let error = service::retry(&db, None, WS, INBOX, Some(USER))
        .await
        .expect_err("refused");
    assert_eq!(code(&error), codes::PAGE_NOT_CONNECTED);
}

#[tokio::test]
async fn retry_a_page_with_no_settings_row_has_nothing_to_retry() {
    let db = seeded().await;
    let error = service::retry(&db, None, WS, INBOX, Some(USER))
        .await
        .expect_err("refused");
    assert_eq!(code(&error), codes::NOTHING_TO_RETRY);
}

#[tokio::test]
async fn cancels_the_pages_live_run() {
    let db = seeded().await;
    with_settings(&db, Row { revision: 1, ..ON }).await;
    with_run(&db, 8001, 1, "pending", None).await;
    service::cancel_live_for_inbox(&db, WS, INBOX, None)
        .await
        .expect("cancelled");
    assert_eq!(runs(&db).await[0].2, "cancelled");
}

#[tokio::test]
async fn can_limit_the_stop_to_one_action_so_switching_the_automation_off_keeps_a_disable_running()
{
    let db = seeded().await;
    with_settings(&db, Row { revision: 1, ..ON }).await;
    setup(&db, |tx| {
        insert_run(tx, 8001, 1, "running", Some("token"))?;
        tx.execute(r#"UPDATE "AIHandoverBulkRun" SET "action" = 'disable'"#, [])?;
        Ok(())
    })
    .await;
    service::cancel_live_for_inbox(&db, WS, INBOX, Some("enable"))
        .await
        .expect("noop");
    assert_eq!(runs(&db).await[0].2, "running");
    service::cancel_live_for_inbox(&db, WS, INBOX, None)
        .await
        .expect("cancelling");
    assert_eq!(runs(&db).await[0].2, "cancelling");
}

#[tokio::test]
async fn a_page_that_never_had_settings_has_no_run_to_cancel() {
    let db = seeded().await;
    with_run(&db, 8001, 1, "pending", None).await;
    service::cancel_live_for_inbox(&db, WS, INBOX, None)
        .await
        .expect("noop");
    assert_eq!(runs(&db).await[0].2, "pending");
}

#[tokio::test]
async fn reports_the_desired_state_and_the_run_serving_its_latest_revision() {
    let db = seeded().await;
    with_settings(
        &db,
        Row {
            apply: true,
            revision: 3,
            ..ON
        },
    )
    .await;
    with_run(&db, 8003, 3, "running", None).await;
    let status = read(&db, |c| service::find_status_in(c, WS, INBOX)).await;
    assert_eq!(
        (
            status.apply_to_all_customers,
            status.revision,
            status.run.map(|r| r.id)
        ),
        (true, 3, Some(8003))
    );
}

#[tokio::test]
async fn a_page_with_no_change_yet_is_off_at_revision_0_with_no_run() {
    let db = seeded().await;
    let status = read(&db, |c| service::find_status_in(c, WS, INBOX)).await;
    assert_eq!(
        (status.apply_to_all_customers, status.revision, status.run),
        (false, 0, None)
    );
}

#[tokio::test]
async fn a_continuation_and_a_sweeper_retry_get_distinct_job_ids() {
    let recorder = RecordingEnqueuer::new();
    service::enqueue_chunk(recorder.as_ref(), 11, WS, 3, None)
        .await
        .expect("3");
    service::enqueue_chunk(recorder.as_ref(), 11, WS, 4, None)
        .await
        .expect("4");
    let ids: Vec<_> = recorder
        .calls()
        .into_iter()
        .map(|c| c.opts.job_id.expect("id"))
        .collect();
    assert_eq!(
        ids,
        ["ai-handover-bulk-11-chunk-3", "ai-handover-bulk-11-chunk-4"]
    );
    assert!(ids.iter().all(|id| !id.contains(':')));
}

#[tokio::test]
async fn a_paused_runs_continuation_is_delayed_by_the_pause_an_ordinary_one_is_not() {
    let recorder = RecordingEnqueuer::new();
    service::enqueue_chunk(recorder.as_ref(), 11, WS, 0, Some(120_000))
        .await
        .expect("paused");
    service::enqueue_chunk(recorder.as_ref(), 11, WS, 0, None)
        .await
        .expect("plain");
    let calls = recorder.calls();
    assert_eq!(calls[0].opts.delay_ms, Some(120_000));
    assert_eq!(calls[1].opts.delay_ms, None);
    // `attempts: 1, removeOnComplete: true, removeOnFail: { count: 100 }`.
    assert_eq!(calls[1].opts.attempts, Some(1));
    assert_eq!(calls[1].opts.remove_on_complete, Some(Retention::Remove));
    assert_eq!(
        calls[1].opts.remove_on_fail,
        Some(Retention::KeepCount(100))
    );
}

#[test]
fn a_disable_reaches_meta_s_7_day_window_and_an_enable_the_last_30_days() {
    // Adapted from the engine surface tests: the windows of `eligibilityOf`.
    use chatbotx_core::utils::channel::ThreadControlChannel;
    let now = at("2026-10-08T00:00:00Z");
    let disable = service::eligibility_of(
        AiHandoverBulkAction::Disable,
        now,
        ThreadControlChannel::Messenger,
        now,
    );
    assert_eq!(disable.last_incoming_since, at("2026-10-01T00:00:00Z"));
    let enable = service::eligibility_of(
        AiHandoverBulkAction::Enable,
        now,
        ThreadControlChannel::Messenger,
        now,
    );
    assert_eq!(enable.last_incoming_since, now - 30 * 86_400_000);
}

#[tokio::test]
async fn records_the_change_without_a_requesting_user() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    service::set_apply_to_all(
        &db,
        None,
        SetApplyToAllInput {
            user_id: None,
            ..request(true, None)
        },
    )
    .await
    .expect("changed");
    assert_eq!(settings_row(&db).await.map(|s| s.3), Some(None));
}

#[tokio::test]
async fn retry_works_without_a_user_too() {
    let db = seeded().await;
    with_settings(&db, Row { revision: 2, ..ON }).await;
    with_run(&db, 8001, 2, "failed", None).await;
    service::retry(&db, None, WS, INBOX, None)
        .await
        .expect("retried");
    assert_eq!(settings_row(&db).await.map(|s| s.3), Some(None));
}

async fn preview(
    db: &common::TestDb,
    apply: bool,
    message: Option<&str>,
) -> chatbotx_business::errors::Result<service::ApplyToAllPreview> {
    service::preview_apply_to_all(db, WS, INBOX, apply, message.map(str::to_owned)).await
}

#[tokio::test]
async fn counts_the_threads_an_on_would_hand_to_the_ai_writing_nothing() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    with_recent_threads(&db, 3).await;
    let p = preview(&db, true, None).await.expect("preview");
    assert_eq!((p.is_changed, p.eligible_count), (true, 3));
    assert_eq!(settings_row(&db).await.map(|s| s.1), Some(0));
    assert!(runs(&db).await.is_empty());
}

#[tokio::test]
async fn an_off_counts_the_threads_the_ai_holds() {
    let db = seeded().await;
    with_settings(&db, APPLIED).await;
    // Recently active but never handed to the AI: not held, so not counted.
    with_recent_threads(&db, 2).await;
    let p = preview(&db, false, Some("back to a human"))
        .await
        .expect("preview");
    assert_eq!((p.is_changed, p.eligible_count), (true, 0));
}

#[tokio::test]
async fn the_state_the_page_is_already_in_is_a_no_op_without_counting() {
    let db = seeded().await;
    with_settings(&db, APPLIED).await;
    let p = preview(&db, true, None).await.expect("preview");
    assert_eq!((p.is_changed, p.eligible_count), (false, 0));
}

#[tokio::test]
async fn preview_reports_the_same_refusals_as_the_real_call() {
    let db = seeded().await;
    with_settings(
        &db,
        Row {
            enabled: false,
            ..ON
        },
    )
    .await;
    assert_eq!(
        code(&preview(&db, true, None).await.expect_err("off")),
        codes::AUTOMATION_NOT_ACTIVE
    );
    // An ON for a Page with no saved settings needs the automation too.
    let db = seeded().await;
    assert_eq!(
        code(&preview(&db, true, None).await.expect_err("none")),
        codes::AUTOMATION_NOT_ACTIVE
    );
    // An OFF still needs its message.
    let db = seeded().await;
    with_settings(&db, APPLIED).await;
    assert_eq!(
        code(&preview(&db, false, Some("  ")).await.expect_err("msg")),
        codes::MESSAGE_REQUIRED
    );
    // A disconnected Page is refused.
    let db = seeded().await;
    with_settings(&db, ON).await;
    disconnect(&db).await;
    assert_eq!(
        code(&preview(&db, true, None).await.expect_err("page")),
        codes::PAGE_NOT_CONNECTED
    );
}

fn confirmed(max: i64) -> SetApplyToAllInput {
    SetApplyToAllInput {
        confirm_max_eligible: Some(max),
        ..request(true, None)
    }
}

#[tokio::test]
async fn refuses_writing_nothing_when_more_threads_are_eligible_than_the_caller_confirmed() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    with_recent_threads(&db, 3).await;
    let error = service::set_apply_to_all(&db, None, confirmed(2))
        .await
        .expect_err("exceeded");
    assert_eq!(code(&error), codes::CONFIRM_COUNT_EXCEEDED);
    assert_eq!(
        error.message(),
        "3 threads are eligible, more than the 2 confirmed"
    );
    assert_eq!(settings_row(&db).await.map(|s| s.1), Some(0));
    assert!(runs(&db).await.is_empty());
}

#[tokio::test]
async fn proceeds_when_the_eligible_count_is_within_the_confirmed_maximum() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    with_recent_threads(&db, 3).await;
    assert!(
        service::set_apply_to_all(&db, None, confirmed(3))
            .await
            .expect("ok")
            .is_changed
    );
}

#[tokio::test]
async fn refuses_a_confirmed_change_whose_run_cannot_start_now() {
    let db = seeded().await;
    with_settings(&db, Row { revision: 1, ..ON }).await;
    with_run(&db, 8001, 1, "running", Some("token")).await;
    let (recorder, jobs) = jobs();
    let error = service::set_apply_to_all(&db, Some(&jobs), confirmed(10))
        .await
        .expect_err("not startable");
    assert_eq!(code(&error), codes::RUN_NOT_STARTABLE);
    assert!(recorder.calls().is_empty());
    // The change is rolled back, the older run's cancel sticks.
    assert_eq!(
        settings_row(&db).await.map(|s| (s.0, s.1)),
        Some((false, 1))
    );
    assert_eq!(runs(&db).await[0].2, "cancelling");
}

#[tokio::test]
async fn an_unconfirmed_change_may_still_wait_for_the_previous_run() {
    let db = seeded().await;
    with_settings(&db, Row { revision: 1, ..ON }).await;
    with_run(&db, 8001, 1, "running", Some("token")).await;
    let change = service::set_apply_to_all(&db, None, request(true, None))
        .await
        .expect("waits");
    assert_eq!((change.is_changed, change.run), (true, None));
}

#[tokio::test]
async fn is_not_consulted_when_the_caller_sends_no_confirmation() {
    // Adapted: more threads than any count, and the change still goes through.
    let db = seeded().await;
    with_settings(&db, ON).await;
    with_recent_threads(&db, 3).await;
    assert!(
        service::set_apply_to_all(&db, None, request(true, None))
            .await
            .expect("ok")
            .run
            .is_some()
    );
}

#[tokio::test]
async fn saving_the_automation_off_also_stops_the_pages_running_enable() {
    let db = seeded().await;
    with_settings(
        &db,
        Row {
            apply: true,
            revision: 1,
            ..ON
        },
    )
    .await;
    with_run(&db, 8001, 1, "pending", None).await;
    let saved = service::save_settings(
        &db,
        SaveSettingsInput {
            enabled: false,
            ..save_input()
        },
    )
    .await
    .expect("saved");
    assert!(!saved.enabled);
    assert_eq!(runs(&db).await[0].2, "cancelled");
}

#[tokio::test]
async fn saving_it_on_leaves_the_running_runs_alone() {
    let db = seeded().await;
    with_settings(
        &db,
        Row {
            apply: true,
            revision: 1,
            ..ON
        },
    )
    .await;
    with_run(&db, 8001, 1, "pending", None).await;
    service::save_settings(&db, save_input())
        .await
        .expect("saved");
    assert_eq!(runs(&db).await[0].2, "pending");
}

#[tokio::test]
async fn merges_the_sent_fields_onto_a_fresh_read() {
    let db = seeded().await;
    service::save_settings(
        &db,
        SaveSettingsInput {
            goto_flow_id: Some(FLOW),
            return_message: Some("Back".into()),
            pause_bot_waiting_for_staff: true,
            ..save_input()
        },
    )
    .await
    .expect("saved");
    let row = service::patch_settings(
        &db,
        WS,
        INBOX,
        SettingsPatch {
            pause_bot_waiting_for_staff: Some(false),
            ..SettingsPatch::default()
        },
    )
    .await
    .expect("patched");
    assert_eq!(
        (
            row.enabled,
            row.goto_flow_id,
            row.return_message.as_deref(),
            row.pause_bot_waiting_for_staff
        ),
        (true, Some(FLOW), Some("Back"), false)
    );
}

#[tokio::test]
async fn a_page_without_saved_settings_starts_from_everything_off() {
    let db = seeded().await;
    let row = service::patch_settings(
        &db,
        WS,
        INBOX,
        SettingsPatch {
            enabled: Some(true),
            ..SettingsPatch::default()
        },
    )
    .await
    .expect("patched");
    assert_eq!(
        (
            row.enabled,
            row.schedule_enabled,
            row.time_ranges.0.clone(),
            row.goto_flow_id,
            row.return_message,
            row.pause_bot_waiting_for_staff
        ),
        (true, false, json!([]), None, None, false)
    );
}

#[tokio::test]
async fn two_partial_updates_sent_at_once_both_survive() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    let (a, b) = tokio::join!(
        service::patch_settings(
            &db,
            WS,
            INBOX,
            SettingsPatch {
                enabled: Some(false),
                ..SettingsPatch::default()
            }
        ),
        service::patch_settings(
            &db,
            WS,
            INBOX,
            SettingsPatch {
                return_message: Some(Some("Later".into())),
                ..SettingsPatch::default()
            }
        ),
    );
    a.expect("a");
    b.expect("b");
    let row = read(&db, |c| service::find_in(c, WS, INBOX))
        .await
        .expect("row");
    assert_eq!(
        (row.enabled, row.return_message.as_deref()),
        (false, Some("Later"))
    );
}

#[tokio::test]
async fn a_partial_update_sent_during_a_full_save_merges_onto_the_save() {
    let db = seeded().await;
    with_settings(&db, ON).await;
    setup(&db, |tx| common::flow(tx, 4008, WS).map(|_| ())).await;
    let (a, b) = tokio::join!(
        service::save_settings(
            &db,
            SaveSettingsInput {
                goto_flow_id: Some(4008),
                ..save_input()
            }
        ),
        service::patch_settings(
            &db,
            WS,
            INBOX,
            SettingsPatch {
                enabled: Some(false),
                ..SettingsPatch::default()
            }
        ),
    );
    a.expect("save");
    b.expect("patch");
    let row = read(&db, |c| service::find_in(c, WS, INBOX))
        .await
        .expect("row");
    assert_eq!((row.goto_flow_id, row.enabled), (Some(4008), false));
}
