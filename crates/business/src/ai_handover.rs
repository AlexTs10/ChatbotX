//! Meta Business AI hand-over: `ai-handover-settings/{policy,service}.ts`
//! (the Page's settings and whether the automation runs now) and the
//! builder-facing half of `ai-handover-bulk-run/service.ts` (the
//! "apply to all customers" switch, retry, reconcile, status, history and the
//! chunk dispatch).
//!
//! No settings cache (P3.6-D1 rule), no template lock / `assertReferencable`.
//! The original's per-Page Redis lock and `FOR UPDATE` row lock are the one
//! SQLite writer: every "read, decide, write" runs in one write transaction.
//! The engine / sweeper surface (`claim`, `recordProgress`, `pickDue`, ...)
//! is the worker's (unit Q.W2); it calls the repositories directly.

use std::sync::Arc;

use chatbotx_core::js;
use chatbotx_core::utils::channel::{
    AiHandoverChannelPolicy, ThreadControlChannel, ai_handover_channel_policy,
    parse_ai_handover_channel,
};
use chatbotx_db::Db;
use chatbotx_db::repo::ai_handover_bulk_run::{
    self as runs_repo, AI_HANDOVER_BULK_LIVE_STATUSES, CreateAiHandoverBulkRunInput, HistoryRow,
};
use chatbotx_db::repo::ai_handover_settings::{
    self as settings_repo, SetApplyToAllInput as SetRowInput, UpsertAiHandoverSettingsInput,
};
use chatbotx_db::repo::contact_inbox;
use chatbotx_db::repo::models::{AiHandoverBulkRun, AiHandoverSettings};
use chatbotx_db::repo::queries::ai_handover_bulk_eligibility::{
    AiHandoverBulkAction, BulkEligibilityInput,
};
use chatbotx_db::repo::utils::PaginationInput;
use chatbotx_db::repo::{Json, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _};
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName, Retention};
use chrono::Timelike as _;
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `AI_HANDOVER_DAY_END_HOUR`.
pub const DAY_END_HOUR: u32 = 24;
/// `AI_HANDOVER_BULK_MAX_ATTEMPTS`.
pub const BULK_MAX_ATTEMPTS: i64 = 5;
const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

/// `AI_HANDOVER_BULK_ERROR_CODES`.
pub mod codes {
    pub const AUTOMATION_NOT_ACTIVE: &str = "aiHandoverBulkAutomationNotActive";
    pub const MESSAGE_REQUIRED: &str = "aiHandoverBulkMessageRequired";
    pub const MESSAGE_TOO_LONG: &str = "aiHandoverBulkMessageTooLong";
    pub const PAGE_NOT_CONNECTED: &str = "aiHandoverBulkPageNotConnected";
    pub const NOTHING_TO_RETRY: &str = "aiHandoverBulkNothingToRetry";
    pub const CONFIRM_COUNT_EXCEEDED: &str = "aiHandoverBulkConfirmCountExceeded";
    pub const RUN_NOT_STARTABLE: &str = "aiHandoverBulkRunNotStartable";
}

fn bulk_error(code: &str, message: impl Into<String>) -> BusinessError {
    BusinessError::exception(message, code, 422)
}

// ---------------------------------------------------------------- policy

/// `AiHandoverTimeRange`: whole hours of the workspace timezone, end-exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRange {
    pub from: u32,
    pub to: u32,
}

/// `aiHandoverTimeRangesSchema.safeParse`: at most 24 ranges of integer
/// hours 0..=24 with `from != to` (unknown keys are stripped). `None` = invalid.
pub fn parse_time_ranges(value: &Value) -> Option<Vec<TimeRange>> {
    let items = value.as_array()?;
    if items.len() > DAY_END_HOUR as usize {
        return None;
    }
    items
        .iter()
        .map(|item| {
            let hour = |key: &str| {
                item.get(key)?
                    .as_f64()
                    .filter(|n| n.fract() == 0.0 && (0.0..=f64::from(DAY_END_HOUR)).contains(n))
                    .map(|n| n as u32)
            };
            let (from, to) = (hour("from")?, hour("to")?);
            (from != to).then_some(TimeRange { from, to })
        })
        .collect()
}

