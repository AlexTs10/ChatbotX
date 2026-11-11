//! `dispatch-manager.ts`: bucket hash, idempotency key and `createDispatch`.

use std::sync::Arc;

use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};

use crate::sequence::repo::{
    ClientMode, IdGenerator, SequenceCtx, SequenceError, parse_id, write_tx,
};

/// `calculateBucket(workspaceId, contactId)`: base-31 hash over UTF-16 code
/// units, mod 2 147 483 647, then `% 256` (0..=255). Persisted on the row.
pub fn calculate_bucket(workspace_id: &str, contact_id: &str) -> u32 {
    let key = format!("{workspace_id}:{contact_id}");
    let mut hash: u64 = 0;
    for unit in key.encode_utf16() {
        hash = (hash * 31 + u64::from(unit)) % 2_147_483_647;
    }
    (hash % 256) as u32
}

/// `generateIdempotencyKey`: `<ws>:<enrollment>:<step>:<runAt.toISOString()>`
/// (ISO with milliseconds and `Z`).
pub fn generate_idempotency_key(
    workspace_id: &str,
    enrollment_id: &str,
    step_id: &str,
    run_at_ms: i64,
) -> String {
    format!(
        "{workspace_id}:{enrollment_id}:{step_id}:{}",
        crate::schedules::iso_ms(run_at_ms)
    )
}

/// `createDispatch` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CreateDispatchParams {
    pub workspace_id: String,
    pub contact_id: String,
    pub contact_inbox_id: String,
    pub enrollment_id: String,
    pub run_at_ms: i64,
    pub sequence_id: String,
    pub step_id: String,
}

/// The `RETURNING { id, bucket, runAtMs }` of `createDispatch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchRecord {
    pub id: String,
    pub bucket: u32,
    pub run_at_ms: i64,
}

/// The `SequenceDispatch` row `createDispatch` inserts (status `pending`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDispatchRow {
    pub id: String,
    pub workspace_id: String,
    pub sequence_id: String,
    pub contact_id: String,
    pub contact_inbox_id: String,
    pub step_id: String,
    pub enrollment_id: String,
    pub bucket: u32,
    pub run_at_ms: i64,
    pub idempotency_key: String,
}

/// `insert(sequenceDispatchModel).values(..).returning(..)`: `Ok(None)` is
/// an empty `RETURNING` (the TypeScript `[]`).
pub trait DispatchInserter: Send + Sync {
    fn insert_returning(
        &self,
        conn: &Connection,
        row: &NewDispatchRow,
    ) -> Result<Option<DispatchRecord>, SequenceError>;
}

/// The SQL insert (UNIQUE idempotency key: a duplicate is an error).
#[derive(Debug, Clone, Copy, Default)]
pub struct SqlDispatchInserter;

impl DispatchInserter for SqlDispatchInserter {
    fn insert_returning(
        &self,
        conn: &Connection,
        row: &NewDispatchRow,
    ) -> Result<Option<DispatchRecord>, SequenceError> {
        Ok(conn
            .prepare_cached(
                r#"INSERT INTO "SequenceDispatch" ("id", "workspaceId", "sequenceId", "contactId",
                   "contactInboxId", "stepId", "enrollmentId", "runAtMs", "bucket", "idempotencyKey",
                   "status", "attempt")
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'pending', 0)
                   RETURNING "id", "bucket", "runAtMs""#,
            )?
            .query_row(
                params![
                    parse_id(&row.id)?,
                    parse_id(&row.workspace_id)?,
                    parse_id(&row.sequence_id)?,
                    parse_id(&row.contact_id)?,
                    parse_id(&row.contact_inbox_id)?,
                    parse_id(&row.step_id)?,
                    parse_id(&row.enrollment_id)?,
                    row.run_at_ms,
                    row.bucket,
                    row.idempotency_key,
                ],
                |r| {
                    Ok(DispatchRecord {
                        id: r.get::<_, i64>(0)?.to_string(),
                        bucket: r.get(1)?,
                        run_at_ms: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }
}

/// `createDispatch({ ..., client })` on a caller connection: builds the row
/// (id from `ids`, bucket, idempotency key) and inserts it with `inserter`.
/// No transaction of its own. Empty `RETURNING` = `Failed to create dispatch`.
pub fn create_dispatch_in(
    conn: &Connection,
    inserter: &dyn DispatchInserter,
    ids: &dyn IdGenerator,
    params: &CreateDispatchParams,
) -> Result<DispatchRecord, SequenceError> {
    let row = NewDispatchRow {
        id: ids.next_id(),
        workspace_id: params.workspace_id.clone(),
        sequence_id: params.sequence_id.clone(),
        contact_id: params.contact_id.clone(),
        contact_inbox_id: params.contact_inbox_id.clone(),
        step_id: params.step_id.clone(),
        enrollment_id: params.enrollment_id.clone(),
        bucket: calculate_bucket(&params.workspace_id, &params.contact_id),
        run_at_ms: params.run_at_ms,
        idempotency_key: generate_idempotency_key(
            &params.workspace_id,
            &params.enrollment_id,
            &params.step_id,
            params.run_at_ms,
        ),
    };
    inserter
        .insert_returning(conn, &row)?
        .ok_or_else(|| SequenceError::Other("Failed to create dispatch".to_owned()))
}

/// `createDispatch(params)` without a client (`ClientMode::Default`) or with
/// the root `db` (`ClientMode::Root`): one insert on `ctx.db` with
/// `ctx.inserter` and `ctx.ids`; never a wrapping transaction.
pub async fn create_dispatch(
    ctx: &SequenceCtx,
    params: CreateDispatchParams,
    mode: ClientMode,
) -> Result<DispatchRecord, SequenceError> {
    // One statement on the default or the root client: no transaction of
    // its own in either mode.
    let _ = mode;
    let inserter = Arc::clone(&ctx.inserter);
    let ids = Arc::clone(&ctx.ids);
    write_tx(&ctx.db, move |conn| {
        create_dispatch_in(conn, inserter.as_ref(), ids.as_ref(), &params)
    })
    .await
}
