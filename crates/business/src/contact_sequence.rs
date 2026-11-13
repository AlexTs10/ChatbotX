//! `packages/business/src/contact-sequence/service.ts` (P3.6-D2 slice
//! contact sequences): `subscribeContacts`, `listByContactId`,
//! `removeContactSequencesForContacts`, `updateContactSequences`. Private
//! queries on `"Sequence"`, `"ContactOnSequence"`, `"Contact"`,
//! `"ContactInbox"`; dispatch rows through `chatbotx_jobs::sequence`
//! (`create_dispatch_in`, `cancel_pending_dispatches_in`). No scheduler
//! add / remove and no `sequenceUnsubscribed` events (the API state has no
//! scheduler or event bus).

use std::collections::HashSet;

use chatbotx_db::Db;
use chatbotx_db::repo::{self, new_id};
use chatbotx_db::rusqlite::types::Value;
use chatbotx_db::rusqlite::{Connection, params, params_from_iter};
use chatbotx_jobs::SystemClock;
use chatbotx_jobs::sequence::cancel::{CancelPendingParams, cancel_pending_dispatches_in};
use chatbotx_jobs::sequence::dispatch::{
    CreateDispatchParams, SqlDispatchInserter, create_dispatch_in,
};
use chatbotx_jobs::sequence::next_run::calculate_next_run_at_in;
use chatbotx_jobs::sequence::repo::{CreateId, SequenceError};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `CHUNK_SIZE` of the contact lookups.
pub const CHUNK_SIZE: usize = 1000;

/// `{ processedContactIds, skippedContactIds }`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubscribeOutcome {
    pub processed_contact_ids: Vec<i64>,
    pub skipped_contact_ids: Vec<i64>,
}

/// One `listByContactId` row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactSequenceRow {
    pub sequence_id: i64,
    pub sequence_name: String,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn sequence_error(error: SequenceError) -> BusinessError {
    BusinessError::Repo(repo::Error::msg(format!("{error:?}")))
}

fn in_list(column: &str, ids: &[i64]) -> (String, Vec<Value>) {
    let marks = vec!["?"; ids.len()].join(", ");
    (
        format!("{column} IN ({marks})"),
        ids.iter().copied().map(Value::Integer).collect(),
    )
}

