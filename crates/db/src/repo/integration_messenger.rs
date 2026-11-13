//! Port of `repositories/integration-messenger/repository.ts`
//! (`integrationMessengerRepository`, P3.2b).

use rusqlite::Connection;

use super::Result;
use super::models::IntegrationMessenger;

/// `updateHandoverResumeFlow` (`None` clears the flow); `None` when the integration is
/// not in the workspace.
pub fn update_handover_resume_flow(
    conn: &Connection,
    id: i64,
    workspace_id: i64,
    handover_resume_flow_id: Option<i64>,
) -> Result<Option<IntegrationMessenger>> {
    super::integration_whatsapp::update_flow(
        "IntegrationMessenger",
        conn,
        id,
        workspace_id,
        handover_resume_flow_id,
    )
}
