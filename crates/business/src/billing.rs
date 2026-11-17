//! `enterprise/billing/service.ts` (`billingService.provisionDefaultPlan`,
//! Q.D11 slice 10): asks the private billing portal (same-origin
//! `/portal/*` proxy of the builder) to provision a new user's default plan.

use std::time::Duration;

use crate::keys::Edition;

/// `PROVISION_PATH`.
const PROVISION_PATH: &str = "/portal/api/users/provision";
/// `PROVISION_TIMEOUT_MS`.
const PROVISION_TIMEOUT: Duration = Duration::from_millis(5000);

/// What [`provision_default_plan`] did (the original returns nothing and
/// only logs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Provision {
    /// Not cloud: no request.
    Skipped,
    /// The portal answered with this status (non-2xx is logged).
    Answered(u16),
    /// The request failed (logged).
    Failed(String),
}

/// `provisionDefaultPlan({ userId, tenantId })`: cloud only, best effort
/// (a failure is logged, never returned as an error).
pub async fn provision_default_plan(
    edition: Edition,
    builder_url: &str,
    user_id: i64,
    tenant_id: Option<i64>,
) -> Provision {
    if !edition.is_cloud() {
        return Provision::Skipped;
    }
    let url = format!("{}{PROVISION_PATH}", builder_url.trim_end_matches('/'));
    let mut body = serde_json::json!({ "userId": user_id.to_string() });
    if let Some(tenant) = tenant_id {
        body["tenantId"] = serde_json::json!(tenant.to_string());
    }
    let sent = reqwest::Client::new()
        .post(&url)
        .header("content-type", "application/json")
        .body(body.to_string())
        .timeout(PROVISION_TIMEOUT)
        .send()
        .await;
    match sent {
        Ok(response) => {
            let status = response.status().as_u16();
            if !response.status().is_success() {
                tracing::error!(
                    user_id,
                    tenant_id,
                    status,
                    "billing: default-plan provisioning returned a non-OK status"
                );
            }
            Provision::Answered(status)
        }
        Err(err) => {
            tracing::error!(%err, user_id, tenant_id, "billing: default-plan provisioning request failed");
            Provision::Failed(err.to_string())
        }
    }
}