/// The ranges as stored (`[{ from, to }]`).
pub fn time_ranges_json(ranges: &[TimeRange]) -> Value {
    Value::Array(
        ranges
            .iter()
            .map(|r| json!({ "from": r.from, "to": r.to }))
            .collect(),
    )
}

/// `formatInTimeZone(now, timeZone, "H")`; an unknown zone reads as UTC.
fn hour_in_time_zone(now_ms: i64, time_zone: &str) -> u32 {
    let tz = time_zone.parse::<chrono_tz::Tz>().unwrap_or(chrono_tz::UTC);
    chrono::DateTime::from_timestamp_millis(now_ms)
        .unwrap_or_default()
        .with_timezone(&tz)
        .hour()
}

/// `isWithinHourRanges`: ranges OR-ed, `from > to` wraps midnight, equal
/// bounds never match.
pub fn is_within_hour_ranges(ranges: &[TimeRange], time_zone: &str, now_ms: i64) -> bool {
    let hour = hour_in_time_zone(now_ms, time_zone);
    ranges.iter().any(|&TimeRange { from, to }| {
        if from == to {
            false
        } else if from < to {
            hour >= from && hour < to
        } else {
            hour >= from || hour < to
        }
    })
}

/// `isAiHandoverActive`: switched on and, with a schedule, inside a window.
pub fn is_ai_handover_active(
    enabled: bool,
    schedule_enabled: bool,
    ranges: &[TimeRange],
    time_zone: &str,
    now_ms: i64,
) -> bool {
    enabled && (!schedule_enabled || is_within_hour_ranges(ranges, time_zone, now_ms))
}

// ---------------------------------------------------------------- settings

/// `AiHandoverInbox`: a Page of the workspace whose channel has an AI hand-over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoverInbox {
    pub id: i64,
    pub status: String,
    pub channel: ThreadControlChannel,
}

impl HandoverInbox {
    fn policy(&self) -> AiHandoverChannelPolicy {
        ai_handover_channel_policy(self.channel).expect("parsed AI hand-over channel")
    }
}

fn repo_err(e: impl Into<chatbotx_db::repo::Error>) -> BusinessError {
    BusinessError::Repo(e.into())
}

