//! `integration/handlers/{shopify,woocommerce}/index.ts`: the store sync
//! jobs. Each runs behind `withBlockedOwnerGuard`; a failure is retried.

use chatbotx_business::shopify;
use chatbotx_jobs::JobError;
use serde_json::Value;

use crate::order_payment::{bus, order_deps};
use crate::{Deps, fail, is_blocked_workspace, str_of};

pub const SHOPIFY_SYNC: &str = "shopifySync";
pub const SHOPIFY_WEBHOOK: &str = "shopifyWebhook";

fn workspace(data: &Value, kind: &str) -> Result<i64, JobError> {
    crate::id_of(&data["workspaceId"])
        .ok_or_else(|| JobError::unrecoverable(format!("{kind}: bad data")))
}

/// `runShopifySync` / `runShopifyWebhook`.
pub async fn shopify(deps: &Deps, kind: &str, data: &Value) -> Result<(), JobError> {
    let ws = workspace(data, kind)?;
    if is_blocked_workspace(&deps.db, &ws.to_string()).await? {
        return Ok(());
    }
    let events = bus(deps);
    let order = order_deps(deps, &events);
    let outcome = if kind == SHOPIFY_SYNC {
        shopify::sync::run(order, ws, str_of(data, "scope").unwrap_or("all")).await
    } else {
        shopify::sync::process_webhook(
            order,
            ws,
            str_of(data, "topic").unwrap_or_default(),
            str_of(data, "objectId"),
        )
        .await
    };
    outcome.map_err(|e| fail(format!("{kind}: {e}")))
}
