//! `packages/business/src/ai-function/service.ts`: the AI functions of a
//! workspace (`"AIFunction"`). No template edit locks / delete guards
//! (P3.6-D1 rule); the `assertReferencable` flow check is kept.

use chatbotx_db::Db;
use chatbotx_db::repo::models::AiFunction;
use chatbotx_db::repo::utils::{PaginationInput, get_pagination_with_defaults};
use chatbotx_db::repo::{Json, Model, find_many, find_one, new_id, now_ms, returning};
use chatbotx_db::rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::audit::AuditService;
use crate::errors::{BusinessError, Result};
use crate::reflink::assert_parent_flows_in_workspace;
use crate::tx;

fn repo_err(error: impl Into<chatbotx_db::repo::Error>) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// One `dataCollect` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataCollect {
    pub from: String,
    pub to: String,
}

/// `CreateAIFunctionRequest` / `UpdateAIFunctionRequest`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AiFunctionInput {
    pub name: String,
    pub purpose: Option<String>,
    pub data_collect: Vec<DataCollect>,
    pub output_message: Option<String>,
    pub trigger_flow_id: Option<i64>,
}

impl AiFunctionInput {
    fn data_collect_json(&self) -> Value {
        Value::Array(
            self.data_collect
                .iter()
                .map(|d| json!({ "from": d.from, "to": d.to }))
                .collect(),
        )
    }

    /// The request as the stored row compares (`isSameJsonValue`).
    fn same_as(&self, row: &AiFunction) -> bool {
        row.name == self.name
            && row.purpose == self.purpose
            && row.data_collect.as_ref().map(|j| &j.0) == Some(&self.data_collect_json())
            && row.output_message == self.output_message
            && row.trigger_flow_id == self.trigger_flow_id
    }
}

/// `listAIFunctions` result.
#[derive(Debug, Clone, PartialEq)]
pub struct AiFunctionPage {
    pub data: Vec<AiFunction>,
    pub page_count: i64,
}

/// `findBy({ id, workspaceId })`.
pub fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<Option<AiFunction>> {
    find_one(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&id, &workspace_id],
    )
    .map_err(repo_err)
}

/// `findBy`.
pub async fn find(db: &Db, workspace_id: i64, id: i64) -> Result<Option<AiFunction>> {
    tx::read(db, move |conn| find_in(conn, workspace_id, id)).await
}

/// `isNameTaken`.
fn is_name_taken(
    conn: &Connection,
    workspace_id: i64,
    name: &str,
    exclude_id: Option<i64>,
) -> Result<bool> {
    let existing: Option<AiFunction> = find_one(
        conn,
        r#""workspaceId" = ?1 AND "name" = ?2"#,
        &[&workspace_id, &name],
    )
    .map_err(repo_err)?;
    Ok(existing.is_some_and(|row| Some(row.id) != exclude_id))
}

/// `listAIFunctions`: newest first; without `page` / `perPage` every row
/// and `pageCount: 1`.
pub async fn list(db: &Db, workspace_id: i64, paging: PaginationInput) -> Result<AiFunctionPage> {
    tx::read(db, move |conn| {
        let order = r#"WHERE "workspaceId" = ?1 ORDER BY "createdAt" DESC, "id" DESC"#;
        if paging.page.is_none() && paging.per_page.is_none() {
            let data = find_many(conn, order, &[&workspace_id]).map_err(repo_err)?;
            return Ok(AiFunctionPage {
                data,
                page_count: 1,
            });
        }
        let p = get_pagination_with_defaults(paging);
        let data = find_many(
            conn,
            &format!("{order} LIMIT ?2 OFFSET ?3"),
            &[&workspace_id, &p.limit, &p.offset],
        )
        .map_err(repo_err)?;
        let total: i64 = conn
            .query_row(
                r#"SELECT count(*) FROM "AIFunction" WHERE "workspaceId" = ?1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        Ok(AiFunctionPage {
            data,
            page_count: (total + p.limit - 1) / p.limit,
        })
    })
    .await
}