fn inbox_row(conn: &Connection, ws: i64, inbox: i64) -> Result<Option<(String, Option<String>)>> {
    conn.query_row(
        r#"SELECT "status", "channel" FROM "Inbox" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        [inbox, ws],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map_err(repo_err)
}

/// `requireInbox`: `Inbox not found` for a foreign id or a channel without an
/// AI hand-over.
pub fn require_inbox(conn: &Connection, ws: i64, inbox: i64) -> Result<HandoverInbox> {
    inbox_row(conn, ws, inbox)?
        .and_then(|(status, channel)| {
            parse_ai_handover_channel(channel.as_deref()).map(|channel| HandoverInbox {
                id: inbox,
                status,
                channel,
            })
        })
        .ok_or_else(|| BusinessError::not_found("Inbox not found"))
}

/// `find`: the Page's settings (`None` = never saved).
pub fn find_in(conn: &Connection, ws: i64, inbox: i64) -> Result<Option<AiHandoverSettings>> {
    settings_repo::find_by_inbox(conn, ws, inbox).map_err(repo_err)
}

fn is_page_connected(conn: &Connection, ws: i64, inbox: i64) -> Result<bool> {
    Ok(inbox_row(conn, ws, inbox)?.is_some_and(|(status, _)| status == "connected"))
}

/// `isActiveNow`: the workspace timezone is read only for a schedule.
pub fn is_active_now(conn: &Connection, s: &AiHandoverSettings, now: i64) -> Result<bool> {
    let ranges = parse_time_ranges(&s.time_ranges.0).unwrap_or_default();
    let time_zone = if s.schedule_enabled {
        conn.query_row(
            r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
            [s.workspace_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(repo_err)?
        .flatten()
        .unwrap_or_else(|| "UTC".to_owned())
    } else {
        "UTC".to_owned()
    };
    Ok(is_ai_handover_active(
        s.enabled,
        s.schedule_enabled,
        &ranges,
        &time_zone,
        now,
    ))
}

/// `findActive`: the settings when the automation applies now and the Page
/// is still connected.
pub fn find_active_in(
    conn: &Connection,
    ws: i64,
    inbox: i64,
    now: i64,
) -> Result<Option<AiHandoverSettings>> {
    let Some(settings) = find_in(conn, ws, inbox)? else {
        return Ok(None);
    };
    if !is_active_now(conn, &settings, now)? || !is_page_connected(conn, ws, inbox)? {
        return Ok(None);
    }
    Ok(Some(settings))
}

/// `findStopReason`: why a bulk run must stop (`None` = carry on).
pub fn find_stop_reason(
    conn: &Connection,
    ws: i64,
    inbox: i64,
    requires_automation: bool,
    now: i64,
) -> Result<Option<&'static str>> {
    if !is_page_connected(conn, ws, inbox)? {
        return Ok(Some("pageDisconnected"));
    }
    if requires_automation {
        let active = match find_in(conn, ws, inbox)? {
            Some(settings) => is_active_now(conn, &settings, now)?,
            None => false,
        };
        if !active {
            return Ok(Some("automationStopped"));
        }
    }
    Ok(None)
}

/// `isConfiguredAndInactive`: saved, but not running right now.
pub fn is_configured_and_inactive(
    conn: &Connection,
    ws: i64,
    inbox: i64,
    now: i64,
) -> Result<bool> {
    match find_in(conn, ws, inbox)? {
        Some(settings) => Ok(!is_active_now(conn, &settings, now)?),
        None => Ok(false),
    }
}

/// `SaveAiHandoverSettingsInput`.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveSettingsInput {
    pub workspace_id: i64,
    pub inbox_id: i64,
    pub enabled: bool,
    pub schedule_enabled: bool,
    pub time_ranges: Value,
    pub goto_flow_id: Option<i64>,
    pub return_message: Option<String>,
    pub pause_bot_waiting_for_staff: bool,
}

/// `aiHandoverSettingsService.save`: the Page, the ranges, the bounded return
/// message (trimmed, blank = null) and an active goto flow of the workspace.
pub fn save_in(conn: &Connection, input: &SaveSettingsInput) -> Result<AiHandoverSettings> {
    let (ws, inbox_id) = (input.workspace_id, input.inbox_id);
    let inbox = require_inbox(conn, ws, inbox_id)?;
    let ranges = parse_time_ranges(&input.time_ranges)
        .ok_or_else(|| BusinessError::validation("timeRanges", "Invalid time ranges"))?;
    if input.schedule_enabled && ranges.is_empty() {
        return Err(BusinessError::validation(
            "timeRanges",
            "Add at least one time range",
        ));
    }
    let return_message = input
        .return_message
        .as_deref()
        .map(js::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_owned);
    let max = inbox.policy().message_max_length;
    if return_message
        .as_deref()
        .is_some_and(|m| js::utf16_len(m) > max)
    {
        return Err(BusinessError::Exception {
            message: "Return message is too long".to_owned(),
            code: "validation".to_owned(),
            status: 422,
            field: Some("returnMessage".to_owned()),
            data: Some(json!({ "max": max })),
        });
    }
    if let Some(flow_id) = input.goto_flow_id {
        let active = conn
            .query_row(
                r#"SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1"#,
                [flow_id, ws],
                |_| Ok(()),
            )
            .optional()
            .map_err(repo_err)?;
        if active.is_none() {
            return Err(BusinessError::not_found("Flow not found"));
        }
    }
    settings_repo::upsert(
        conn,
        &UpsertAiHandoverSettingsInput {
            workspace_id: ws,
            inbox_id,
            channel: inbox.channel.as_str().to_owned(),
            enabled: input.enabled,
            schedule_enabled: input.schedule_enabled,
            time_ranges: Json(time_ranges_json(&ranges)),
            goto_flow_id: input.goto_flow_id,
            return_message,
            pause_bot_waiting_for_staff: input.pause_bot_waiting_for_staff,
        },
    )
    .map_err(repo_err)
}

/// `Partial<AiHandoverSettingsFields>`: `None` keeps the saved value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingsPatch {
    pub enabled: Option<bool>,
    pub schedule_enabled: Option<bool>,
    pub time_ranges: Option<Value>,
    pub goto_flow_id: Option<Option<i64>>,
    pub return_message: Option<Option<String>>,
    pub pause_bot_waiting_for_staff: Option<bool>,
}

