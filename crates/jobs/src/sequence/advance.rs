//! `advance-enrollment.ts`: move an enrollment past the step just sent.

use chatbotx_db::rusqlite::{OptionalExtension, params};

use crate::sequence::dispatch::CreateDispatchParams;
use crate::sequence::next_run::StepTiming;
use crate::sequence::repo::{
    SequenceCtx, SequenceError, StatementKind, parse_id, read_tx, write_tx,
};
use crate::sequence::send_time::SendWindow;

/// The step that was just sent (`{ id, order }`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentStep {
    pub id: String,
    pub order: i64,
}

/// `advanceEnrollment` input (the scheduler is `ctx.schedule`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvanceParams {
    pub enrollment_id: String,
    pub workspace_id: String,
    pub sequence_id: String,
    pub contact_id: String,
    pub current_step: CurrentStep,
    pub sent_at_ms: i64,
}

/// `advanceEnrollment(params)`:
/// - missing enrollment: `Enrollment <id> not found`;
/// - status not `active`, or `lastStepId == current_step.id`: no change;
/// - no next active step: one UPDATE (no transaction) to `completed`,
///   `completedAt = sentAt`, `nextStepId/nextRunAt = NULL`,
///   `currentStep = order + 1`;
/// - else one transaction: UPDATE (`lastStepId`, `nextStepId`, `nextRunAt`,
///   `currentStep`) and one dispatch per contact inbox (`ctx.dispatches`,
///   run time from `ctx.next_run`); after the commit each dispatch is added
///   to `ctx.schedule`.
pub async fn advance_enrollment(
    ctx: &SequenceCtx,
    params: AdvanceParams,
) -> Result<(), SequenceError> {
    let worker = ctx.clone();
    let p = params.clone();
    let (enrollment, next_step) = read_tx(&ctx.db, move |conn| {
        worker.note(StatementKind::Select, "ContactOnSequence");
        let enrollment: Option<(Option<String>, Option<i64>)> = conn
            .query_row(
                r#"SELECT "status", "lastStepId" FROM "ContactOnSequence"
                   WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![parse_id(&p.enrollment_id)?, parse_id(&p.workspace_id)?],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some(enrollment) = enrollment else {
            return Ok((None, None));
        };
        worker.note(StatementKind::Select, "SequenceStep");
        let next: Option<NextStep> = conn
            .query_row(
                r#"SELECT "id", "order", "delayDays", "delayMinutes", "delayUnit", "specificDateTime",
                          "anytime", "sendTimeStart", "sendTimeEnd", "sendDays"
                   FROM "SequenceStep"
                   WHERE "sequenceId" = ?1 AND "order" > ?2 AND "isActive" = 1
                   ORDER BY "order" LIMIT 1"#,
                params![parse_id(&p.sequence_id)?, p.current_step.order],
                |r| {
                    Ok(NextStep {
                        id: r.get::<_, i64>(0)?.to_string(),
                        order: r.get(1)?,
                        timing: StepTiming {
                            delay_days: r.get(2)?,
                            delay_minutes: r.get(3)?,
                            delay_unit: r.get(4)?,
                            specific_date_time_ms: r.get(5)?,
                        },
                        window: SendWindow {
                            anytime: r.get(6)?,
                            send_time_start: r.get(7)?,
                            send_time_end: r.get(8)?,
                            send_days: r.get(9)?,
                        },
                    })
                },
            )
            .optional()?;
        Ok((Some(enrollment), next))
    })
    .await?;
    let Some((status, last_step_id)) = enrollment else {
        return Err(SequenceError::NotFound(format!(
            "Enrollment {} not found",
            params.enrollment_id
        )));
    };
    if status.as_deref() != Some("active") {
        return Ok(());
    }
    if last_step_id.map(|id| id.to_string()).as_deref() == Some(params.current_step.id.as_str()) {
        return Ok(());
    }
    let now = ctx.clock.now_ms();
    let Some(next) = next_step else {
        // Sequence completed: one statement, no transaction of its own.
        let worker = ctx.clone();
        let p = params.clone();
        write_tx(&ctx.db, move |conn| {
            worker.note(StatementKind::Update, "ContactOnSequence");
            conn.execute(
                r#"UPDATE "ContactOnSequence" SET "status" = 'completed', "completedAt" = ?3,
                   "currentStep" = ?4, "lastStepId" = ?5, "nextStepId" = NULL, "nextRunAt" = NULL,
                   "updatedAt" = ?6
                   WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![
                    parse_id(&p.enrollment_id)?,
                    parse_id(&p.workspace_id)?,
                    p.sent_at_ms,
                    p.current_step.order + 1,
                    parse_id(&p.current_step.id)?,
                    now
                ],
            )?;
            Ok(())
        })
        .await?;
        return Ok(());
    };
    let next_run_at = ctx.next_run.next_valid_send_time(
        ctx.next_run
            .next_run_at_from_step(&next.timing, params.sent_at_ms),
        &next.window,
    );
    let worker = ctx.clone();
    let p = params.clone();
    ctx.begin();
    let dispatches = write_tx(&ctx.db, move |conn| {
        worker.note(StatementKind::Update, "ContactOnSequence");
        conn.execute(
            r#"UPDATE "ContactOnSequence" SET "currentStep" = ?3, "lastStepId" = ?4,
               "nextStepId" = ?5, "nextRunAt" = ?6, "updatedAt" = ?7
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![
                parse_id(&p.enrollment_id)?,
                parse_id(&p.workspace_id)?,
                next.order,
                parse_id(&p.current_step.id)?,
                parse_id(&next.id)?,
                next_run_at,
                now
            ],
        )?;
        let inboxes =
            worker
                .contact_inboxes
                .contact_inboxes(conn, &p.workspace_id, &p.contact_id)?;
        let mut dispatches = Vec::with_capacity(inboxes.len());
        for inbox in inboxes {
            dispatches.push(worker.dispatches.create_dispatch(
                conn,
                &CreateDispatchParams {
                    workspace_id: p.workspace_id.clone(),
                    contact_id: p.contact_id.clone(),
                    contact_inbox_id: inbox.id,
                    enrollment_id: p.enrollment_id.clone(),
                    run_at_ms: next_run_at,
                    sequence_id: p.sequence_id.clone(),
                    step_id: next.id.clone(),
                },
            )?);
        }
        Ok(dispatches)
    })
    .await?;
    ctx.commit();
    for dispatch in dispatches {
        ctx.schedule
            .add_to_schedule(dispatch.bucket, &dispatch.id, dispatch.run_at_ms)
            .await?;
    }
    Ok(())
}

/// The next active step after the current one.
struct NextStep {
    id: String,
    order: i64,
    timing: StepTiming,
    window: SendWindow,
}
