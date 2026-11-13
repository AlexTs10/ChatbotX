//! `qr-code/qr-code.service.ts` (`qrCodeService`): `"Reflink"` rows of type
//! `qrCode`, the name stored with a `qr_` prefix and the size in `qrStyles`.
//! No read cache and no template locks in the Rust port.

use chatbotx_db::Db;
use chatbotx_db::repo::models::{Flow, Reflink};
use chatbotx_db::repo::utils::{order_by_sql, parse_order_by};
use chatbotx_db::repo::{
    Model, PaginationInput, SortItem, get_pagination_with_defaults, is_unique_violation_error,
    like_contains, new_id,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, params, params_from_iter};
use serde_json::json;

use crate::errors::{BusinessError, Result};
use crate::reflink::{assert_parent_flows_in_workspace, delete_in_tx, find_typed, update_typed};
use crate::tx;

const KIND: &str = "qrCode";
/// `QR_CODE_SIZE.DEFAULT`.
pub const DEFAULT_SIZE: i64 = 600;

fn not_found() -> BusinessError {
    BusinessError::not_found("QR Code not found")
}

fn db_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `flowService.exists` (private query).
fn flow_exists(conn: &Connection, workspace_id: i64, flow_id: i64) -> Result<bool> {
    conn.query_row(
        r#"SELECT EXISTS (SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2)"#,
        params![flow_id, workspace_id],
        |r| r.get(0),
    )
    .map_err(db_error)
}

/// `CreateQrCodeData` / `UpdateQrCodeData`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QrCodeData {
    pub name: Option<String>,
    pub flow_id: Option<i64>,
    pub custom_field_id: Option<Option<i64>>,
    pub size: Option<i64>,
}

/// `ListQrCodesInput`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListQrCodes {
    pub workspace_id: i64,
    pub page: Option<i64>,
    pub per_page: i64,
    pub sort: Option<Vec<SortItem>>,
    pub keyword: Option<String>,
}

/// One listed QR code with its flow.
#[derive(Debug, Clone, PartialEq)]
pub struct QrCodeRow {
    pub qr_code: Reflink,
    pub flow: Flow,
}

/// `list` result.
#[derive(Debug, Clone, PartialEq)]
pub struct QrCodePage {
    pub data: Vec<QrCodeRow>,
    pub page_count: i64,
}

/// `find`.
pub async fn find(db: &Db, workspace_id: i64, id: i64) -> Result<Option<Reflink>> {
    tx::read(db, move |conn| find_typed(conn, workspace_id, id, KIND)).await
}

/// `findOrFail` (`QR Code not found`).
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<Reflink> {
    find(db, workspace_id, id).await?.ok_or_else(not_found)
}

/// `create`: the new id.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    data: QrCodeData,
    duplicate_name_message: &str,
) -> Result<i64> {
    let duplicate = duplicate_name_message.to_string();
    tx::write(db, move |conn| {
        let flow_id = data.flow_id.unwrap_or_default();
        // `assertReferencable`'s parent check runs before `flowService.exists`.
        assert_parent_flows_in_workspace(conn, workspace_id, &[flow_id])?;
        if !flow_exists(conn, workspace_id, flow_id)? {
            return Err(BusinessError::validation("flowId", "Flow not found in this workspace"));
        }
        let id = new_id();
        let size = data.size.unwrap_or(DEFAULT_SIZE);
        conn.execute(
            r#"INSERT INTO "Reflink" ("id", "workspaceId", "type", "name", "flowId", "customFieldId", "qrStyles")
               VALUES (?1, ?2, 'qrCode', ?3, ?4, ?5, ?6)"#,
            params![
                id,
                workspace_id,
                format!("qr_{}", data.name.unwrap_or_default()),
                flow_id,
                data.custom_field_id.flatten(),
                json!({ "size": size }).to_string()
            ],
        )
        .map_err(|e| {
            let error = chatbotx_db::repo::Error::from(e);
            if is_unique_violation_error(&error, None) {
                BusinessError::validation("name", duplicate.clone())
            } else {
                BusinessError::Repo(error)
            }
        })?;
        Ok(id)
    })
    .await
}

