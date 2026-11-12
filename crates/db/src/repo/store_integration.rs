//! Port of `repositories/shopify/repository.ts` and `repositories/woocommerce/repository.ts`
//! (P3.3): one row per workspace, the same four operations on two tables.

use rusqlite::Connection;
use rusqlite::types::Value;

use super::models::{IntegrationShopify, IntegrationWoocommerce};
use super::patch::{Patch, excluded_clause, insert_clause, set_clause};
use super::sql::Sql;
use super::{Error, Model, Result, new_id, now_ms};

fn upsert<M: Model>(conn: &Connection, workspace_id: i64, values: &Patch) -> Result<M> {
    let now = now_ms();
    let mut all: Patch = vec![
        ("id", Value::Integer(new_id())),
        ("createdAt", Value::Integer(now)),
        ("updatedAt", Value::Integer(now)),
        ("workspaceId", Value::Integer(workspace_id)),
    ];
    all.extend(values.iter().cloned());
    let mut q = Sql::raw(format!("INSERT INTO \"{}\" ", M::TABLE));
    q.append(insert_clause(M::COLUMNS, &all)?);
    q.push(" ON CONFLICT (\"workspaceId\") DO UPDATE SET \"updatedAt\" = excluded.\"updatedAt\"");
    if !values.is_empty() {
        q.push(", ");
        q.push(&excluded_clause(values));
    }
    q.push(&format!(" RETURNING {}", M::columns_sql(None)));
    q.model::<M>(conn)?
        .ok_or_else(|| Error::msg(format!("Failed to upsert {}", M::TABLE)))
}

fn update<M: Model>(conn: &Connection, workspace_id: i64, values: &Patch) -> Result<Option<M>> {
    let mut q = Sql::raw(format!("UPDATE \"{}\" SET ", M::TABLE));
    let mut all = values.clone();
    all.push(("updatedAt", Value::Integer(now_ms())));
    q.append(set_clause(M::COLUMNS, &all)?);
    q.push(" WHERE \"workspaceId\" = ");
    q.push_bind(workspace_id);
    q.push(&format!(" RETURNING {}", M::columns_sql(None)));
    q.model::<M>(conn)
}

fn delete(conn: &Connection, table: &str, workspace_id: i64) -> Result<()> {
    conn.execute(
        &format!("DELETE FROM \"{table}\" WHERE \"workspaceId\" = ?1"),
        [workspace_id],
    )?;
    Ok(())
}

/// `shopifyRepository.findIntegration`.
pub fn find_shopify(conn: &Connection, workspace_id: i64) -> Result<Option<IntegrationShopify>> {
    super::find_one(conn, "\"workspaceId\" = ?1", &[&workspace_id])
}

/// `shopifyRepository.upsertIntegration` (`values` = the columns besides id, timestamps
/// and workspace).
pub fn upsert_shopify(
    conn: &Connection,
    workspace_id: i64,
    values: &Patch,
) -> Result<IntegrationShopify> {
    upsert(conn, workspace_id, values)
}

/// `shopifyRepository.updateIntegration`.
pub fn update_shopify(
    conn: &Connection,
    workspace_id: i64,
    values: &Patch,
) -> Result<Option<IntegrationShopify>> {
    update(conn, workspace_id, values)
}

/// `shopifyRepository.deleteIntegration`.
pub fn delete_shopify(conn: &Connection, workspace_id: i64) -> Result<()> {
    delete(conn, "IntegrationShopify", workspace_id)
}

/// `woocommerceRepository.findIntegration`.
pub fn find_woocommerce(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Option<IntegrationWoocommerce>> {
    super::find_one(conn, "\"workspaceId\" = ?1", &[&workspace_id])
}

/// `woocommerceRepository.upsertIntegration`.
pub fn upsert_woocommerce(
    conn: &Connection,
    workspace_id: i64,
    values: &Patch,
) -> Result<IntegrationWoocommerce> {
    upsert(conn, workspace_id, values)
}

/// `woocommerceRepository.updateIntegration`.
pub fn update_woocommerce(
    conn: &Connection,
    workspace_id: i64,
    values: &Patch,
) -> Result<Option<IntegrationWoocommerce>> {
    update(conn, workspace_id, values)
}

/// `woocommerceRepository.deleteIntegration`.
pub fn delete_woocommerce(conn: &Connection, workspace_id: i64) -> Result<()> {
    delete(conn, "IntegrationWoocommerce", workspace_id)
}
