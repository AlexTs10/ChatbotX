//! `integration-api/service.ts` (`createWithToken`, `connect`) and the
//! `api` connect adapter of `packages/connections/src/registry.ts`.

use chatbotx_db::Db;

use crate::Result;
use crate::connection::ConnectionRow;
use crate::net::ssrf_guard::DohFetch;

/// `CreateApiWithTokenInput` (an existing workspace).
#[derive(Debug, Clone)]
pub struct CreateApiChannel {
    pub workspace_id: i64,
    /// `actorUserId` (`Connection.createdBy`); a token actor has none.
    pub actor_user_id: Option<i64>,
    pub name: String,
    pub callback_url: Option<String>,
}

/// `integrationApiService.createWithToken`: the SSRF check of `callbackUrl`
/// (`422 invalidRequestData`), a new token, the inbox, the `IntegrationApi`
/// row and the `connected` connection in one transaction. Returns the
/// connection and the one-time token.
pub async fn create_with_token(
    _db: &Db,
    _input: CreateApiChannel,
    _doh: &impl DohFetch,
    _now: i64,
) -> Result<(ConnectionRow, String)> {
    todo!("P3.6-D2")
}