/// `update`.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    data: QrCodeData,
    duplicate_name_message: &str,
) -> Result<Reflink> {
    let duplicate = duplicate_name_message.to_string();
    tx::write(db, move |conn| {
        let stored = find_typed(conn, workspace_id, id, KIND)?.ok_or_else(not_found)?;
        if let Some(flow_id) = data.flow_id
            && !flow_exists(conn, workspace_id, flow_id)?
        {
            return Err(BusinessError::validation(
                "flowId",
                "Flow not found in this workspace",
            ));
        }
        let mut sets: Vec<(&'static str, Sql)> = Vec::new();
        if let Some(name) = &data.name {
            sets.push(("name", Sql::Text(format!("qr_{name}"))));
        }
        if let Some(flow_id) = data.flow_id {
            sets.push(("flowId", Sql::Integer(flow_id)));
        }
        if let Some(custom_field_id) = data.custom_field_id {
            sets.push((
                "customFieldId",
                custom_field_id.map_or(Sql::Null, Sql::Integer),
            ));
        }
        if let Some(size) = data.size {
            let mut styles = stored
                .qr_styles
                .as_ref()
                .and_then(|j| j.0.as_object().cloned())
                .unwrap_or_default();
            styles.insert("size".into(), json!(size));
            sets.push((
                "qrStyles",
                Sql::Text(serde_json::Value::Object(styles).to_string()),
            ));
        }
        update_typed(conn, workspace_id, id, KIND, sets)
            .map_err(|error| {
                if is_unique_violation_error(&error, None) {
                    BusinessError::validation("name", duplicate.clone())
                } else {
                    BusinessError::Repo(error)
                }
            })?
            .ok_or_else(not_found)
    })
    .await
}

/// `deleteMany` (QR codes only).
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<()> {
    tx::write(db, move |conn| {
        delete_in_tx(conn, workspace_id, &ids, Some(KIND)).map(|_| ())
    })
    .await
}

/// `list`: QR codes with their flow, paged.
pub async fn list(db: &Db, input: ListQrCodes) -> Result<QrCodePage> {
    tx::read(db, move |conn| {
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: Some(input.per_page),
        });
        let keyword = input.keyword.as_deref().filter(|k| !k.is_empty()).map(like_contains);
        let where_sql = "r.\"workspaceId\" = ?1 AND r.\"type\" = 'qrCode' AND (?2 IS NULL OR ilike(r.\"name\", ?2))";
        let order = parse_order_by(Reflink::COLUMNS, input.sort.as_deref(), None);
        let sql = format!(
            "SELECT {}, {} FROM \"Reflink\" r JOIN \"Flow\" f ON f.\"id\" = r.\"flowId\" WHERE {where_sql} {} LIMIT ?3 OFFSET ?4",
            Reflink::columns_sql(Some("r")),
            Flow::columns_sql(Some("f")),
            order_by_sql(&order, Some("r")),
        );
        let n = Reflink::COLUMNS.len();
        let mut stmt = conn.prepare(&sql).map_err(db_error)?;
        let args = vec![
            Sql::Integer(input.workspace_id),
            keyword.clone().map_or(Sql::Null, Sql::Text),
            Sql::Integer(pagination.limit),
            Sql::Integer(pagination.offset),
        ];
        let data = stmt
            .query_map(params_from_iter(args), |row| {
                Ok(QrCodeRow {
                    qr_code: Reflink::from_row(row)?,
                    flow: Flow::from_row_at(row, n)?,
                })
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        let total: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM \"Reflink\" r WHERE {where_sql}"),
                params![input.workspace_id, keyword],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        let per_page = input.per_page.max(1);
        Ok(QrCodePage {
            data,
            page_count: (total + per_page - 1) / per_page,
        })
    })
    .await
}
