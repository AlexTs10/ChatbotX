//! Mini Apps in the SPA (Q.D1m): `miniAppsAPI.listMiniAppOptionsAPI`
//! (`features/mini-apps/api/private.ts`), the page queries
//! (`features/mini-apps/queries`) and the builder actions
//! (`features/mini-apps/actions/*`, the WhatsApp publish / draft / preview
//! actions included).

use chatbotx_business::BusinessError;
use chatbotx_business::mini_app::{self as service, DefinitionInput};
use chatbotx_core::mini_app as app;
use chatbotx_db::repo::SortItem;
use chatbotx_db::repo::models::{IntegrationWhatsapp, MiniApp, MiniAppPublication};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, dm_conversation_id, fail, page};
use crate::handlers::{date, id};
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "listMiniApps",
    "findMiniApp",
    "listMiniAppSubmissions",
    "listWhatsappPublishTargets",
    "createMiniAppAction",
    "updateMiniAppAction",
    "duplicateMiniAppAction",
    "deleteMiniAppsAction",
    "getMiniAppFlowJsonAction",
    "findSubmissionConversationAction",
    "loadMediaLibraryImageAction",
    "publishMiniAppWhatsappAction",
    "publishMiniAppWhatsappDraftAction",
    "getMiniAppWhatsappPreviewAction",
    "deleteMiniAppWhatsappDraftAction",
];

