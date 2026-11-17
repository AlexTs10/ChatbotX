//! The worker side of `broadcast/recurrence.service.ts`: `listDue`,
//! `findOldestDueAgeMs`, `skipOccurrence` and `materializeOccurrence`, which
//! turns one due slot of a repeating broadcast into a run or a reasoned skip.

use chatbotx_core::utils::recurrence::{
    RecurrenceRule, StepLimitExceeded, compute_latest_occurrence, compute_next_occurrence,
    is_excluded_slot, to_local_slot,
};
use chatbotx_db::Db;
use chatbotx_db::repo::broadcast_recurrence::{
    self as repository, DueRecurrence, DueRecurrenceCursor, LedgerRowInput, RecurrencePatch,
};
use chatbotx_db::repo::models::{Broadcast, BroadcastRecurrence};
use chatbotx_db::repo::partials::broadcast as partials;
use chatbotx_db::repo::{self, now_ms};
use chatbotx_db::rusqlite::types::Value as SqlValue;
use chatbotx_db::rusqlite::{Connection, params};

use super::plan_policy::{self, BROADCAST_PLAN_LIMIT_CODE, PlanContext};
use super::recurrence::{DisarmReason, account_slots, disarm, stored_rule};
use super::write::{copied_columns, copy_targets, insert_broadcast};
use super::{chatbotx_exception, is_recurring_template};
use crate::audit::AuditService;
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `MaterializeOccurrenceInput`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterializeInput {
    pub recurrence_id: i64,
    /// The workspace the job was enqueued for; the series must belong to it.
    pub workspace_id: i64,
    pub occurrence_at: i64,
}

/// `MaterializeOccurrenceResult` (`NOTHING_MATERIALIZED` is the default).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaterializeResult {
    pub created_broadcast_id: Option<i64>,
    pub series_ended: bool,
    /// `planLimit`, `noTargets` or `undeliverable`.
    pub skipped: Option<&'static str>,
    /// The slot was already settled by an earlier attempt; this call changed nothing.
    pub replayed: bool,
}

/// `reserveCapacity`: the clamped rate, nothing to reserve, or a full slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capacity {
    Unrestricted,
    Rate(i64),
    PlanLimit,
}

