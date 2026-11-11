//! `enroll-contact.ts`: `enrollContactInSequence` and
//! `enrollContactsInSequenceBulk`.

use std::sync::Arc;

use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};

use crate::sequence::dispatch::{CreateDispatchParams, DispatchRecord};
use crate::sequence::repo::{
    ClientMode, ContactInboxRef, SequenceCtx, SequenceError, StatementKind, parse_id, read_tx,
    write_tx,
};

/// `enrollContactInSequence` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnrollParams {
    pub workspace_id: String,
    pub contact_id: String,
    pub sequence_id: String,
    pub next_run_at_ms: i64,
    pub next_step_id: Option<String>,
    /// Contact inboxes the caller already has; `None` = look them up.
    pub contact_inboxes: Option<Vec<ContactInboxRef>>,
}

/// `{ enrolled }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnrollResult {
    pub enrolled: bool,
}

/// `enrollContactInSequence(params)`: `INSERT ... ON CONFLICT DO NOTHING
/// RETURNING id` of the `ContactsOnSequence` row (id from `ctx.ids`);
/// conflict = `{ enrolled: false }` and nothing else. With a next step, one
/// dispatch per contact inbox through `ctx.dispatches`, in the same
/// transaction (`ClientMode::Default`) or without one (`ClientMode::Root`);
/// the dispatches are added to `ctx.schedule` only after the commit.
pub async fn enroll_contact_in_sequence(
    ctx: &SequenceCtx,
    params: EnrollParams,
    mode: ClientMode,
) -> Result<EnrollResult, SequenceError> {
    let now = ctx.clock.now_ms();
    let enrollment_id = ctx.ids.next_id();
    let (enrolled, dispatches) = match mode {
        ClientMode::Default => {
            let worker = ctx.clone();
            let p = params.clone();
            ctx.begin();
            let outcome = write_tx(&ctx.db, move |conn| {
                enroll_in(conn, &worker, &p, &enrollment_id, now)
            })
            .await?;
            ctx.commit();
            outcome
        }
        ClientMode::Root => {
            // The root client: every statement commits on its own.
            let worker = ctx.clone();
            let p = params.clone();
            let id = enrollment_id.clone();
            let inserted = write_tx(&ctx.db, move |conn| {
                insert_enrollment(conn, &worker, &p, &id, now)
            })
            .await?;
            if !inserted {
                (false, Vec::new())
            } else if let Some(step_id) = params.next_step_id.clone() {
                let inboxes = match params.contact_inboxes.clone() {
                    Some(list) => list,
                    None => {
                        let lookup = Arc::clone(&ctx.contact_inboxes);
                        let (ws, contact) =
                            (params.workspace_id.clone(), params.contact_id.clone());
                        read_tx(&ctx.db, move |conn| {
                            lookup.contact_inboxes(conn, &ws, &contact)
                        })
                        .await?
                    }
                };
                let mut dispatches = Vec::new();
                for inbox in inboxes {
                    let creator = Arc::clone(&ctx.dispatches);
                    let create = dispatch_params(&params, &enrollment_id, &step_id, &inbox.id);
                    dispatches.push(
                        write_tx(&ctx.db, move |conn| creator.create_dispatch(conn, &create))
                            .await?,
                    );
                }
                (true, dispatches)
            } else {
                (true, Vec::new())
            }
        }
    };
    // Scheduled only after the enrollment committed.
    for dispatch in dispatches {
        ctx.schedule
            .add_to_schedule(dispatch.bucket, &dispatch.id, dispatch.run_at_ms)
            .await?;
    }
    Ok(EnrollResult { enrolled })
}

fn dispatch_params(
    params: &EnrollParams,
    enrollment_id: &str,
    step_id: &str,
    contact_inbox_id: &str,
) -> CreateDispatchParams {
    CreateDispatchParams {
        workspace_id: params.workspace_id.clone(),
        contact_id: params.contact_id.clone(),
        contact_inbox_id: contact_inbox_id.to_owned(),
        enrollment_id: enrollment_id.to_owned(),
        run_at_ms: params.next_run_at_ms,
        sequence_id: params.sequence_id.clone(),
        step_id: step_id.to_owned(),
    }
}

/// `INSERT ... ON CONFLICT DO NOTHING RETURNING id`: `false` = already
/// enrolled (or a concurrent enrollment won the unique index).
fn insert_enrollment(
    conn: &Connection,
    ctx: &SequenceCtx,
    params: &EnrollParams,
    enrollment_id: &str,
    now: i64,
) -> Result<bool, SequenceError> {
    ctx.note(StatementKind::Insert, "ContactOnSequence");
    let step = params.next_step_id.as_deref().map(parse_id).transpose()?;
    let inserted = conn
        .prepare_cached(
            r#"INSERT INTO "ContactOnSequence" ("id", "workspaceId", "contactId", "sequenceId",
               "currentStep", "status", "nextRunAt", "nextStepId", "enrolledAt")
               VALUES (?1, ?2, ?3, ?4, 0, 'active', ?5, ?6, ?7)
               ON CONFLICT DO NOTHING RETURNING "id""#,
        )?
        .query_row(
            params![
                parse_id(enrollment_id)?,
                parse_id(&params.workspace_id)?,
                parse_id(&params.contact_id)?,
                parse_id(&params.sequence_id)?,
                params.next_run_at_ms,
                step,
                now
            ],
            |r| r.get::<_, i64>(0),
        )
        .optional()?;
    Ok(inserted.is_some())
}

