//! Port of `broadcast/recurrence.service.ts` (schedule and read parts) and
//! `broadcast/recurrence-ledger.ts`.

use chatbotx_core::js::to_iso_string;
use chatbotx_core::utils::recurrence::{
    MonthDay, RECURRENCE_WEEK_STARTS_ON, RecurrenceEnd, RecurrenceRule, compute_next_occurrence,
    compute_next_sendable, list_occurrences_between, list_upcoming_occurrences,
    parse_recurrence_rule, to_local_slot,
};
use chatbotx_core::utils::timezone::canonicalize_timezone;
use chatbotx_db::repo::Json;
use chatbotx_db::repo::broadcast_recurrence::{
    self as repository, LedgerRowInput, RecurrencePatch,
};
use chatbotx_db::repo::models::BroadcastRecurrence;
use chatbotx_db::rusqlite::Connection;
use serde_json::{Value, json};

use super::plan_policy::{self, RestrictedContext};
use crate::errors::{BusinessError, Result};

/// `BROADCAST_PREVIEW_MAX_COUNT`.
pub const BROADCAST_PREVIEW_MAX_COUNT: usize = 10;
const MAX_LEDGER_BACKFILL_ROWS: usize = 500;
const MAX_BACKFILL_SLOTS_COUNTED: usize = 5000;

/// The rule as stored (`recurrenceRuleSchema` output).
pub fn rule_to_json(rule: &RecurrenceRule) -> Value {
    let mut out = json!({
        "frequency": rule.frequency.as_str(),
        "interval": rule.interval,
        "timeOfDay": rule.time_of_day,
        "timezone": rule.timezone,
        "startsOn": rule.starts_on,
        "end": match &rule.end {
            RecurrenceEnd::Never => json!({ "kind": "never" }),
            RecurrenceEnd::OnDate(date) => json!({ "kind": "onDate", "date": date }),
            RecurrenceEnd::AfterOccurrences(count) => {
                json!({ "kind": "afterOccurrences", "count": count })
            }
        },
    });
    if let Some(weekdays) = &rule.weekdays {
        out["weekdays"] = json!(weekdays);
    }
    if let Some(month_day) = &rule.month_day {
        out["monthDay"] = match month_day {
            MonthDay::Day(day) => json!(day),
            MonthDay::Last => json!("last"),
        };
    }
    if let Some(dates) = &rule.exclude_dates {
        out["excludeDates"] = json!(dates);
    }
    out
}

/// `recurrenceRuleSchema.parse` of a stored rule; `None` when it no longer parses.
pub fn stored_rule(rule: &Json) -> Option<RecurrenceRule> {
    parse_recurrence_rule(&rule.0).ok()
}

/// `normalizeRule`: validated, canonical timezone, distinct sorted skipped dates.
pub fn normalize_rule(rule: &Value) -> Result<RecurrenceRule> {
    let invalid = |issues: Vec<chatbotx_core::utils::recurrence::RuleIssue>| {
        let message = issues
            .first()
            .map_or_else(|| "Invalid recurrence".to_owned(), |i| i.message.clone());
        BusinessError::validation("recurrence", message)
    };
    let mut rule = parse_recurrence_rule(rule).map_err(invalid)?;
    rule.timezone = canonicalize_timezone(&rule.timezone);
    rule.exclude_dates = rule.exclude_dates.take().and_then(|mut dates| {
        dates.sort();
        dates.dedup();
        (!dates.is_empty()).then_some(dates)
    });
    // `recurrenceRuleSchema.parse` of the normalized rule.
    parse_recurrence_rule(&rule_to_json(&rule)).map_err(invalid)
}

/// `upsertRule`: stores the rule (creating the series row when missing), returns it.
pub fn upsert_rule(
    tx: &Connection,
    workspace_id: i64,
    broadcast_id: i64,
    rule: &Value,
) -> Result<BroadcastRecurrence> {
    let rule = Json(rule_to_json(&normalize_rule(rule)?));
    repository::insert_if_missing(tx, workspace_id, broadcast_id, &rule)?;
    let Some(mut locked) = repository::lock_for_write(tx, workspace_id, &[broadcast_id])?
        .into_iter()
        .next()
    else {
        return Err(BusinessError::not_found("Recurrence not found"));
    };
    let patch = RecurrencePatch {
        rule: Some(rule.clone()),
        ..RecurrencePatch::default()
    };
    repository::update(tx, locked.id, &patch, None)?;
    locked.rule = rule;
    Ok(locked)
}

/// The armed schedule [`arm`] hands back to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Armed {
    pub next_run_at: i64,
    /// `sendRateOverride.sendRatePerMinute`.
    pub send_rate_override: Option<i64>,
}

fn no_upcoming_run() -> BusinessError {
    BusinessError::validation("recurrence", "The schedule has no upcoming run")
}

