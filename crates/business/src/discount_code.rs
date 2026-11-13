//! The two `discountCodeRepository` writes the catalog services run in their
//! delete transactions (private queries; they move to `crates/db` later).

use chatbotx_db::rusqlite::Connection;

use crate::errors::Result;

/// `deactivateByCategoryIds`: switches off the `category` codes of these categories.
pub fn deactivate_by_category_ids(
    conn: &Connection,
    workspace_id: i64,
    category_ids: &[i64],
) -> Result<()> {
    let _ = (conn, workspace_id, category_ids);
    todo!("P3.6-D1")
}

/// `removeProductIds`: drops the ids from `products` codes; a code left with
/// no product is switched off.
pub fn remove_product_ids(conn: &Connection, workspace_id: i64, product_ids: &[i64]) -> Result<()> {
    let _ = (conn, workspace_id, product_ids);
    todo!("P3.6-D1")
}
