//! Port of `repositories/meta-catalog-item/repository.ts` (P3.3). `lockCatalogAssignments`
//! (`pg_advisory_xact_lock`) is a no-op under the one SQLite writer.

use rusqlite::Connection;

use super::models::MetaCatalogItem;
use super::sql::Sql;
use super::utils::{PaginationInput, get_pagination_with_defaults, like_contains};
use super::{Json, Model, Result, id_list, new_id, now_ms, text_list};

const DEFAULT_SEARCH_LIMIT: i64 = 20;

/// `CatalogScope`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogScope {
    pub integration_meta_catalog_id: i64,
    pub catalog_id: String,
}

/// A linked product (`sendableProductColumns`).
#[derive(Debug, Clone, PartialEq)]
pub struct SendableProduct {
    pub product_id: i64,
    pub retailer_id: String,
    pub catalog_id: String,
    pub name: String,
    pub images: Json,
    pub price: f64,
    pub currency: String,
}

/// Input of [`list_sendable_products`].
#[derive(Debug, Clone, PartialEq)]
pub struct SendableListInput {
    pub scope: CatalogScope,
    pub workspace_id: i64,
    pub keyword: Option<String>,
    pub product_ids: Option<Vec<i64>>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// Result of [`list_sendable_products`].
#[derive(Debug, Clone, PartialEq)]
pub struct SendablePage {
    pub data: Vec<SendableProduct>,
    pub page_count: i64,
}

/// One row of [`search_with_product_info`].
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogSearchRow {
    pub retailer_id: String,
    pub name: String,
    pub images: Json,
}

/// Result of [`find_latest_by_product_id`].
#[derive(Debug, Clone, PartialEq)]
pub struct LatestCatalogLink {
    pub catalog_id: String,
    pub retailer_id: String,
    pub direction: String,
    pub last_synced_at: Option<i64>,
}

/// One `{ productId, retailerId, fingerprint? }` item of [`link_imported`] / [`mark_succeeded`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogLinkItem {
    pub product_id: i64,
    pub retailer_id: String,
    pub fingerprint: Option<String>,
}

fn in_catalog(scope: &CatalogScope) -> Sql {
    let mut s = Sql::bind(
        "i.\"integrationMetaCatalogId\" = ",
        scope.integration_meta_catalog_id,
    );
    s.push(" AND i.\"catalogId\" = ");
    s.push_bind(scope.catalog_id.clone());
    s
}

const LINKED_FROM: &str = "FROM \"MetaCatalogItem\" i JOIN \"Product\" p ON i.\"productId\" = p.\"id\" \
     JOIN \"IntegrationMetaCatalog\" c ON i.\"integrationMetaCatalogId\" = c.\"id\" WHERE ";
const SENDABLE_COLUMNS: &str = "SELECT p.\"id\", i.\"retailerId\", i.\"catalogId\", p.\"name\", p.\"images\", p.\"price\", p.\"currency\" ";

fn linked_where(scope: &CatalogScope, workspace_id: i64) -> Sql {
    let mut s = in_catalog(scope);
    s.push(" AND c.\"workspaceId\" = ");
    s.push_bind(workspace_id);
    s.push(" AND c.\"catalogId\" = ");
    s.push_bind(scope.catalog_id.clone());
    s.push(" AND c.\"deletedAt\" IS NULL AND p.\"workspaceId\" = ");
    s.push_bind(workspace_id);
    s
}

fn sendable_where(
    scope: &CatalogScope,
    workspace_id: i64,
    keyword: Option<&str>,
    product_ids: Option<&[i64]>,
) -> Sql {
    Sql::and([
        Some(linked_where(scope, workspace_id)),
        Some(Sql::raw("p.\"status\" = 'active'")),
        keyword
            .filter(|k| !k.is_empty())
            .map(|k| Sql::bind("ilike(p.\"name\", ", like_contains(k)).closed(")")),
        product_ids.map(|ids| {
            Sql::bind(
                "i.\"productId\" IN (SELECT value FROM json_each(",
                id_list(ids),
            )
            .closed("))")
        }),
    ])
}

fn sendable(r: &rusqlite::Row<'_>) -> rusqlite::Result<SendableProduct> {
    Ok(SendableProduct {
        product_id: r.get(0)?,
        retailer_id: r.get(1)?,
        catalog_id: r.get(2)?,
        name: r.get(3)?,
        images: r.get(4)?,
        price: r.get(5)?,
        currency: r.get(6)?,
    })
}

/// `lockCatalogAssignments`: no-op (one SQLite writer).
pub fn lock_catalog_assignments(_conn: &Connection, _scope: &CatalogScope) -> Result<()> {
    todo!("P3.3")
}

/// `findLatestByProductId`: last synced first (NULLs last), then newest.
pub fn find_latest_by_product_id(
    conn: &Connection,
    product_id: i64,
) -> Result<Option<LatestCatalogLink>> {
    todo!("P3.3")
}

fn items_where(
    conn: &Connection,
    scope: &CatalogScope,
    col: &str,
    list: String,
) -> Result<Vec<MetaCatalogItem>> {
    let mut q = Sql::raw(format!(
        "SELECT {} FROM \"MetaCatalogItem\" i WHERE ",
        MetaCatalogItem::columns_sql(Some("i"))
    ));
    q.append(in_catalog(scope));
    q.push(&format!(
        " AND i.\"{col}\" IN (SELECT value FROM json_each("
    ));
    q.push_bind(list);
    q.push("))");
    q.models(conn)
}

