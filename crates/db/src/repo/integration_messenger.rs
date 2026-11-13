//! Port of `repositories/integration-messenger/repository.ts`
//! (`integrationMessengerRepository`, P3.2b).

use rusqlite::Connection;

use super::Result;
use super::models::IntegrationMessenger;

/// `updateHandoverResumeFlow` (`None` clears the flow); `None` when the integration is
/// not in the workspace.
pub fn update_handover_resume_flow(
    _conn: &Connection,
    _id: i64,
    _workspace_id: i64,
    _handover_resume_flow_id: Option<i64>,
) -> Result<Option<IntegrationMessenger>> {
    todo!("P3.2b")
}