/// `arm`: accounts the slots missed while off, computes and stores the next run.
pub fn arm(
    tx: &Connection,
    recurrence: &BroadcastRecurrence,
    send_rate_per_minute: Option<i64>,
    restriction: Option<&RestrictedContext>,
    now: i64,
) -> Result<Armed> {
    let rule = stored_rule(&recurrence.rule).ok_or_else(no_upcoming_run)?;
    let next_run_at = compute_next_occurrence(&rule, now, recurrence.occurrence_count)
        .ok()
        .flatten()
        .ok_or_else(no_upcoming_run)?;
    let send_rate_override = match restriction {
        Some(restriction) => {
            plan_policy::assert_send_rate_allowed(restriction, send_rate_per_minute)?;
            Some(plan_policy::resolve_send_rate_override(
                restriction,
                send_rate_per_minute,
            ))
        }
        None => None,
    };
    let reason = if recurrence.paused_at.is_some() {
        "paused"
    } else {
        "draft"
    };
    account_slots(tx, recurrence, &rule, now, "skipped", reason, now)?;
    repository::update(
        tx,
        recurrence.id,
        &RecurrencePatch {
            next_run_at: Some(Some(next_run_at)),
            accounted_through_at: Some(Some(now)),
            paused_at: Some(None),
            ended_at: Some(None),
            ..RecurrencePatch::default()
        },
        None,
    )?;
    Ok(Armed {
        next_run_at,
        send_rate_override,
    })
}

/// `DisarmReason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisarmReason {
    Paused,
    Draft,
    Deleted,
}

/// `disarmSeries`: accounts a slot already due, then stops the series.
pub fn disarm(
    tx: &Connection,
    recurrence: &BroadcastRecurrence,
    reason: DisarmReason,
    now: i64,
) -> Result<()> {
    let armed = recurrence.next_run_at.is_some();
    let account_as = if armed {
        match reason {
            DisarmReason::Paused => Some("paused"),
            DisarmReason::Draft => Some("draft"),
            DisarmReason::Deleted => None,
        }
    } else if recurrence.paused_at.is_some() && reason == DisarmReason::Draft {
        Some("paused")
    } else {
        None
    };
    if let (Some(account_as), Some(rule)) = (account_as, stored_rule(&recurrence.rule)) {
        account_slots(tx, recurrence, &rule, now, "skipped", account_as, now)?;
    }
    let mut patch = RecurrencePatch {
        next_run_at: Some(None),
        ..RecurrencePatch::default()
    };
    match reason {
        DisarmReason::Paused => patch.paused_at = Some(Some(now)),
        DisarmReason::Draft => patch.paused_at = Some(None),
        DisarmReason::Deleted => {}
    }
    if account_as.is_some() {
        patch.accounted_through_at = Some(Some(now));
    }
    repository::update(tx, recurrence.id, &patch, None)?;
    Ok(())
}

/// `accountSlots`: ledger rows for the slots between `accountedThroughAt` and `through`.
pub fn account_slots(
    tx: &Connection,
    recurrence: &BroadcastRecurrence,
    rule: &RecurrenceRule,
    through: i64,
    outcome: &str,
    reason: &str,
    now: i64,
) -> Result<()> {
    let Some(lower_bound) = recurrence.accounted_through_at else {
        return Ok(());
    };
    if lower_bound >= through {
        return Ok(());
    }
    let slots = list_occurrences_between(
        rule,
        lower_bound,
        through,
        recurrence.occurrence_count,
        MAX_BACKFILL_SLOTS_COUNTED,
    )
    .unwrap_or_default();
    if slots.is_empty() {
        return Ok(());
    }
    let individual_count = slots.len().min(MAX_LEDGER_BACKFILL_ROWS - 1);
    let older_count = slots.len() - individual_count;
    let row = |slot: i64, superseded: usize| LedgerRowInput {
        workspace_id: recurrence.workspace_id,
        recurrence_id: recurrence.id,
        local_slot: to_local_slot(slot, &rule.timezone),
        occurrence_at: slot,
        outcome: outcome.to_owned(),
        reason: Some(reason.to_owned()),
        child_broadcast_id: None,
        materialized_at: now,
        superseded_count: Some(superseded as i64),
    };
    let mut rows = Vec::with_capacity(individual_count + 1);
    if older_count > 0 {
        rows.push(row(slots[0], older_count));
    }
    rows.extend(slots[older_count..].iter().map(|slot| row(*slot, 1)));
    repository::insert_ledger_rows(tx, &rows)?;
    Ok(())
}

/// `nextSendAtOf`: the armed slot, or the first one after skipped dates.
pub fn next_send_at_of(
    rule: &Json,
    next_run_at: Option<i64>,
    occurrence_count: i64,
) -> Option<i64> {
    let from = next_run_at?;
    let rule = stored_rule(rule)?;
    compute_next_sendable(&rule, from, occurrence_count)
        .ok()
        .flatten()
}

/// `upcomingRunsOf`: the next `count` runs as ISO strings.
pub fn upcoming_runs_of(
    rule: &RecurrenceRule,
    occurrence_count: i64,
    now: i64,
    count: usize,
) -> Vec<String> {
    list_upcoming_occurrences(rule, now, occurrence_count, count)
        .unwrap_or_default()
        .into_iter()
        .filter_map(to_iso_string)
        .collect()
}

/// `previewUpcoming`: `{ upcomingRuns, timezone, weekStartsOn }`.
pub fn preview_upcoming(rule: &Value, count: usize, now: i64) -> Result<Value> {
    let rule = normalize_rule(rule)?;
    Ok(json!({
        "upcomingRuns": upcoming_runs_of(&rule, 0, now, count.min(BROADCAST_PREVIEW_MAX_COUNT)),
        "timezone": rule.timezone,
        "weekStartsOn": RECURRENCE_WEEK_STARTS_ON,
    }))
}