/// `aiHandoverBulkRunService.saveSettings`: saving the automation off also
/// stops the Page's running enable (logged, never fatal).
pub async fn save_settings(db: &Db, input: SaveSettingsInput) -> Result<AiHandoverSettings> {
    let saved = tx::write(db, move |conn| save_in(conn, &input)).await?;
    after_save(db, &saved).await;
    Ok(saved)
}

/// `patchSettings`: the sent fields merged onto a fresh read (a Page that
/// never saved starts from everything off), in one write transaction.
pub async fn patch_settings(
    db: &Db,
    ws: i64,
    inbox: i64,
    changes: SettingsPatch,
) -> Result<AiHandoverSettings> {
    let saved = tx::write(db, move |conn| {
        require_inbox(conn, ws, inbox)?;
        let saved = find_in(conn, ws, inbox)?;
        let s = saved.as_ref();
        let input = SaveSettingsInput {
            workspace_id: ws,
            inbox_id: inbox,
            enabled: changes
                .enabled
                .unwrap_or_else(|| s.is_some_and(|s| s.enabled)),
            schedule_enabled: changes
                .schedule_enabled
                .unwrap_or_else(|| s.is_some_and(|s| s.schedule_enabled)),
            time_ranges: changes
                .time_ranges
                .unwrap_or_else(|| s.map_or(json!([]), |s| s.time_ranges.0.clone())),
            goto_flow_id: changes
                .goto_flow_id
                .unwrap_or_else(|| s.and_then(|s| s.goto_flow_id)),
            return_message: changes
                .return_message
                .unwrap_or_else(|| s.and_then(|s| s.return_message.clone())),
            pause_bot_waiting_for_staff: changes
                .pause_bot_waiting_for_staff
                .unwrap_or_else(|| s.is_some_and(|s| s.pause_bot_waiting_for_staff)),
        };
        save_in(conn, &input)
    })
    .await?;
    after_save(db, &saved).await;
    Ok(saved)
}

async fn after_save(db: &Db, saved: &AiHandoverSettings) {
    if saved.enabled {
        return;
    }
    if let Err(err) =
        cancel_live_for_inbox(db, saved.workspace_id, saved.inbox_id, Some("enable")).await
    {
        tracing::error!(
            inbox_id = saved.inbox_id,
            %err,
            "AI hand-over settings saved off: could not stop the Page's enable run"
        );
    }
}

// ---------------------------------------------------------------- bulk run

/// `SetApplyToAllInput`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetApplyToAllInput {
    pub workspace_id: i64,
    pub inbox_id: i64,
    /// `None` for a workspace-token caller.
    pub user_id: Option<i64>,
    pub apply_to_all_customers: bool,
    pub message: Option<String>,
    pub confirm_max_eligible: Option<i64>,
}

