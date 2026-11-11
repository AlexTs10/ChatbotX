//! `dispatch-manager.ts`: bucket hash, idempotency key and `createDispatch`.

use chatbotx_db::rusqlite::Connection;

use crate::sequence::repo::{ClientMode, IdGenerator, SequenceCtx, SequenceError};

/// `calculateBucket(workspaceId, contactId)`: base-31 hash over UTF-16 code
/// units, mod 2 147 483 647, then `% 256` (0..=255). Persisted on the row.
pub fn calculate_bucket(workspace_id: &str, contact_id: &str) -> u32 {
    todo!()
}

/// `generateIdempotencyKey`: `<ws>:<enrollment>:<step>:<runAt.toISOString()>`
/// (ISO with milliseconds and `Z`).
pub fn generate_idempotency_key(
    workspace_id: &str,
    enrollment_id: &str,
    step_id: &str,
    run_at_ms: i64,
) -> String {
    todo!()
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
        todo!()
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
    todo!()
}

/// `createDispatch(params)` without a client (`ClientMode::Default`) or with
/// the root `db` (`ClientMode::Root`): one insert on `ctx.db` with
/// `ctx.inserter` and `ctx.ids`; never a wrapping transaction.
pub async fn create_dispatch(
    ctx: &SequenceCtx,
    params: CreateDispatchParams,
    mode: ClientMode,
) -> Result<DispatchRecord, SequenceError> {
    todo!()
}
