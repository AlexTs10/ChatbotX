//! The option lists of the contacts / flows screens: tags, custom fields,
//! bot fields, workspace members (`features/{tags,custom-fields,bot-fields,
//! workspace-members}/api/private.ts`).

use chatbotx_business::bot_field::{self, ListBotFields};
use chatbotx_business::custom_field::{self, ListCustomFields};
use chatbotx_business::tag::{self, ListTags};
use chatbotx_business::workspace_member;
use serde_json::{Value, json};

use chatbotx_db::repo::sequence::{self, SequenceListInput};

use super::{Ctx, In, Outcome, action_fail, arg, fail, page, public};
use crate::orpc::OrpcError;

const PATHS: &[&str] = &[
    "sequencesAPI.listSequencesWorkspaceAuthAPI",
    "tagsAPI.privateListWorkspaceTagsAPI",
    "customFieldsAPI.privateListCustomFieldsAPI",
    "botFieldAPIs.privateListBotFieldsAPI",
    "workspaceMembersAPI.listWorkspaceMembersAuthenticatedAPI",
];

/// The `{ id, name }` option lists of the flow editor steps.
async fn flow_options(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let pairs = |rows: Vec<(i64, String)>| -> Vec<Value> {
        rows.into_iter()
            .map(|(id, name)| json!({ "id": id.to_string(), "name": name }))
            .collect()
    };
    Some(match path {
        // `reflinkService.listOptions`: `refLink` rows, newest first, 500 max.
        "refLinksAPI.listRefLinkOptionsAuthenticatedAPI" => {
            chatbotx_business::tx::read(db, move |conn| {
                let rows: Vec<chatbotx_db::repo::models::Reflink> = chatbotx_db::repo::find_many(
                    conn,
                    r#"WHERE "workspaceId" = ?1 AND "type" = 'refLink' ORDER BY "createdAt" DESC LIMIT 500"#,
                    &[&ws],
                )?;
                Ok(rows.into_iter().map(|r| (r.id, r.name)).collect())
            })
            .await
            .map(|rows| json!({ "data": pairs(rows) }))
            .map_err(fail)
        }
        "questionnairesAPI.listQuestionnairesForFlowAPI" => match In(input).opt_str("keyword") {
            Ok(keyword) => chatbotx_business::questionnaire::list_for_flow(db, ws, keyword)
                .await
                .map(|rows| Value::Array(pairs(rows)))
                .map_err(fail),
            Err(error) => Err(error),
        },
        "placesNearMeAPI.listPlaceCategoryOptionsAPI" => {
            chatbotx_business::place_near_me::category::list(db, ws)
                .await
                .map(|rows| json!({ "data": pairs(rows.into_iter().map(|c| (c.id, c.name)).collect()) }))
                .map_err(fail)
        }
        "productsAPI.listSendableProductsAuthorizedAPI" => {
            let query = super::query_of(input, &["channel", "keyword", "page", "perPage"]);
            public(ctx, "products.listSendable", &[], Some(&query), None).await
        }
        _ => return None,
    })
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    if let Some(outcome) = flow_options(ctx, path, input).await {
        return Some(outcome);
    }
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
                "sequencesAPI.listSequencesWorkspaceAuthAPI" => {
                    let (page_number, per_page) = paging()?;
                    let list_input = SequenceListInput {
                        workspace_id: ws,
                        name: input.opt_str("name")?,
                        folder_id: input.opt_id("folderId")?.map(|id| id.to_string()),
                        active: input.opt_bool("active")?,
                        page: page_number,
                        per_page,
                        sort: input.sort(),
                    };
                    chatbotx_business::tx::read(db, move |conn| {
                        let rows = sequence::list_with_counts(conn, &list_input)?;
                        let total = sequence::count(conn, &list_input)?;
                        let limit = list_input.per_page.unwrap_or(10).max(1);
                        Ok(page(
                            rows.iter().map(|row| {
                                let mut out = row.sequence.to_json();
                                out["stepsCount"] = json!(row.steps_count);
                                out["subscribersCount"] = json!(row.subscribers_count);
                                out
                            }),
                            (total + limit - 1) / limit,
                        ))
                    })
                    .await
                    .map_err(fail)
                }
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

// ---- actions --------------------------------------------------------------

/// The ported actions.
pub const ACTIONS: &[&str] = &[
    "createTagAction",
    "updateTagAction",
    "deleteTagAction",
    "updateCustomFieldAction",
    "deleteFieldsAction",
    "createBotFieldAction",
    "updateBotFieldAction",
    "deleteBotFieldsAction",
    "resetBotFieldsAction",
];

/// The tags / custom fields / bot fields pages. Create and update run the
/// public route (same request schema and service); a `validation` error is
/// the form's `name` error (`returnValidationErrors`).
pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    Some(
        async {
            let body = arg(args, args.len().saturating_sub(1).max(1));
            let id = || {
                In(&json!({ "id": arg(args, 1) }))
                    .id("id")
                    .map(|id| id.to_string())
            };
            let named = |route: &'static str, params: Vec<(&'static str, String)>| async move {
                public(ctx, route, &params, None, Some(body))
                    .await
                    .map_err(name_error)
            };
            match name {
                "createTagAction" => named("tags.create", vec![]).await,
                "updateTagAction" => named("tags.update", vec![("id", id()?)]).await,
                "updateCustomFieldAction" => named("customFields.update", vec![("id", id()?)])
                    .await
                    .map(|_| Value::Null),
                "createBotFieldAction" => named("botFields.create", vec![]).await,
                "updateBotFieldAction" => {
                    named("botFields.update", vec![("idOrName", id()?)]).await
                }
                _ => {
                    let ids = In(body).ids("ids")?;
                    match name {
                        "deleteTagAction" => tag::soft_delete(db, ws, &ids).await.map(|_| ()),
                        "deleteFieldsAction" => custom_field::delete(db, ws, &ids).await,
                        "deleteBotFieldsAction" => bot_field::bulk_delete(db, ws, &ids).await,
                        _ => bot_field::bulk_clear_values(db, ws, &ids).await,
                    }
                    .map(|()| Value::Null)
                    .map_err(action_fail)
                }
            }
        }
        .await,
    )
}

/// A `validation` error of the route as `validationErrors.name`.
fn name_error(mut error: OrpcError) -> OrpcError {
    if error.code == "validation" {
        error.data = Some(json!({
            "validationErrors": {
                "_errors": ["Validation Exception"],
                "name": { "_errors": [error.message] },
            }
        }));
    }
    error
}