/// `ApplyToAllPreview`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyToAllPreview {
    pub is_changed: bool,
    pub eligible_count: i64,
}

/// `ApplyToAllChange`.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyToAllChange {
    pub is_changed: bool,
    pub run: Option<AiHandoverBulkRun>,
}

/// `ApplyToAllStatus`.
#[derive(Debug, Clone, PartialEq)]
pub struct ApplyToAllStatus {
    pub apply_to_all_customers: bool,
    pub revision: i64,
    pub run: Option<AiHandoverBulkRun>,
}

/// `eligibilityOf`: a disable reaches the takeover-message window, an
/// enable the recently active contacts.
pub fn eligibility_of(
    action: AiHandoverBulkAction,
    requested_at: i64,
    channel: ThreadControlChannel,
    now: i64,
) -> BulkEligibilityInput {
    let policy = ai_handover_channel_policy(channel).expect("AI hand-over channel");
    let window = match action {
        AiHandoverBulkAction::Disable => policy.takeover_message_window_ms,
        AiHandoverBulkAction::Enable => {
            i64::from(policy.hand_to_ai_active_within_days) * MS_PER_DAY
        }
    };
    BulkEligibilityInput {
        action,
        requested_at,
        now,
        last_incoming_since: now - window,
    }
}

fn action_of(apply: bool) -> AiHandoverBulkAction {
    if apply {
        AiHandoverBulkAction::Enable
    } else {
        AiHandoverBulkAction::Disable
    }
}

/// `validateMessage`: the text an OFF sends (required, bounded), `None` for an ON.
fn validate_message(
    apply: bool,
    message: Option<&str>,
    inbox: &HandoverInbox,
) -> Result<Option<String>> {
    if apply {
        return Ok(None);
    }
    let message = js::trim(message.unwrap_or(""));
    if message.is_empty() {
        return Err(bulk_error(codes::MESSAGE_REQUIRED, "A message is required"));
    }
    if js::utf16_len(message) > inbox.policy().message_max_length {
        return Err(bulk_error(
            codes::MESSAGE_TOO_LONG,
            "The message is too long",
        ));
    }
    Ok(Some(message.to_owned()))
}

fn count_eligible_for_change(
    conn: &Connection,
    inbox: &HandoverInbox,
    ws: i64,
    apply: bool,
    now: i64,
) -> Result<i64> {
    let eligibility = eligibility_of(action_of(apply), now, inbox.channel, now);
    contact_inbox::count_bulk_ai_eligible(conn, &eligibility, ws, inbox.id, None).map_err(repo_err)
}

fn not_connected() -> BusinessError {
    bulk_error(codes::PAGE_NOT_CONNECTED, "The Page is not connected")
}

fn automation_not_active() -> BusinessError {
    bulk_error(
        codes::AUTOMATION_NOT_ACTIVE,
        "Business AI automation must be enabled and running",
    )
}

/// `assertCanApply`.
fn assert_can_apply(
    conn: &Connection,
    settings: &AiHandoverSettings,
    inbox_status: &str,
    apply: bool,
    now: i64,
) -> Result<()> {
    if inbox_status != "connected" {
        return Err(not_connected());
    }
    if apply && !is_active_now(conn, settings, now)? {
        return Err(automation_not_active());
    }
    Ok(())
}

