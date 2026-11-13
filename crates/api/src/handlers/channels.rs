//! `features/integration-api/api/public.ts`: the channel-token routes
//! (`channels.*`). Only `channels.me` is ported (P3.6-D2).

use chatbotx_business::BusinessError;
use serde_json::json;

use super::{ApiState, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &["channels.me"];

/// The authenticated channel (`context.inbox`, `context.workspace`).
#[derive(Debug, Clone, Copy)]
pub struct ChannelCall {
    pub inbox_id: i64,
    pub workspace_id: i64,
}

/// Runs one of [`ROUTES`]; `None` = not ported.
pub async fn handle(state: &ApiState, route_id: &str, call: ChannelCall) -> Option<Reply> {
    match route_id {
        "channels.me" => Some(me(state, call).await),
        _ => None,
    }
}

/// `channels.me`: the inbox and workspace of the token.
async fn me(state: &ApiState, call: ChannelCall) -> Reply {
    let inbox_id = call.inbox_id;
    let name = chatbotx_business::tx::read(&state.db, move |conn| {
        use chatbotx_db::rusqlite::OptionalExtension as _;
        conn.query_row(
            r#"SELECT "name" FROM "Inbox" WHERE "id" = ?1"#,
            [inbox_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))
    })
    .await
    .map_err(|e| business_error(e, &[]))?;
    Ok(json(
        200,
        json!({
            "inboxId": call.inbox_id.to_string(),
            "inboxName": name.unwrap_or_default(),
            "workspaceId": call.workspace_id.to_string(),
        }),
    ))
}