fn ids_query(conn: &Connection, sql: &str, params: Vec<Value>) -> repo::Result<Vec<i64>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |r| r.get::<_, i64>(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

/// `assertSequencesInWorkspace`: 404 "Sequence not found" when any id is
/// not one of the workspace's sequences.
fn assert_sequences_in_workspace(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let (clause, mut params) = in_list("\"id\"", ids);
    params.insert(0, Value::Integer(workspace_id));
    let owned: HashSet<i64> = ids_query(
        conn,
        &format!("SELECT \"id\" FROM \"Sequence\" WHERE \"workspaceId\" = ? AND {clause}"),
        params,
    )?
    .into_iter()
    .collect();
    if ids.iter().any(|id| !owned.contains(id)) {
        return Err(BusinessError::not_found("Sequence not found"));
    }
    Ok(())
}

/// `enrollContactInSequence` (database part): the enrollment at the first
/// active step (`ON CONFLICT DO NOTHING`), then one dispatch per contact
/// inbox when there is a next step.
fn enroll(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
    sequence_id: i64,
    now: i64,
) -> Result<()> {
    let next = calculate_next_run_at_in(conn, &SystemClock, &sequence_id.to_string(), now)
        .map_err(sequence_error)?;
    let enrollment_id = new_id();
    let next_step_id: Option<i64> = next.next_step_id.as_deref().and_then(|id| id.parse().ok());
    let inserted = conn
        .execute(
            r#"INSERT INTO "ContactOnSequence" ("id", "createdAt", "updatedAt", "enrolledAt", "currentStep", "status", "nextRunAt", "nextStepId", "contactId", "sequenceId", "workspaceId")
               VALUES (?1, ?2, ?2, ?2, 0, 'active', ?3, ?4, ?5, ?6, ?7) ON CONFLICT DO NOTHING"#,
            params![enrollment_id, now, next.next_run_at_ms, next_step_id, contact_id, sequence_id, workspace_id],
        )
        .map_err(repo::Error::from)?;
    let (Some(step_id), 1) = (next.next_step_id, inserted) else {
        return Ok(());
    };
    let inboxes = ids_query(
        conn,
        r#"SELECT "id" FROM "ContactInbox" WHERE "contactId" = ? ORDER BY "id""#,
        vec![Value::Integer(contact_id)],
    )?;
    for contact_inbox_id in inboxes {
        create_dispatch_in(
            conn,
            &SqlDispatchInserter,
            &CreateId,
            &CreateDispatchParams {
                workspace_id: workspace_id.to_string(),
                contact_id: contact_id.to_string(),
                contact_inbox_id: contact_inbox_id.to_string(),
                enrollment_id: enrollment_id.to_string(),
                run_at_ms: next.next_run_at_ms,
                sequence_id: sequence_id.to_string(),
                step_id: step_id.clone(),
            },
        )
        .map_err(sequence_error)?;
    }
    Ok(())
}

/// `removeEnrollmentsWithClient`: cancels the pending dispatches of each
/// enrollment, then deletes it.
fn remove_in(
    conn: &Connection,
    workspace_id: i64,
    contact_ids: &[i64],
    sequence_ids: &[i64],
) -> Result<()> {
    if contact_ids.is_empty() || sequence_ids.is_empty() {
        return Ok(());
    }
    let (contacts, contact_params) = in_list("\"contactId\"", contact_ids);
    let (sequences, sequence_params) = in_list("\"sequenceId\"", sequence_ids);
    let mut params = vec![Value::Integer(workspace_id)];
    params.extend(contact_params);
    params.extend(sequence_params);
    let enrollments = ids_query(
        conn,
        &format!(
            "SELECT \"id\" FROM \"ContactOnSequence\" WHERE \"workspaceId\" = ? AND {contacts} AND {sequences}"
        ),
        params,
    )?;
    for enrollment in enrollments {
        let mut cancel =
            CancelPendingParams::new(&enrollment.to_string(), &workspace_id.to_string());
        cancel.remove_from_schedule = false;
        cancel_pending_dispatches_in(conn, None, &cancel).map_err(sequence_error)?;
        conn.execute(
            r#"DELETE FROM "ContactOnSequence" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![enrollment, workspace_id],
        )
        .map_err(repo::Error::from)?;
    }
    Ok(())
}

fn list_in(
    conn: &Connection,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<ContactSequenceRow>> {
    let mut stmt = conn
        .prepare(
            r#"SELECT cos."sequenceId", s."name" FROM "ContactOnSequence" AS cos
               JOIN "Sequence" AS s ON s."id" = cos."sequenceId"
               WHERE cos."workspaceId" = ?1 AND cos."contactId" = ?2 ORDER BY cos."id""#,
        )
        .map_err(repo::Error::from)?;
    let rows = stmt
        .query_map(params![workspace_id, contact_id], |r| {
            Ok(ContactSequenceRow {
                sequence_id: r.get(0)?,
                sequence_name: r.get(1)?,
            })
        })
        .map_err(repo::Error::from)?;
    Ok(rows
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(repo::Error::from)?)
}

fn contact_exists(conn: &Connection, workspace_id: i64, contact_id: i64) -> Result<bool> {
    Ok(!ids_query(
        conn,
        r#"SELECT "id" FROM "Contact" WHERE "id" = ? AND "workspaceId" = ?"#,
        vec![Value::Integer(contact_id), Value::Integer(workspace_id)],
    )?
    .is_empty())
}

/// `subscribeContacts`: every sequence must be the workspace's (404
/// "Sequence not found", nothing enrolled); contacts that do not resolve are
/// skipped; an existing enrollment is kept.
pub async fn subscribe_contacts(
    db: &Db,
    workspace_id: i64,
    contact_ids: &[i64],
    sequence_ids: &[i64],
) -> Result<SubscribeOutcome> {
    let contact_ids = contact_ids.to_vec();
    let sequence_ids = sequence_ids.to_vec();
    tx::write(db, move |tx| {
        assert_sequences_in_workspace(tx, workspace_id, &sequence_ids)?;
        let now = now_ms();
        let mut processed = Vec::new();
        for chunk in contact_ids.chunks(CHUNK_SIZE) {
            let (clause, mut params) = in_list("\"id\"", chunk);
            params.insert(0, Value::Integer(workspace_id));
            let found: HashSet<i64> = ids_query(
                tx,
                &format!("SELECT \"id\" FROM \"Contact\" WHERE \"workspaceId\" = ? AND {clause}"),
                params,
            )?
            .into_iter()
            .collect();
            for contact_id in chunk.iter().copied().filter(|id| found.contains(id)) {
                if processed.contains(&contact_id) {
                    continue;
                }
                processed.push(contact_id);
                for sequence_id in &sequence_ids {
                    enroll(tx, workspace_id, contact_id, *sequence_id, now)?;
                }
            }
        }
        let skipped = contact_ids
            .iter()
            .copied()
            .filter(|id| !processed.contains(id))
            .collect();
        Ok(SubscribeOutcome {
            processed_contact_ids: processed,
            skipped_contact_ids: skipped,
        })
    })
    .await
}

/// `listByContactId`: 404 "Contact not found" for a contact outside the
/// workspace.
pub async fn list_by_contact_id(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
) -> Result<Vec<ContactSequenceRow>> {
    tx::read(db, move |conn| {
        if !contact_exists(conn, workspace_id, contact_id)? {
            return Err(BusinessError::not_found("Contact not found"));
        }
        list_in(conn, workspace_id, contact_id)
    })
    .await
}

/// `removeContactSequencesForContacts` (`subscription_removed`): deletes the
/// enrollments and cancels their pending dispatches, in one transaction.
pub async fn remove_contact_sequences_for_contacts(
    db: &Db,
    workspace_id: i64,
    contact_ids: &[i64],
    sequence_ids: &[i64],
) -> Result<()> {
    let contact_ids = contact_ids.to_vec();
    let sequence_ids = sequence_ids.to_vec();
    tx::write(db, move |tx| {
        remove_in(tx, workspace_id, &contact_ids, &sequence_ids)
    })
    .await
}

/// `updateContactSequences`: replaces the contact's set in one transaction
/// (a sequence to add that is not the workspace's is 404 and changes nothing).
pub async fn update_contact_sequences(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    sequence_ids: &[i64],
) -> Result<Vec<ContactSequenceRow>> {
    let sequence_ids = sequence_ids.to_vec();
    tx::write(db, move |tx| {
        let current: Vec<i64> = list_in(tx, workspace_id, contact_id)?
            .into_iter()
            .map(|row| row.sequence_id)
            .collect();
        let mut to_add: Vec<i64> = Vec::new();
        for id in &sequence_ids {
            if !current.contains(id) && !to_add.contains(id) {
                to_add.push(*id);
            }
        }
        let to_remove: Vec<i64> = current
            .iter()
            .copied()
            .filter(|id| !sequence_ids.contains(id))
            .collect();
        assert_sequences_in_workspace(tx, workspace_id, &to_add)?;
        remove_in(tx, workspace_id, &[contact_id], &to_remove)?;
        let now = now_ms();
        for sequence_id in to_add {
            enroll(tx, workspace_id, contact_id, sequence_id, now)?;
        }
        list_in(tx, workspace_id, contact_id)
    })
    .await
}