/// `createRunIfDue`: `None` when the revision already ran, a live run holds
/// the Page, the Page is disconnected, or an ON has no running automation.
fn create_run_if_due(
    conn: &Connection,
    settings: &AiHandoverSettings,
    inbox_status: &str,
    now: i64,
) -> Result<Option<AiHandoverBulkRun>> {
    let (ws, inbox, revision) = (
        settings.workspace_id,
        settings.inbox_id,
        settings.apply_to_all_revision,
    );
    if revision == 0 || inbox_status != "connected" {
        return Ok(None);
    }
    if runs_repo::find_by_revision(conn, ws, inbox, revision)
        .map_err(repo_err)?
        .is_some()
    {
        return Ok(None);
    }
    let enable = settings.apply_to_all_customers;
    if enable && !is_active_now(conn, settings, now)? {
        return Ok(None);
    }
    runs_repo::create_for_revision(
        conn,
        &CreateAiHandoverBulkRunInput {
            workspace_id: ws,
            inbox_id: inbox,
            channel: settings.channel.clone(),
            revision,
            action: if enable { "enable" } else { "disable" }.to_owned(),
            message: settings.apply_to_all_message.clone(),
            requested_by_user_id: settings.apply_to_all_requested_by_user_id,
            requested_at: now,
        },
    )
    .map_err(repo_err)
}

fn preview_in(
    conn: &Connection,
    ws: i64,
    inbox_id: i64,
    apply: bool,
    message: Option<&str>,
    now: i64,
) -> Result<ApplyToAllPreview> {
    let inbox = require_inbox(conn, ws, inbox_id)?;
    validate_message(apply, message, &inbox)?;
    let settings = find_in(conn, ws, inbox_id)?;
    if settings.as_ref().is_some_and(|s| s.apply_to_all_customers) == apply {
        return Ok(ApplyToAllPreview {
            is_changed: false,
            eligible_count: 0,
        });
    }
    if inbox.status != "connected" {
        return Err(not_connected());
    }
    if apply {
        let active = match &settings {
            Some(s) => is_active_now(conn, s, now)?,
            None => false,
        };
        if !active {
            return Err(automation_not_active());
        }
    }
    Ok(ApplyToAllPreview {
        is_changed: true,
        eligible_count: count_eligible_for_change(conn, &inbox, ws, apply, now)?,
    })
}

/// `previewApplyToAll`: what [`set_apply_to_all`] would do, writing nothing.
pub async fn preview_apply_to_all(
    db: &Db,
    ws: i64,
    inbox: i64,
    apply: bool,
    message: Option<String>,
) -> Result<ApplyToAllPreview> {
    tx::read(db, move |conn| {
        preview_in(conn, ws, inbox, apply, message.as_deref(), now_ms())
    })
    .await
}

fn confirm_exceeded(eligible: i64, max: i64) -> BusinessError {
    bulk_error(
        codes::CONFIRM_COUNT_EXCEEDED,
        format!("{eligible} threads are eligible, more than the {max} confirmed"),
    )
}