enum SlotDecision {
    Created(i64),
    Skipped(&'static str),
}

/// `listDue`.
pub async fn list_due(
    db: &Db,
    limit: i64,
    cursor: Option<DueRecurrenceCursor>,
) -> Result<Vec<DueRecurrence>> {
    tx::read(db, move |conn| {
        let now = repository::select_db_now(conn)?;
        Ok(repository::list_due(conn, limit, cursor, now)?)
    })
    .await
}

/// `findOldestDueAgeMs`.
pub async fn find_oldest_due_age_ms(db: &Db) -> Result<Option<i64>> {
    tx::read(db, |conn| {
        let now = repository::select_db_now(conn)?;
        Ok(repository::find_oldest_due_age_ms(conn, now)?)
    })
    .await
}

/// `skipOccurrence`: a blocked owner's slot is settled as skipped and the series moves on.
pub async fn skip_occurrence(db: &Db, input: MaterializeInput, reason: &'static str) -> Result<()> {
    tx::write(db, move |tx| {
        let Some((recurrence, template)) =
            repository::find_for_materialize(tx, input.recurrence_id, input.workspace_id)?
        else {
            return Ok(());
        };
        if !is_due_slot(&recurrence, input.occurrence_at) {
            return Ok(());
        }
        let rule = rule_of(&recurrence)?;
        let now = repository::select_db_now(tx)?;
        settle_slot(
            tx,
            &recurrence,
            &rule,
            &template,
            input.occurrence_at,
            now,
            SlotDecision::Skipped(reason),
        )?;
        Ok(())
    })
    .await
}

/// `materializeOccurrence`: the ledger row, the run, its targets and the cursor
/// commit together. A replayed job finds the slot settled and changes nothing.
pub async fn materialize_occurrence(
    db: &Db,
    audit: &AuditService,
    input: MaterializeInput,
) -> Result<MaterializeResult> {
    let result = tx::write(db, move |tx| {
        let Some((recurrence, template)) =
            repository::find_for_materialize(tx, input.recurrence_id, input.workspace_id)?
        else {
            return Ok(MaterializeResult::default());
        };
        let now = repository::select_db_now(tx)?;
        let rule = rule_of(&recurrence)?;
        if !is_due_slot(&recurrence, input.occurrence_at) {
            return find_settled_run(tx, &recurrence, &rule, input.occurrence_at);
        }
        if !is_live_template(&template) {
            disarm(tx, &recurrence, DisarmReason::Draft, now)?;
            return Ok(MaterializeResult::default());
        }
        let effective = resolve_effective_slot(&recurrence, &rule, input.occurrence_at, now)?;
        account_slots(
            tx,
            &recurrence,
            &rule,
            effective - 1,
            "missed",
            "catchUpSuperseded",
            now,
        )?;
        let decision = decide_slot(tx, &rule, &template, effective)?;
        settle_slot(tx, &recurrence, &rule, &template, effective, now, decision)
    })
    .await?;
    if let (Some(id), false) = (result.created_broadcast_id, result.replayed) {
        audit
            .audit(
                "launch",
                &format!("launched a recurring broadcast run (#{id})"),
            )
            .await;
    }
    Ok(result)
}

fn rule_of(recurrence: &BroadcastRecurrence) -> Result<RecurrenceRule> {
    stored_rule(&recurrence.rule).ok_or_else(|| chatbotx_exception("Invalid recurrence rule"))
}

fn step<T>(result: std::result::Result<T, StepLimitExceeded>) -> Result<T> {
    result.map_err(|e| chatbotx_exception(&e.to_string()))
}

/// `isDueSlot`: only the armed `nextRunAt` is actionable.
fn is_due_slot(recurrence: &BroadcastRecurrence, occurrence_at: i64) -> bool {
    recurrence.next_run_at == Some(occurrence_at)
}

/// `isLiveTemplate`.
fn is_live_template(template: &Broadcast) -> bool {
    template.status == "recurring"
        && template.deleted_at.is_none()
        && is_recurring_template(template)
}

/// `resolveEffectiveSlot`: the latest slot not in the future; after an outage only it is sent.
fn resolve_effective_slot(
    recurrence: &BroadcastRecurrence,
    rule: &RecurrenceRule,
    occurrence_at: i64,
    now: i64,
) -> Result<i64> {
    let latest = step(compute_latest_occurrence(
        rule,
        now,
        recurrence.occurrence_count,
    ))?;
    Ok(latest
        .filter(|l| *l > occurrence_at)
        .unwrap_or(occurrence_at))
}

/// `findSettledRun`: a retried job reports the decision the first attempt took.
fn find_settled_run(
    tx: &Connection,
    recurrence: &BroadcastRecurrence,
    rule: &RecurrenceRule,
    occurrence_at: i64,
) -> Result<MaterializeResult> {
    let row = |at: i64| {
        repository::find_ledger_row(tx, recurrence.id, &to_local_slot(at, &rule.timezone), at)
    };
    let own = row(occurrence_at)?;
    let superseded_by = match (&own, recurrence.last_occurrence_at) {
        (Some(own), Some(last)) if own.outcome == "missed" && last > occurrence_at => row(last)?,
        _ => None,
    };
    // No ledger row: the job was merely stale (paused or edited since enqueued).
    let Some(settled) = superseded_by.or(own) else {
        return Ok(MaterializeResult::default());
    };
    Ok(MaterializeResult {
        created_broadcast_id: settled.child_broadcast_id,
        series_ended: recurrence.next_run_at.is_none() && recurrence.ended_at.is_some(),
        skipped: None,
        replayed: true,
    })
}

/// `decideSlot`: skip (excluded, nothing to deliver, no capacity) or create the run.
fn decide_slot(
    tx: &Connection,
    rule: &RecurrenceRule,
    template: &Broadcast,
    occurrence_at: i64,
) -> Result<SlotDecision> {
    // Checked first: a skipped date must not reserve capacity or touch targets.
    if is_excluded_slot(rule, occurrence_at) {
        return Ok(SlotDecision::Skipped("excludedDate"));
    }
    if let Some(reason) = find_blocking_reason(tx, template)? {
        return Ok(SlotDecision::Skipped(reason));
    }
    let rate = match reserve_capacity(
        tx,
        template,
        &plan_policy::resolve_for_workspace(template.workspace_id),
    )? {
        Capacity::PlanLimit => return Ok(SlotDecision::Skipped("planLimit")),
        Capacity::Rate(rate) => Some(rate),
        Capacity::Unrestricted => None,
    };
    Ok(SlotDecision::Created(insert_run(
        tx,
        template,
        rule,
        occurrence_at,
        rate,
    )?))
}

/// `findBlockingReason`: a page or flow removed since arming leaves nothing deliverable.
fn find_blocking_reason(tx: &Connection, template: &Broadcast) -> Result<Option<&'static str>> {
    let json = template.to_json();
    if !partials::uses_broadcast_targets(&json) {
        if partials::is_template_send_without_page(&json) {
            return Ok(Some("undeliverable"));
        }
        let sends = template.flow_id.is_some() || template.template_id.is_some();
        return Ok((!sends).then_some("undeliverable"));
    }
    let (count, deliverable): (i64, i64) = tx.query_row(
        r#"SELECT count(*), count(*) FILTER (WHERE "flowId" IS NOT NULL OR "templateId" IS NOT NULL)
           FROM "BroadcastTarget" WHERE "broadcastId" = ?1"#,
        params![template.id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(if count == 0 {
        Some("noTargets")
    } else if deliverable == 0 {
        Some("undeliverable")
    } else {
        None
    })
}

/// `reserveCapacity`: checked BEFORE the run is inserted. The template rate is
/// clamped to the plan's CURRENT cap.
pub fn reserve_capacity(
    tx: &Connection,
    template: &Broadcast,
    plan: &PlanContext,
) -> Result<Capacity> {
    if !plan_policy::applies_to_channel(&template.channel) {
        return Ok(Capacity::Unrestricted);
    }
    let Some(restriction) = plan_policy::restriction_for(plan, &template.channel) else {
        return Ok(Capacity::Unrestricted);
    };
    let cap = restriction.policy.max_send_rate_per_minute;
    let rate = template.send_rate_per_minute.unwrap_or(cap).min(cap);
    match plan_policy::assert_activation_allowed(
        tx,
        template.workspace_id,
        &template.channel,
        &restriction,
        Some(rate),
        None,
    ) {
        Ok(rate) => Ok(Capacity::Rate(rate)),
        Err(error) if is_active_broadcasts_limit(&error) => Ok(Capacity::PlanLimit),
        Err(error) => Err(error),
    }
}

/// `isActiveBroadcastsLimit`.
pub fn is_active_broadcasts_limit(error: &BusinessError) -> bool {
    matches!(
        error,
        BusinessError::Exception { code, data: Some(data), .. }
            if code == BROADCAST_PLAN_LIMIT_CODE && data["reason"] == "activeBroadcasts"
    )
}

/// `insertRun`: one instant is one send, so an existing run is reused.
fn insert_run(
    tx: &Connection,
    template: &Broadcast,
    rule: &RecurrenceRule,
    occurrence_at: i64,
    rate_override: Option<i64>,
) -> Result<i64> {
    // ponytail: the original inserts ON CONFLICT DO NOTHING and reads back; under
    // the single SQLite writer a read first is the same and needs no conflict path.
    if let Some(existing) = repo::find_one::<Broadcast>(
        tx,
        r#""parentBroadcastId" = ?1 AND "occurrenceAt" = ?2"#,
        &[&template.id, &occurrence_at],
    )? {
        return Ok(existing.id);
    }
    // `buildDerivedBroadcastValues` plus the run's own columns.
    let mut columns = copied_columns(template);
    let json = |v: &Option<repo::Json>| {
        v.as_ref()
            .map_or(SqlValue::Null, |j| SqlValue::Text(j.0.to_string()))
    };
    columns.extend([
        ("contactFilter", json(&template.contact_filter)),
        (
            "name",
            SqlValue::Text(format!(
                "{} · {}",
                template.name,
                to_local_slot(occurrence_at, &rule.timezone)
            )),
        ),
        ("status", SqlValue::Text("scheduled".into())),
        ("schedulesType", SqlValue::Text("future".into())),
        ("schedulesAt", SqlValue::Integer(occurrence_at)),
        ("parentBroadcastId", SqlValue::Integer(template.id)),
        ("occurrenceAt", SqlValue::Integer(occurrence_at)),
    ]);
    if let Some(rate) = rate_override {
        columns.retain(|(c, _)| *c != "sendRatePerMinute");
        columns.push(("sendRatePerMinute", SqlValue::Integer(rate)));
    }
    let run = insert_broadcast(tx, &columns)?;
    copy_targets(tx, template.id, run.id)?;
    Ok(run.id)
}

/// `settleSlot`: the ledger row, then a compare-and-set on `nextRunAt`. Only a
/// created run counts toward an `afterOccurrences` budget.
fn settle_slot(
    tx: &Connection,
    recurrence: &BroadcastRecurrence,
    rule: &RecurrenceRule,
    template: &Broadcast,
    occurrence_at: i64,
    now: i64,
    decision: SlotDecision,
) -> Result<MaterializeResult> {
    let (child, reason) = match decision {
        SlotDecision::Created(id) => (Some(id), None),
        SlotDecision::Skipped(reason) => (None, Some(reason)),
    };
    repository::insert_ledger_rows(
        tx,
        &[LedgerRowInput {
            workspace_id: recurrence.workspace_id,
            recurrence_id: recurrence.id,
            local_slot: to_local_slot(occurrence_at, &rule.timezone),
            occurrence_at,
            outcome: if child.is_some() {
                "created"
            } else {
                "skipped"
            }
            .to_owned(),
            reason: reason.map(str::to_owned),
            child_broadcast_id: child,
            materialized_at: now,
            superseded_count: None,
        }],
    )?;
    let occurrence_count = recurrence.occurrence_count + i64::from(child.is_some());
    let next_run_at = step(compute_next_occurrence(
        rule,
        occurrence_at,
        occurrence_count,
    ))?;
    let advanced = repository::update(
        tx,
        recurrence.id,
        &RecurrencePatch {
            next_run_at: Some(next_run_at),
            accounted_through_at: Some(Some(occurrence_at)),
            occurrence_count: Some(occurrence_count),
            last_occurrence_at: Some(Some(occurrence_at)),
            ended_at: next_run_at.is_none().then_some(Some(now)),
            ..RecurrencePatch::default()
        },
        recurrence.next_run_at,
    )?;
    if !advanced {
        return Err(chatbotx_exception("Recurrence moved while materializing"));
    }
    let status = if next_run_at.is_none() {
        "sent"
    } else {
        template.status.as_str()
    };
    tx.execute(
        r#"UPDATE "Broadcast" SET "schedulesAt" = ?1, "status" = ?2, "updatedAt" = ?3 WHERE "id" = ?4"#,
        params![next_run_at.unwrap_or(occurrence_at), status, now_ms(), template.id],
    )?;
    Ok(MaterializeResult {
        created_broadcast_id: child,
        series_ended: next_run_at.is_none(),
        skipped: reason.filter(|r| matches!(*r, "planLimit" | "noTargets" | "undeliverable")),
        replayed: false,
    })
}