/// The body of `enroll(dbClient)` on one connection.
fn enroll_in(
    conn: &Connection,
    ctx: &SequenceCtx,
    params: &EnrollParams,
    enrollment_id: &str,
    now: i64,
) -> Result<(bool, Vec<DispatchRecord>), SequenceError> {
    if !insert_enrollment(conn, ctx, params, enrollment_id, now)? {
        return Ok((false, Vec::new()));
    }
    let Some(step_id) = params.next_step_id.as_deref() else {
        return Ok((true, Vec::new()));
    };
    let inboxes = match &params.contact_inboxes {
        Some(list) => list.clone(),
        None => {
            ctx.contact_inboxes
                .contact_inboxes(conn, &params.workspace_id, &params.contact_id)?
        }
    };
    let mut dispatches = Vec::with_capacity(inboxes.len());
    for inbox in inboxes {
        let create = dispatch_params(params, enrollment_id, step_id, &inbox.id);
        dispatches.push(ctx.dispatches.create_dispatch(conn, &create)?);
    }
    Ok((true, dispatches))
}

/// One enrollment of a bulk enroll.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BulkEnrollment {
    pub contact_id: String,
    pub sequence_id: String,
    pub next_run_at_ms: Option<i64>,
    pub next_step_id: Option<String>,
}

/// `enrollContactsInSequenceBulk({ workspaceId, enrollments })`: one bulk
/// insert (`ON CONFLICT DO NOTHING RETURNING`); only inserted rows with both
/// a next step and a run time get dispatches (one per contact inbox), then
/// each is scheduled after the commit.
pub async fn enroll_contacts_in_sequence_bulk(
    ctx: &SequenceCtx,
    workspace_id: &str,
    enrollments: Vec<BulkEnrollment>,
) -> Result<(), SequenceError> {
    if enrollments.is_empty() {
        return Ok(());
    }
    let now = ctx.clock.now_ms();
    let rows: Vec<(String, BulkEnrollment)> = enrollments
        .into_iter()
        .map(|e| (ctx.ids.next_id(), e))
        .collect();
    let worker = ctx.clone();
    let ws = workspace_id.to_owned();
    // One multi-row insert (one statement, no transaction of its own).
    let created: Vec<(String, BulkEnrollment)> = write_tx(&ctx.db, move |conn| {
        worker.note(StatementKind::Insert, "ContactOnSequence");
        let workspace = parse_id(&ws)?;
        let mut values = Vec::with_capacity(rows.len());
        let mut binds: Vec<chatbotx_db::rusqlite::types::Value> = Vec::new();
        for (i, (id, e)) in rows.iter().enumerate() {
            let base = i * 7;
            values.push(format!(
                "(?{}, ?{}, ?{}, ?{}, 0, 'active', ?{}, ?{}, ?{})",
                base + 1,
                base + 2,
                base + 3,
                base + 4,
                base + 5,
                base + 6,
                base + 7
            ));
            use chatbotx_db::rusqlite::types::Value as V;
            binds.push(V::Integer(parse_id(id)?));
            binds.push(V::Integer(workspace));
            binds.push(V::Integer(parse_id(&e.contact_id)?));
            binds.push(V::Integer(parse_id(&e.sequence_id)?));
            binds.push(e.next_run_at_ms.map_or(V::Null, V::Integer));
            binds.push(match &e.next_step_id {
                Some(step) => V::Integer(parse_id(step)?),
                None => V::Null,
            });
            binds.push(V::Integer(now));
        }
        let sql = format!(
            r#"INSERT INTO "ContactOnSequence" ("id", "workspaceId", "contactId", "sequenceId",
               "currentStep", "status", "nextRunAt", "nextStepId", "enrolledAt")
               VALUES {} ON CONFLICT DO NOTHING RETURNING "id""#,
            values.join(", ")
        );
        let inserted: Vec<i64> = conn
            .prepare(&sql)?
            .query_map(chatbotx_db::rusqlite::params_from_iter(binds), |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(rows
            .into_iter()
            .filter(|(id, _)| inserted.iter().any(|n| n.to_string() == *id))
            .collect())
    })
    .await?;
    for (enrollment_id, e) in created {
        let (Some(step_id), Some(run_at)) = (e.next_step_id.clone(), e.next_run_at_ms) else {
            continue;
        };
        let lookup = Arc::clone(&ctx.contact_inboxes);
        let (ws, contact) = (workspace_id.to_owned(), e.contact_id.clone());
        let inboxes = read_tx(&ctx.db, move |conn| {
            lookup.contact_inboxes(conn, &ws, &contact)
        })
        .await?;
        for inbox in inboxes {
            let creator = Arc::clone(&ctx.dispatches);
            let create = CreateDispatchParams {
                workspace_id: workspace_id.to_owned(),
                contact_id: e.contact_id.clone(),
                contact_inbox_id: inbox.id,
                enrollment_id: enrollment_id.clone(),
                run_at_ms: run_at,
                sequence_id: e.sequence_id.clone(),
                step_id: step_id.clone(),
            };
            let dispatch =
                write_tx(&ctx.db, move |conn| creator.create_dispatch(conn, &create)).await?;
            ctx.schedule
                .add_to_schedule(dispatch.bucket, &dispatch.id, dispatch.run_at_ms)
                .await?;
        }
    }
    Ok(())
}
