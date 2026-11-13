//! `ecommerce-settings/store-source.ts` (`resolveStoreSource`) and
//! `external-store/errors.ts` (`externalStoreManagedException`). The original
//! caches the source under the settings tag; the Rust port reads it each time.

use chatbotx_db::rusqlite::{Connection, OptionalExtension};
use serde_json::json;

use crate::errors::{BusinessError, Result};

/// `resolveStoreSource`: `EcommerceSettings.storeSource`, `chatbotx` without a row.
pub fn resolve_store_source(conn: &Connection, workspace_id: i64) -> Result<String> {
    let source: Option<String> = conn
        .query_row(
            r#"SELECT "storeSource" FROM "EcommerceSettings" WHERE "workspaceId" = ?1"#,
            [workspace_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))?;
    Ok(source.unwrap_or_else(|| "chatbotx".to_string()))
}

/// `isExternalStoreSource`: `shopify` or `woocommerce`.
pub fn is_external_store_source(source: &str) -> bool {
    matches!(source, "shopify" | "woocommerce")
}

/// `externalStoreManagedException(source)` (409, `data.store`).
pub fn external_store_managed(source: &str) -> BusinessError {
    let name = match source {
        "shopify" => "Shopify",
        "woocommerce" => "WooCommerce",
        other => other,
    };
    BusinessError::Exception {
        message: format!("This store is managed in {name}. Change it in {name} and it syncs here."),
        code: "externalStoreManaged".to_string(),
        status: 409,
        field: None,
        data: Some(json!({ "store": source })),
    }
}
