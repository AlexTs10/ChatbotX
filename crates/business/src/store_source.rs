//! `ecommerce-settings/store-source.ts` (`resolveStoreSource`) and
//! `external-store/errors.ts` (`externalStoreManagedException`). The original
//! caches the source under the settings tag; the Rust port reads it each time.

use chatbotx_db::rusqlite::Connection;

use crate::errors::{BusinessError, Result};

/// `resolveStoreSource`: `EcommerceSettings.storeSource`, `chatbotx` without a row.
pub fn resolve_store_source(conn: &Connection, workspace_id: i64) -> Result<String> {
    let _ = (conn, workspace_id);
    todo!("P3.6-D1")
}

/// `isExternalStoreSource`: `shopify` or `woocommerce`.
pub fn is_external_store_source(source: &str) -> bool {
    let _ = source;
    todo!("P3.6-D1")
}

/// `externalStoreManagedException(source)` (409, `data.store`).
pub fn external_store_managed(source: &str) -> BusinessError {
    let _ = source;
    todo!("P3.6-D1")
}
