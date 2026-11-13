//! The two `discountCodeRepository` writes the catalog services run in their
//! delete transactions (private queries; they move to `crates/db` later).

use chatbotx_db::rusqlite::Connection;

use crate::errors::{BusinessError, Result};

fn id_list(ids: &[i64]) -> String {
    serde_json::Value::from(ids.iter().map(|id| id.to_string()).collect::<Vec<_>>()).to_string()
}

/// `deactivateByCategoryIds`: switches off the `category` codes of these categories.
pub fn deactivate_by_category_ids(
    conn: &Connection,
    workspace_id: i64,
    category_ids: &[i64],
) -> Result<()> {
    if category_ids.is_empty() {
        return Ok(());
    }
    conn.execute(
        r#"UPDATE "DiscountCode" SET "isActive" = 0 WHERE "workspaceId" = ?1 AND "appliesTo" = 'category'
           AND "categoryId" IN (SELECT CAST(value AS INTEGER) FROM json_each(?2))"#,
        chatbotx_db::rusqlite::params![workspace_id, id_list(category_ids)],
    )
    .map_err(|e| BusinessError::Repo(e.into()))?;
    Ok(())
}

/// `removeProductIds`: drops the ids from `products` codes; a code left with
/// no product is switched off.
pub fn remove_product_ids(conn: &Connection, workspace_id: i64, product_ids: &[i64]) -> Result<()> {
    if product_ids.is_empty() {
        return Ok(());
    }
    // `productIds` holds the ids as JSON strings (or numbers); compare as text.
    conn.execute(
        r#"UPDATE "DiscountCode" SET
             "productIds" = coalesce((SELECT json_group_array(l.value) FROM json_each("DiscountCode"."productIds") l
                                      WHERE CAST(l.value AS TEXT) NOT IN (SELECT value FROM json_each(?2))), '[]'),
             "isActive" = CASE WHEN EXISTS (SELECT 1 FROM json_each("DiscountCode"."productIds") l
                                      WHERE CAST(l.value AS TEXT) NOT IN (SELECT value FROM json_each(?2)))
                               THEN "isActive" ELSE 0 END
           WHERE "workspaceId" = ?1 AND "appliesTo" = 'products'
             AND EXISTS (SELECT 1 FROM json_each("DiscountCode"."productIds") l
                         WHERE CAST(l.value AS TEXT) IN (SELECT value FROM json_each(?2)))"#,
        chatbotx_db::rusqlite::params![workspace_id, id_list(product_ids)],
    )
    .map_err(|e| BusinessError::Repo(e.into()))?;
    Ok(())
}