/// `MINI_APP_OPTIONS_LIMIT`.
const OPTIONS_LIMIT: i64 = 500;
/// `MAX_SOURCE_IMAGE_BYTES`.
const MAX_SOURCE_IMAGE_BYTES: i64 = 8 * 1024 * 1024;

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, _input: &Value) -> Outcome {
    if path != "miniAppsAPI.listMiniAppOptionsAPI" {
        return None;
    }
    Some(
        service::list(&ctx.api.db, ctx.workspace_id, None, 1, OPTIONS_LIMIT, None)
            .await
            .map(|(rows, _)| {
                rows.iter()
                    .map(|r| json!({ "id": id(r.id), "name": r.name }))
                    .collect()
            })
            .map_err(fail),
    )
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

fn publication_json(p: &MiniAppPublication) -> Value {
    json!({
        "id": id(p.id),
        "createdAt": date(p.created_at),
        "updatedAt": date(p.updated_at),
        "miniAppId": id(p.mini_app_id),
        "integrationWhatsappId": id(p.integration_whatsapp_id),
        "whatsappFlowId": p.whatsapp_flow_id.map(id),
        "sourceId": p.source_id,
        "status": p.status,
        "validationErrors": p.validation_errors.0,
        "publishedAt": p.published_at.map(date),
    })
}

/// The `MiniAppModel` row with its publications.
fn mini_app_json(row: &MiniApp, publications: &[MiniAppPublication]) -> Value {
    json!({
        "id": id(row.id),
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
        "workspaceId": id(row.workspace_id),
        "name": row.name,
        "enabled": row.enabled,
        "definition": row.definition.0,
        "flowJson": row.flow_json.0,
        "submissionsCount": row.submissions_count,
        "publications": publications.iter().map(publication_json).collect::<Vec<_>>(),
    })
}

/// `getSortingStateParser` items of a list search (`[{ id, desc }]`).
fn sort_of(search: &Value) -> Option<Vec<SortItem>> {
    let items: Vec<SortItem> = search
        .get("sort")?
        .as_array()?
        .iter()
        .filter_map(|item| {
            Some(SortItem {
                id: item.get("id")?.as_str()?.to_owned(),
                desc: item.get("desc")?.as_bool()?,
            })
        })
        .collect();
    (!items.is_empty()).then_some(items)
}

/// `whatsappNumberLabel`: verified name · display number, else the id.
fn number_label(integration_id: i64, auth: &Value) -> String {
    let phone = auth.pointer("/metadata/phoneNumber");
    let part = |key: &str| {
        phone
            .and_then(|p| p.get(key))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    let label: Vec<&str> = [part("verified_name"), part("display_phone_number")]
        .into_iter()
        .flatten()
        .collect();
    if label.is_empty() {
        integration_id.to_string()
    } else {
        label.join(" · ")
    }
}

async fn publish_targets(ctx: &Ctx<'_>) -> Result<Value, OrpcError> {
    use chatbotx_db::repo::Model as _;
    let ws = ctx.workspace_id;
    let rows = chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        let sql = format!(
            r#"SELECT {} FROM "IntegrationWhatsapp" WHERE "workspaceId" = ?1 ORDER BY "createdAt""#,
            IntegrationWhatsapp::columns_sql(None)
        );
        let mut stmt = conn.prepare(&sql).map_err(chatbotx_db::repo::Error::from)?;
        let rows = stmt
            .query_map([ws], IntegrationWhatsapp::from_row)
            .and_then(Iterator::collect::<Result<Vec<_>, _>>)
            .map_err(chatbotx_db::repo::Error::from)?;
        Ok(rows)
    })
    .await
    .map_err(fail)?;
    Ok(Value::Array(
        rows.iter()
            .map(|r| json!({ "id": id(r.id), "label": number_label(r.id, &r.auth.0) }))
            .collect(),
    ))
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let (db, ws) = (&ctx.api.db, ctx.workspace_id);
    let bound = |index: usize| In(&json!({ "id": arg(args, index) })).id("id");
    match name {
        "listMiniApps" => {
            let search = In(arg(args, 0));
            let (page_no, per_page) = (
                search.opt_positive("page")?.unwrap_or(1),
                search.opt_positive("perPage")?.unwrap_or(10),
            );
            let keyword = search.opt_str("keyword")?.filter(|k| !k.is_empty());
            let (rows, page_count) =
                service::list(db, ws, keyword, page_no, per_page, sort_of(search.0))
                    .await
                    .map_err(fail)?;
            let data = rows.iter().map(|r| {
                json!({
                    "id": id(r.id),
                    "createdAt": date(r.created_at),
                    "updatedAt": date(r.updated_at),
                    "workspaceId": id(r.workspace_id),
                    "name": r.name,
                    "enabled": r.enabled,
                    "submissionsCount": r.submissions_count,
                    "publications": r.publications.iter().map(|p| json!({
                        "id": id(p.id),
                        "integrationWhatsappId": id(p.integration_whatsapp_id),
                        "sourceId": p.source_id,
                        "status": p.status,
                        "publishedAt": p.published_at.map(date),
                    })).collect::<Vec<_>>(),
                })
            });
            Ok(page(data, page_count))
        }
        // `findOrFail(..).catch(() => undefined)`.
        "findMiniApp" => {
            let app_id = In(arg(args, 0)).id("id")?;
            Ok(match service::find_or_fail(db, ws, app_id).await {
                Ok(found) => mini_app_json(&found.mini_app, &found.publications),
                Err(_) => Value::Null,
            })
        }
        "listMiniAppSubmissions" => {
            let input = In(arg(args, 0));
            let app_id = input.id("miniAppId")?;
            let (page_no, per_page) = (
                input.opt_positive("page")?.unwrap_or(1),
                input.opt_positive("perPage")?.unwrap_or(20),
            );
            let (rows, page_count, total) =
                service::list_submissions(db, ws, app_id, page_no, per_page)
                    .await
                    .map_err(fail)?;
            let data: Vec<Value> = rows
                .iter()
                .map(|r| {
                    let s = &r.submission;
                    json!({
                        "id": id(s.id),
                        "createdAt": date(s.created_at),
                        "updatedAt": date(s.updated_at),
                        "workspaceId": id(s.workspace_id),
                        "miniAppId": id(s.mini_app_id),
                        "contactId": s.contact_id.map(id),
                        "source": s.source,
                        "answers": s.answers.0,
                        "contact": r.contact.as_ref().map(|(cid, full_name, avatar)| {
                            json!({ "id": id(*cid), "fullName": full_name, "avatar": avatar })
                        }),
                    })
                })
                .collect();
            Ok(json!({ "data": data, "pageCount": page_count, "totalRows": total }))
        }
        "listWhatsappPublishTargets" => publish_targets(ctx).await,
        "createMiniAppAction" => {
            let input = In(arg(args, 1));
            let name = input.str("name", true, 1, app::NAME_MAX_LENGTH)?;
            let data = DefinitionInput {
                definition: Some(app::starter_definition().to_value()),
                flow_json: None,
                custom_field_mappings: None,
            };
            let created = service::create(db, ws, name, data)
                .await
                .map_err(action_fail)?;
            Ok(json!({ "id": id(created.id) }))
        }
        "updateMiniAppAction" => {
            let app_id = bound(1)?;
            let input = In(arg(args, 2));
            let name = match input.get("name") {
                Some(_) => Some(input.str("name", true, 1, app::NAME_MAX_LENGTH)?),
                None => None,
            };
            let enabled = input.opt_bool("enabled")?;
            let definition = input.get("definition").cloned();
            if definition
                .as_ref()
                .is_some_and(|d| app::parse_definition(d).is_none())
            {
                return Err(super::invalid());
            }
            let data = DefinitionInput {
                definition,
                flow_json: None,
                custom_field_mappings: None,
            };
            service::update(db, ws, app_id, name, enabled, data)
                .await
                .map_err(action_fail)?;
            Ok(Value::Null)
        }
        "duplicateMiniAppAction" => {
            let copy = service::duplicate(db, ws, bound(1)?)
                .await
                .map_err(action_fail)?;
            Ok(json!({ "id": id(copy.id) }))
        }
        "deleteMiniAppsAction" => {
            let input = In(arg(args, 1));
            let ids = input.ids("ids")?;
            if ids.len() > crate::handlers::mini_apps::BULK_DELETE_MAX_IDS {
                return Err(super::invalid());
            }
            let delete_flows = input.opt_bool("deleteWhatsappFlows")?.unwrap_or(false);
            crate::handlers::mini_apps::delete_mini_apps(ctx.api, ws, ids, delete_flows)
                .await
                .map_err(action_fail)
        }
        "publishMiniAppWhatsappAction"
        | "publishMiniAppWhatsappDraftAction"
        | "getMiniAppWhatsappPreviewAction"
        | "deleteMiniAppWhatsappDraftAction" => {
            use chatbotx_business::mini_app_whatsapp as whatsapp;
            let input = In(arg(args, 2));
            let t = whatsapp::Target {
                workspace_id: ws,
                mini_app_id: bound(1)?,
                integration_whatsapp_id: input.id("integrationWhatsappId")?,
            };
            let graph = ctx.api.graph_url.as_str();
            match name {
                "publishMiniAppWhatsappAction" => {
                    let publish = input.opt_bool("publish")?.unwrap_or(true);
                    let out = whatsapp::publish(db, graph, t, publish)
                        .await
                        .map_err(action_fail)?;
                    Ok(crate::handlers::mini_apps::publish_json(&out))
                }
                "publishMiniAppWhatsappDraftAction" => {
                    let out = whatsapp::publish_draft(db, graph, t)
                        .await
                        .map_err(action_fail)?;
                    Ok(crate::handlers::mini_apps::publish_json(&out))
                }
                "getMiniAppWhatsappPreviewAction" => {
                    whatsapp::preview(db, graph, t).await.map_err(action_fail)
                }
                _ => {
                    let outcome = whatsapp::delete_draft(db, graph, t)
                        .await
                        .map_err(action_fail)?;
                    Ok(json!({ "outcome": outcome }))
                }
            }
        }
        "getMiniAppFlowJsonAction" => {
            let found = service::find_or_fail(db, ws, bound(1)?)
                .await
                .map_err(action_fail)?;
            Ok(json!({ "flowJson": found.mini_app.flow_json.0 }))
        }
        "findSubmissionConversationAction" => {
            let contact_id = In(arg(args, 1)).id("contactId")?;
            Ok(json!({ "conversationId": dm_conversation_id(ctx, contact_id).await?.map(id) }))
        }
        "loadMediaLibraryImageAction" => {
            let file_id = In(arg(args, 1)).id("fileId")?;
            let Some(uploader) = &ctx.api.files else {
                return Err(action_fail(BusinessError::not_found("File not found")));
            };
            let (mime_type, base64) = chatbotx_business::media_library::read_image(
                db,
                uploader,
                ws,
                file_id,
                MAX_SOURCE_IMAGE_BYTES,
            )
            .await
            .map_err(action_fail)?;
            Ok(json!({ "mimeType": mime_type, "base64": base64 }))
        }
        _ => Err(fail(BusinessError::not_found("Operation not ported yet"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_label_joins_the_verified_name_and_number_else_the_id() {
        let both = json!({ "metadata": { "phoneNumber": { "verified_name": "Shop", "display_phone_number": "+1 555" } } });
        assert_eq!(number_label(7, &both), "Shop · +1 555");
        assert_eq!(number_label(7, &json!({})), "7");
    }
}