/// `findByRetailerIds`.
pub fn find_by_retailer_ids(
    conn: &Connection,
    scope: &CatalogScope,
    retailer_ids: &[&str],
) -> Result<Vec<MetaCatalogItem>> {
    todo!("P3.3")
}

/// `searchWithProductInfo`: by product name; `limit` defaults to 20.
pub fn search_with_product_info(
    conn: &Connection,
    scope: &CatalogScope,
    keyword: Option<&str>,
    retailer_id: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<CatalogSearchRow>> {
    todo!("P3.3")
}

/// `listSendableProducts`: active linked products by name, id; an empty `productIds` list
/// matches nothing.
pub fn list_sendable_products(
    conn: &Connection,
    input: &SendableListInput,
) -> Result<SendablePage> {
    todo!("P3.3")
}

/// `findSendableByProductIds`.
pub fn find_sendable_by_product_ids(
    conn: &Connection,
    scope: &CatalogScope,
    workspace_id: i64,
    product_ids: &[i64],
) -> Result<Vec<SendableProduct>> {
    todo!("P3.3")
}

/// `findLinkedProductsByRetailerIds` (inactive products too).
pub fn find_linked_products_by_retailer_ids(
    conn: &Connection,
    scope: &CatalogScope,
    workspace_id: i64,
    retailer_ids: &[&str],
) -> Result<Vec<SendableProduct>> {
    todo!("P3.3")
}

/// `findByProductIds`.
pub fn find_by_product_ids(
    conn: &Connection,
    scope: &CatalogScope,
    product_ids: &[i64],
) -> Result<Vec<MetaCatalogItem>> {
    todo!("P3.3")
}

fn clear_stale_product_links(
    conn: &Connection,
    scope: &CatalogScope,
    items: &[CatalogLinkItem],
) -> Result<()> {
    let mut stmt = conn.prepare_cached(
        "DELETE FROM \"MetaCatalogItem\" WHERE \"integrationMetaCatalogId\" = ?1 AND \"catalogId\" = ?2 AND \
         ((\"productId\" = ?3 AND \"retailerId\" <> ?4) OR (\"retailerId\" = ?4 AND \"productId\" <> ?3))",
    )?;
    for item in items {
        stmt.execute(rusqlite::params![
            scope.integration_meta_catalog_id,
            scope.catalog_id,
            item.product_id,
            item.retailer_id
        ])?;
    }
    Ok(())
}

fn upsert_links(
    conn: &Connection,
    scope: &CatalogScope,
    items: &[CatalogLinkItem],
    push: bool,
) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    lock_catalog_assignments(conn, scope)?;
    clear_stale_product_links(conn, scope, items)?;
    let now = now_ms();
    let sql = if push {
        "INSERT INTO \"MetaCatalogItem\" (\"id\", \"createdAt\", \"updatedAt\", \"integrationMetaCatalogId\", \"catalogId\", \
         \"direction\", \"productId\", \"retailerId\", \"lastSyncedFingerprint\", \"lastSyncedAt\") \
         VALUES (?1, ?2, ?2, ?3, ?4, 'push', ?5, ?6, ?7, ?2) \
         ON CONFLICT (\"integrationMetaCatalogId\", \"catalogId\", \"retailerId\") DO UPDATE SET \
         \"productId\" = excluded.\"productId\", \"direction\" = excluded.\"direction\", \
         \"lastSyncedFingerprint\" = excluded.\"lastSyncedFingerprint\", \"lastSyncedAt\" = ?2, \"updatedAt\" = ?2"
    } else {
        "INSERT INTO \"MetaCatalogItem\" (\"id\", \"createdAt\", \"updatedAt\", \"integrationMetaCatalogId\", \"catalogId\", \
         \"direction\", \"productId\", \"retailerId\") VALUES (?1, ?2, ?2, ?3, ?4, 'import', ?5, ?6) \
         ON CONFLICT (\"integrationMetaCatalogId\", \"catalogId\", \"retailerId\") DO UPDATE SET \
         \"productId\" = excluded.\"productId\", \"direction\" = excluded.\"direction\", \"updatedAt\" = ?2"
    };
    let mut stmt = conn.prepare_cached(sql)?;
    for item in items {
        if push {
            stmt.execute(rusqlite::params![
                new_id(),
                now,
                scope.integration_meta_catalog_id,
                scope.catalog_id,
                item.product_id,
                item.retailer_id,
                item.fingerprint
            ])?;
        } else {
            stmt.execute(rusqlite::params![
                new_id(),
                now,
                scope.integration_meta_catalog_id,
                scope.catalog_id,
                item.product_id,
                item.retailer_id
            ])?;
        }
    }
    Ok(())
}

/// `linkImported`: `import` links; a product / retailer id keeps one link per catalog.
pub fn link_imported(
    conn: &Connection,
    scope: &CatalogScope,
    items: &[CatalogLinkItem],
) -> Result<()> {
    todo!("P3.3")
}

/// `markSucceeded`: `push` links with the synced fingerprint and time.
pub fn mark_succeeded(
    conn: &Connection,
    scope: &CatalogScope,
    items: &[CatalogLinkItem],
) -> Result<()> {
    todo!("P3.3")
}
