//! The option lists of the contacts / flows screens: tags, custom fields,
//! bot fields, workspace members (`features/{tags,custom-fields,bot-fields,
//! workspace-members}/api/private.ts`).

use chatbotx_business::bot_field::{self, ListBotFields};
use chatbotx_business::custom_field::{self, ListCustomFields};
use chatbotx_business::tag::{self, ListTags};
use chatbotx_business::workspace_member;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, fail, page};
use crate::orpc::OrpcError;

const PATHS: &[&str] = &[
    "tagsAPI.privateListWorkspaceTagsAPI",
    "customFieldsAPI.privateListCustomFieldsAPI",
    "botFieldAPIs.privateListBotFieldsAPI",
    "workspaceMembersAPI.listWorkspaceMembersAuthenticatedAPI",
];

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    if !PATHS.contains(&path) {
        return None;
    }
    let input = In(input);
    Some(
        async {
            let db = &ctx.api.db;
            let ws = ctx.workspace_id;
            let paging = || -> Result<(Option<i64>, Option<i64>), OrpcError> {
                Ok((input.opt_positive("page")?, input.opt_positive("perPage")?))
            };
            match path {
                "tagsAPI.privateListWorkspaceTagsAPI" => {
                    let (page_number, per_page) = paging()?;
                    let result = tag::list(
                        db,
                        ws,
                        ListTags {
                            name: input.opt_str("name")?,
                            folder_id: input.opt_id("folderId")?,
                            page: page_number,
                            per_page,
                            sort: input.sort(),
                        },
                    )
                    .await
                    .map_err(fail)?;
                    Ok(page(
                        result.data.iter().map(|row| row.tag.to_json()),
                        result.page_count,
                    ))
                }
                "customFieldsAPI.privateListCustomFieldsAPI" => {
                    let (page_number, per_page) = paging()?;
                    let result = custom_field::list(
                        db,
                        ws,
                        ListCustomFields {
                            name: input.opt_str("name")?,
                            folder_id: input.opt_id("folderId")?,
                            page: page_number,
                            per_page,
                            sort: input.sort(),
                        },
                    )
                    .await
                    .map_err(fail)?;
                    Ok(page(
                        result.data.iter().map(|f| f.to_json()),
                        result.page_count,
                    ))
                }
                "botFieldAPIs.privateListBotFieldsAPI" => {
                    let (page_number, per_page) = paging()?;
                    let result = bot_field::list(
                        db,
                        ws,
                        ListBotFields {
                            name: input.opt_str("name")?,
                            folder_id: input.opt_id("folderId")?,
                            page: page_number,
                            per_page,
                            sort: input.sort(),
                        },
                    )
                    .await
                    .map_err(fail)?;
                    Ok(page(
                        result.data.iter().map(|f| f.to_json()),
                        result.page_count,
                    ))
                }
                _ => {
                    let (page_number, per_page) = paging()?;
                    let result = workspace_member::list_paginated(
                        db,
                        ws,
                        page_number,
                        per_page,
                        input.opt_str("keyword")?,
                    )
                    .await
                    .map_err(fail)?;
                    Ok(page(
                        result.data.iter().map(|m| {
                            let parse = |text: &str| {
                                serde_json::from_str::<Value>(text).unwrap_or(Value::Null)
                            };
                            json!({
                                "id": m.id.to_string(),
                                "createdAt": chatbotx_db::repo::iso_millis(m.created_at),
                                "updatedAt": chatbotx_db::repo::iso_millis(m.updated_at),
                                "workspaceId": m.workspace_id.to_string(),
                                "userId": m.user_id.to_string(),
                                "role": m.role,
                                "notificationChannels": parse(&m.notification_channels),
                                "notificationTypes": parse(&m.notification_types),
                                "permissions": parse(&m.permissions),
                                "onlineSince": m.online_since.map(chatbotx_db::repo::iso_millis),
                                "user": {
                                    "id": m.user_id.to_string(),
                                    "name": m.user_name,
                                    "image": m.user_image,
                                },
                            })
                        }),
                        result.page_count,
                    ))
                }
            }
        }
        .await,
    )
}