/// `create` (no transaction given): name check, flow check, insert, audit.
pub async fn create(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    input: AiFunctionInput,
) -> Result<AiFunction> {
    let created = tx::write(db, move |tx| {
        if is_name_taken(tx, workspace_id, &input.name, None)? {
            return Err(BusinessError::validation("name", "Name is already taken"));
        }
        create_in(tx, workspace_id, &input)
    })
    .await?;
    audit
        .audit(
            "create",
            &format!("created a new AI Function (#{})", created.id),
        )
        .await;
    Ok(created)
}

/// `create(workspaceId, data, tx)`: the insert inside a caller's
/// transaction (no name check, no audit).
pub fn create_in(
    conn: &Connection,
    workspace_id: i64,
    input: &AiFunctionInput,
) -> Result<AiFunction> {
    if let Some(flow_id) = input.trigger_flow_id {
        assert_parent_flows_in_workspace(conn, workspace_id, &[flow_id])?;
    }
    let now = now_ms();
    let sql = format!(
        r#"INSERT INTO {} ("id", "createdAt", "updatedAt", "name", "purpose", "dataCollect", "outputMessage", "triggerFlowId", "workspaceId")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
        AiFunction::TABLE
    );
    returning::<AiFunction>(
        conn,
        &sql,
        &[
            &new_id(),
            &now,
            &input.name,
            &input.purpose,
            &Json(input.data_collect_json()),
            &input.output_message,
            &input.trigger_flow_id,
            &workspace_id,
        ],
    )
    .map_err(repo_err)?
    .pop()
    .ok_or_else(|| BusinessError::not_found("AI Function not found"))
}

/// `updateAIFunction`: 404 `AI Function not found`, name check, flow check,
/// write; audits when a field changed.
pub async fn update(
    db: &Db,
    audit: &AuditService,
    workspace_id: i64,
    id: i64,
    input: AiFunctionInput,
) -> Result<AiFunction> {
    let (updated, changed) = tx::write(db, move |tx| {
        let existing = find_in(tx, workspace_id, id)?
            .ok_or_else(|| BusinessError::not_found("AI Function not found"))?;
        if is_name_taken(tx, workspace_id, &input.name, Some(id))? {
            return Err(BusinessError::validation("name", "Name is already taken"));
        }
        if let Some(flow_id) = input.trigger_flow_id {
            assert_parent_flows_in_workspace(tx, workspace_id, &[flow_id])?;
        }
        let changed = !input.same_as(&existing);
        let sql = format!(
            r#"UPDATE {} SET "name" = ?1, "purpose" = ?2, "dataCollect" = ?3, "outputMessage" = ?4,
                 "triggerFlowId" = ?5, "updatedAt" = ?6
               WHERE "id" = ?7 AND "workspaceId" = ?8"#,
            AiFunction::TABLE
        );
        let row = returning::<AiFunction>(
            tx,
            &sql,
            &[
                &input.name,
                &input.purpose,
                &Json(input.data_collect_json()),
                &input.output_message,
                &input.trigger_flow_id,
                &now_ms(),
                &id,
                &workspace_id,
            ],
        )
        .map_err(repo_err)?
        .pop()
        .ok_or_else(|| BusinessError::not_found("AI Function not found"))?;
        Ok((row, changed))
    })
    .await?;
    if changed {
        audit
            .audit(
                "update",
                &format!("updated an AI Function (#{})", updated.id),
            )
            .await;
    }
    Ok(updated)
}

/// `deleteAIFunction`: 404 `AI Function not found`, delete, audit.
pub async fn delete(db: &Db, audit: &AuditService, workspace_id: i64, id: i64) -> Result<()> {
    tx::write(db, move |tx| {
        if find_in(tx, workspace_id, id)?.is_none() {
            return Err(BusinessError::not_found("AI Function not found"));
        }
        delete_in(tx, workspace_id, &[id])?;
        Ok(())
    })
    .await?;
    audit
        .audit("delete", &format!("deleted an AI Function (#{id})"))
        .await;
    Ok(())
}

/// `deleteInTx`: the deleted ids of `ids` in the workspace.
pub fn delete_in(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
    let mut out = Vec::new();
    for id in ids {
        let n = conn
            .execute(
                r#"DELETE FROM "AIFunction" WHERE "workspaceId" = ?1 AND "id" = ?2"#,
                params![workspace_id, id],
            )
            .map_err(repo_err)?;
        if n > 0 {
            out.push(*id);
        }
    }
    Ok(out)
}
