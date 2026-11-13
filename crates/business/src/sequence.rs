//! `packages/business/src/sequence/service.ts` (P3.6-D2 slice contact
//! sequences): `create` only. Private queries on `"Sequence"`, `"Folder"`.
//! No audit record.

use chatbotx_db::repo::{self, new_id};
use chatbotx_db::rusqlite::{OptionalExtension, params};
use chatbotx_db::{Db, DbError};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `create`: a `folderId` must be a sequence folder of the workspace (404
/// "Folder not found"); a taken name is a `name` validation error.
pub async fn create(db: &Db, workspace_id: i64, name: &str, folder_id: Option<i64>) -> Result<i64> {
    let name = name.to_string();
    tx::write(db, move |tx| {
        if let Some(folder_id) = folder_id {
            let found: Option<i64> = tx
                .query_row(
                    r#"SELECT 1 FROM "Folder" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "folderType" = 'sequence'"#,
                    params![folder_id, workspace_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(repo::Error::from)?;
            if found.is_none() {
                return Err(BusinessError::not_found("Folder not found"));
            }
        }
        let id = new_id();
        match tx.execute(
            r#"INSERT INTO "Sequence" ("id", "name", "folderId", "workspaceId") VALUES (?1, ?2, ?3, ?4)"#,
            params![id, name, folder_id, workspace_id],
        ) {
            Ok(_) => Ok(id),
            Err(error) => {
                let error = DbError::from(error);
                if error.sqlstate() == Some("23505") {
                    Err(BusinessError::validation("name", "Name is already taken."))
                } else {
                    Err(BusinessError::Repo(repo::Error::Db(error)))
                }
            }
        }
    })
    .await
}