/// `setApplyToAll`: a request for the current state changes nothing; a real
/// change bumps the revision, cancels the older run and creates (and
/// dispatches) the new one when it is due.
pub async fn set_apply_to_all(
    db: &Db,
    jobs: Option<&Arc<dyn Enqueuer>>,
    input: SetApplyToAllInput,
) -> Result<ApplyToAllChange> {
    let (ws, inbox_id) = (input.workspace_id, input.inbox_id);
    let outcome = tx::write(db, move |conn| {
        let now = now_ms();
        let apply = input.apply_to_all_customers;
        let inbox = require_inbox(conn, ws, inbox_id)?;
        let message = validate_message(apply, input.message.as_deref(), &inbox)?;
        // ponytail: the original's pre-lock count and its re-check under the
        // lock are one check here (the single writer sees one state).
        if let Some(max) = input.confirm_max_eligible {
            let preview = preview_in(conn, ws, inbox_id, apply, input.message.as_deref(), now)?;
            if preview.is_changed && preview.eligible_count > max {
                return Err(confirm_exceeded(preview.eligible_count, max));
            }
        }
        let existing = settings_repo::lock_existing(conn, ws, inbox_id).map_err(repo_err)?;
        if existing
            .as_ref()
            .is_some_and(|e| e.settings.apply_to_all_customers)
            == apply
        {
            return Ok(ApplyToAllChange {
                is_changed: false,
                run: None,
            });
        }
        let locked = match existing {
            Some(locked) => locked,
            None => settings_repo::lock_for_apply_to_all(conn, ws, inbox_id, inbox.channel.as_str())
                .map_err(repo_err)?,
        };
        assert_can_apply(conn, &locked.settings, &locked.inbox_status, apply, now)?;
        let updated = settings_repo::set_apply_to_all(
            conn,
            &SetRowInput {
                workspace_id: ws,
                inbox_id,
                apply_to_all_customers: apply,
                apply_to_all_message: message,
                requested_by_user_id: input.user_id,
            },
        )
        .map_err(repo_err)?;
        runs_repo::cancel_live(conn, ws, inbox_id, None, now).map_err(repo_err)?;
        let run = create_run_if_due(conn, &updated, &locked.inbox_status, now)?;
        if input.confirm_max_eligible.is_some() && run.is_none() {
            return Err(bulk_error(
                codes::RUN_NOT_STARTABLE,
                "A previous run was still running on this Page and has been told to stop. The confirmed count would be stale by the time this change could start: try again shortly.",
            ));
        }
        Ok(ApplyToAllChange {
            is_changed: true,
            run,
        })
    })
    .await;
    let change = match outcome {
        Err(error) if error.code() == Some(codes::RUN_NOT_STARTABLE) => {
            // The refusal rolled the cancel back: make it stick.
            cancel_live_for_inbox(db, ws, inbox_id, None).await?;
            return Err(error);
        }
        other => other?,
    };
    after_change(jobs, change.run.as_ref()).await;
    Ok(change)
}

/// `retry`: the latest revision's run failed or was stopped: the same state
/// under a new revision, under the same rules as a change.
pub async fn retry(
    db: &Db,
    jobs: Option<&Arc<dyn Enqueuer>>,
    ws: i64,
    inbox_id: i64,
    user_id: Option<i64>,
) -> Result<ApplyToAllChange> {
    let run = tx::write(db, move |conn| {
        let now = now_ms();
        require_inbox(conn, ws, inbox_id)?;
        let locked = settings_repo::lock_existing(conn, ws, inbox_id).map_err(repo_err)?;
        let latest = match &locked {
            Some(l) => {
                runs_repo::find_by_revision(conn, ws, inbox_id, l.settings.apply_to_all_revision)
                    .map_err(repo_err)?
            }
            None => None,
        };
        let (Some(locked), Some(latest)) = (locked, latest) else {
            return Err(bulk_error(
                codes::NOTHING_TO_RETRY,
                "There is nothing to retry",
            ));
        };
        if latest.status == "completed"
            || AI_HANDOVER_BULK_LIVE_STATUSES.contains(&latest.status.as_str())
        {
            return Err(bulk_error(
                codes::NOTHING_TO_RETRY,
                "There is nothing to retry",
            ));
        }
        let s = &locked.settings;
        assert_can_apply(conn, s, &locked.inbox_status, s.apply_to_all_customers, now)?;
        let updated = settings_repo::set_apply_to_all(
            conn,
            &SetRowInput {
                workspace_id: ws,
                inbox_id,
                apply_to_all_customers: s.apply_to_all_customers,
                apply_to_all_message: s.apply_to_all_message.clone(),
                requested_by_user_id: user_id,
            },
        )
        .map_err(repo_err)?;
        create_run_if_due(conn, &updated, &locked.inbox_status, now)
    })
    .await?;
    after_change(jobs, run.as_ref()).await;
    Ok(ApplyToAllChange {
        is_changed: true,
        run,
    })
}

/// `reconcile`: creates the run the latest revision waits for, if due.
pub async fn reconcile(
    db: &Db,
    jobs: Option<&Arc<dyn Enqueuer>>,
    ws: i64,
    inbox_id: i64,
) -> Result<()> {
    let run = tx::write(db, move |conn| {
        match settings_repo::lock_existing(conn, ws, inbox_id).map_err(repo_err)? {
            Some(l) => create_run_if_due(conn, &l.settings, &l.inbox_status, now_ms()),
            None => Ok(None),
        }
    })
    .await?;
    after_change(jobs, run.as_ref()).await;
    Ok(())
}

