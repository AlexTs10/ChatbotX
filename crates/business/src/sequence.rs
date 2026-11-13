//! `packages/business/src/sequence/service.ts` (P3.6-D2 slice contact
//! sequences): `create` only. Private queries on `"Sequence"`, `"Folder"`.
//! No audit record.

use chatbotx_db::Db;

use crate::errors::Result;

/// `create`: a `folderId` must be a sequence folder of the workspace (404
/// "Folder not found"); a taken name is a `name` validation error.
pub async fn create(db: &Db, workspace_id: i64, name: &str, folder_id: Option<i64>) -> Result<i64> {
    let _ = (db, workspace_id, name, folder_id);
    todo!("P3.6-D2")
}
