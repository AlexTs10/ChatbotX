//! Port of `repositories/integration-messenger/repository.ts`
//! (`integrationMessengerRepository`, P3.2b).

use rusqlite::Connection;

use super::models::IntegrationMessenger;
use super::{Json, Result};

/// One row of `listPersonasByWorkspaceId`.
#[derive(Debug, Clone, PartialEq)]
pub struct MessengerPersonas {
    pub name: String,
    pub personas: Json,
}

/// `listPersonasByWorkspaceId`: `name` and `personas` of the workspace's pages, by `createdAt`.
pub fn list_personas_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Vec<MessengerPersonas>> {
    let _ = (conn, workspace_id);
    todo!("P3.2b")
}

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