/// `cancelLiveForInbox`: stops the Page's live run (of `action` only, when
/// given); a Page without settings has none.
pub async fn cancel_live_for_inbox(
    db: &Db,
    ws: i64,
    inbox_id: i64,
    action: Option<&'static str>,
) -> Result<()> {
    tx::write(db, move |conn| cancel_live_in(conn, ws, inbox_id, action)).await
}

/// [`cancel_live_for_inbox`] inside the caller's write transaction (the
/// inbox disconnect calls it after its own writes, `inboxService.disconnect`).
pub fn cancel_live_in(
    conn: &Connection,
    ws: i64,
    inbox_id: i64,
    action: Option<&str>,
) -> Result<()> {
    if settings_repo::lock_existing(conn, ws, inbox_id)
        .map_err(repo_err)?
        .is_some()
    {
        runs_repo::cancel_live(conn, ws, inbox_id, action, now_ms()).map_err(repo_err)?;
    }
    Ok(())
}

/// `findStatus`.
pub fn find_status_in(conn: &Connection, ws: i64, inbox_id: i64) -> Result<ApplyToAllStatus> {
    let settings = find_in(conn, ws, inbox_id)?;
    let revision = settings.as_ref().map_or(0, |s| s.apply_to_all_revision);
    let run = if revision > 0 {
        runs_repo::find_by_revision(conn, ws, inbox_id, revision).map_err(repo_err)?
    } else {
        None
    };
    Ok(ApplyToAllStatus {
        apply_to_all_customers: settings.is_some_and(|s| s.apply_to_all_customers),
        revision,
        run,
    })
}

/// `listHistory`: the Page's runs, newest first, with the requesting user.
pub fn list_history_in(
    conn: &Connection,
    ws: i64,
    inbox_id: i64,
    pagination: PaginationInput,
) -> Result<(Vec<HistoryRow>, i64)> {
    runs_repo::list_history(conn, ws, inbox_id, pagination).map_err(repo_err)
}

/// `afterChange`: dispatches the new run (best effort: the sweeper picks a
/// `pending` row up).
async fn after_change(jobs: Option<&Arc<dyn Enqueuer>>, run: Option<&AiHandoverBulkRun>) {
    let (Some(jobs), Some(run)) = (jobs, run) else {
        return;
    };
    if let Err(err) =
        enqueue_chunk(jobs.as_ref(), run.id, run.workspace_id, run.chunk_seq, None).await
    {
        tracing::error!(run_id = run.id, %err, "ai-handover bulk: enqueue failed");
    }
}

/// `buildAiHandoverBulkJobId`.
pub fn bulk_job_id(run_id: i64, chunk_seq: i64) -> String {
    format!("ai-handover-bulk-{run_id}-chunk-{chunk_seq}")
}

/// `enqueueChunk`: one `aiHandoverBulkToggle` job on `integration`.
pub async fn enqueue_chunk(
    jobs: &dyn Enqueuer,
    run_id: i64,
    workspace_id: i64,
    chunk_seq: i64,
    delay_ms: Option<u64>,
) -> std::result::Result<(), chatbotx_jobs::JobsError> {
    const NAME: &str = "aiHandoverBulkToggle";
    jobs.add(
        QueueName::Integration,
        NAME,
        json!({
            "type": NAME,
            "data": { "runId": run_id.to_string(), "workspaceId": workspace_id.to_string() },
        }),
        JobOptions {
            job_id: Some(bulk_job_id(run_id, chunk_seq)),
            attempts: Some(1),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::KeepCount(100)),
            delay_ms: delay_ms.filter(|d| *d > 0),
            ..JobOptions::default()
        },
    )
    .await
    .map(|_| ())
}
