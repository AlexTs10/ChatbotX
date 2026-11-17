//! Email topics: `features/email-topics/api/private.ts` and
//! `features/email-topics/actions/*` (Q.D5).

use chatbotx_business::email_topic::{self, ListEmailTopics};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, page};
use crate::orpc::OrpcError;

/// The ported actions.
pub const ACTIONS: &[&str] = &[
    "createEmailTopicAction",
    "updateEmailTopicAction",
    "deleteEmailTopicAction",
];

/// `createEmailTopicRequest.name` / `updateEmailTopicRequest.name`.
fn topic_name(input: &In<'_>) -> Result<String, OrpcError> {
    input.str("name", true, 1, 255)
}

/// A `zodBigintAsString` bind arg.
fn bound_id(value: &Value) -> Result<i64, OrpcError> {
    match value {
        Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(text.parse::<i64>().unwrap_or(-1))
        }
        _ => Err(invalid()),
    }
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let input = In(input);
    Some(match path {
        "emailTopicsAPI.privateListWorkspaceEmailTopicsAPI" => {
            async {
                let result = email_topic::list(
                    db,
                    ws,
                    ListEmailTopics {
                        name: input.opt_str("name")?,
                        folder_id: input.opt_id("folderId")?,
                        page: input.opt_positive("page")?,
                        per_page: input.opt_positive("perPage")?,
                        sort: input.sort(),
                    },
                )
                .await
                .map_err(fail)?;
                Ok(page(
                    result.data.iter().map(|t| t.to_json()),
                    result.page_count,
                ))
            }
            .await
        }
        "emailTopicsAPI.privateCreateWorkspaceEmailTopicAPI" => {
            async {
                let name = topic_name(&input)?;
                let topic = email_topic::create(db, ws, &name, input.opt_id("folderId")?)
                    .await
                    .map_err(fail)?;
                Ok(json!({ "id": topic.id.to_string() }))
            }
            .await
        }
        "emailTopicsAPI.privateUpdateEmailTopicAPI" => {
            async {
                let id = input.id("id")?;
                let name = topic_name(&input)?;
                email_topic::update(db, ws, id, &name)
                    .await
                    .map(|t| t.to_json())
                    .map_err(fail)
            }
            .await
        }
        "emailTopicsAPI.privateDeleteEmailTopicAPI" => {
            async {
                let id = input.id("id")?;
                email_topic::delete(db, ws, vec![id]).await.map_err(fail)?;
                Ok(Value::Null)
            }
            .await
        }
        _ => return None,
    })
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    Some(
        async {
            match name {
                // [workspaceId], input
                "createEmailTopicAction" => {
                    let input = In(arg(args, 1));
                    let topic_name = topic_name(&input)?;
                    email_topic::create(db, ws, &topic_name, input.opt_id("folderId")?)
                        .await
                        .map(|t| t.to_json())
                        .map_err(action_fail)
                }
                // [workspaceId, id], input
                "updateEmailTopicAction" => {
                    let id = bound_id(arg(args, 1))?;
                    let topic_name = topic_name(&In(arg(args, 2)))?;
                    email_topic::update(db, ws, id, &topic_name)
                        .await
                        .map(|t| t.to_json())
                        .map_err(action_fail)
                }
                // [workspaceId], { ids }
                _ => {
                    let ids = In(arg(args, 1)).ids("ids")?;
                    email_topic::delete(db, ws, ids)
                        .await
                        .map_err(action_fail)?;
                    Ok(Value::Null)
                }
            }
        }
        .await,
    )
}
