//! Flows and folders: `features/flows/api/private.ts`,
//! `features/folders/api/private.ts`, `features/flows/actions/*` and
//! `features/folders/actions/*`.

use chatbotx_business::flow::ListFlows;
use chatbotx_business::{flow, flow_version, folder};
use chatbotx_db::repo::flow::FlowWithVersions;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail, invalid, page, public};
use crate::orpc::OrpcError;

/// The ported actions.
pub const ACTIONS: &[&str] = &[
    "createFlowAction",
    "updateFlowAction",
    "deleteFlowAction",
    "duplicateFlowAction",
    "importFlowAction",
    "publishFlowAction",
    "updateDraftFlowVersionAction",
    "revertToPublishedAction",
    "restoreFlowVersionAction",
    "createFolderAction",
    "editFolderAction",
    "deleteFolderAction",
    "changeFolderAction",
];

/// `flowWithVersionsResource`.
pub(super) fn flow_json(flow: &FlowWithVersions) -> Value {
    let mut out = flow.flow.to_json();
    out["flowVersions"] = flow.flow_versions.iter().map(|v| v.to_json()).collect();
    out
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    Some(match path {
        "flowsAPI.privateListFlowsAPI" => list_flows(ctx, input).await,
        "flowsAPI.privateListFlowVersionsAPI" => match In(input).id("flowId") {
            Ok(flow_id) => flow_version::list(db, ws, flow_id)
                .await
                .map(|versions| versions.iter().map(|v| v.to_json()).collect())
                .map_err(fail),
            Err(error) => Err(error),
        },
        "foldersAPI.listFoldersAuthenticatedAPI" => list_folders(ctx, input).await,
        _ => return None,
    })
}

/// `listFlows` (`flowService.list`).
async fn list_flows(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let input = In(input);
    let list = ListFlows {
        workspace_id: ctx.workspace_id,
        name: input.opt_str("name")?,
        folder_id: input.opt_id("folderId")?.map(|id| id.to_string()),
        active: input.opt_bool("active")?,
        enable_in_inbox: None,
        page: input.opt_positive("page")?,
        per_page: input.opt_positive("perPage")?,
        sort: input.sort(),
        start_type: input.opt_str("startType")?,
        integration_whatsapp_id: input.opt_id("integrationWhatsappId")?,
    };
    let result = flow::list(&ctx.api.db, list).await.map_err(fail)?;
    Ok(page(result.data.iter().map(flow_json), result.page_count))
}

/// `listFolders` (`folderService.list`): `{ data }`.
async fn list_folders(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let input = In(input);
    let folder_type = input.opt_str("folderType")?.ok_or_else(invalid)?;
    let parent = input.opt_id("folderId")?.map(|id| id.to_string());
    let folders = folder::list(
        &ctx.api.db,
        ctx.workspace_id,
        &folder_type,
        parent.as_deref(),
        input.opt_bool("isTrash")?,
    )
    .await
    .map_err(fail)?;
    Ok(json!({ "data": folders.iter().map(|f| f.to_json()).collect::<Vec<_>>() }))
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    // `[workspaceId, id]` bind args, then the input.
    let bound_id = || match arg(args, 1) {
        Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(text.parse::<i64>().unwrap_or(-1))
        }
        _ => Err(invalid()),
    };
    let second = In(arg(args, 2));
    let first = In(arg(args, 1));
    Some(
        async {
            match name {
                "createFlowAction" => {
                    let created =
                        public(ctx, "flows.create", &[], None, Some(arg(args, 1))).await?;
                    let id = created
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(|id| id.parse::<i64>().ok())
                        .ok_or_else(invalid)?;
                    let flow = flow::find_by_id(db, ws, id).await.map_err(action_fail)?;
                    Ok(flow.flow.to_json())
                }
                "updateFlowAction" => {
                    let id = bound_id()?;
                    public(
                        ctx,
                        "flows.update",
                        &[("id", id.to_string())],
                        None,
                        Some(arg(args, 2)),
                    )
                    .await?;
                    Ok(Value::Null)
                }
                "deleteFlowAction" => {
                    let deleted = flow::delete_many(db, ws, first.ids("ids")?)
                        .await
                        .map_err(action_fail)?;
                    if !deleted.is_empty() {
                        let list = deleted
                            .iter()
                            .map(|id| format!("#{id}"))
                            .collect::<Vec<_>>();
                        let plural = if deleted.len() > 1 { "s" } else { "" };
                        ctx.api
                            .record_audit(
                                "delete",
                                &format!("deleted flow{plural} ({})", list.join(", ")),
                            )
                            .await;
                    }
                    Ok(Value::Null)
                }
                "duplicateFlowAction" => {
                    let id = bound_id()?;
                    let copy = public(
                        ctx,
                        "flows.duplicate",
                        &[("id", id.to_string())],
                        None,
                        None,
                    )
                    .await?;
                    let copy_id = copy
                        .get("id")
                        .and_then(Value::as_str)
                        .and_then(|id| id.parse::<i64>().ok())
                        .ok_or_else(invalid)?;
                    let flow = flow::find_by_id(db, ws, copy_id)
                        .await
                        .map_err(action_fail)?;
                    Ok(flow.flow.to_json())
                }
                "importFlowAction" => {
                    public(ctx, "flows.import", &[], None, Some(arg(args, 1))).await
                }
                "publishFlowAction" => {
                    let id = bound_id()?;
                    public(
                        ctx,
                        "flows.publish",
                        &[("id", id.to_string())],
                        None,
                        Some(arg(args, 2)),
                    )
                    .await?;
                    Ok(Value::Null)
                }
                "updateDraftFlowVersionAction" => {
                    let id = bound_id()?;
                    public(
                        ctx,
                        "flows.updateDraft",
                        &[("id", id.to_string())],
                        None,
                        Some(arg(args, 2)),
                    )
                    .await?;
                    Ok(json!({ "ok": true }))
                }
                "revertToPublishedAction" => {
                    let (nodes, edges) =
                        flow_version::revert_draft_to_published(db, ws, bound_id()?)
                            .await
                            .map_err(action_fail)?;
                    Ok(json!({ "nodes": nodes, "edges": edges }))
                }
                "restoreFlowVersionAction" => {
                    let flow_id = bound_id()?;
                    let version_id = second.id("versionId")?;
                    let version = flow_version::list(db, ws, flow_id)
                        .await
                        .map_err(action_fail)?
                        .into_iter()
                        .find(|v| v.id == version_id)
                        .ok_or_else(|| {
                            action_fail(chatbotx_business::BusinessError::not_found(
                                "Flow version not found",
                            ))
                        })?;
                    let (nodes, edges) = (version.nodes.0.clone(), version.edges.0.clone());
                    flow_version::restore(db, version)
                        .await
                        .map_err(action_fail)?;
                    Ok(json!({ "nodes": nodes, "edges": edges }))
                }
                "createFolderAction" => {
                    let name = first.str("name", true, 1, 255)?;
                    let folder_type = first.opt_str("folderType")?.ok_or_else(invalid)?;
                    if !folder::FOLDER_TYPES.contains(&folder_type.as_str()) {
                        return Err(invalid());
                    }
                    let parent = first.opt_id("parentId")?.filter(|id| *id != 0);
                    folder::create(db, ws, &name, &folder_type, parent)
                        .await
                        .map_err(action_fail)?;
                    Ok(Value::Null)
                }
                "editFolderAction" => {
                    let id = bound_id()?;
                    if let Some(name) = second.opt_str("name")? {
                        let name = name.trim().to_string();
                        if name.is_empty() || name.chars().count() > 255 {
                            return Err(invalid());
                        }
                        folder::update(db, ws, id, &name)
                            .await
                            .map_err(action_fail)?;
                    }
                    Ok(Value::Null)
                }
                "deleteFolderAction" => {
                    folder::bulk_delete(db, ws, &first.ids("ids")?)
                        .await
                        .map_err(action_fail)?;
                    Ok(Value::Null)
                }
                _ => {
                    let folder_type = first.opt_str("folderType")?.ok_or_else(invalid)?;
                    let new_folder_id = first.opt_str("newFolderId")?.ok_or_else(invalid)?;
                    if !new_folder_id.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(invalid());
                    }
                    folder::change_folder(
                        db,
                        ws,
                        &folder_type,
                        &first.ids("modelIds")?,
                        &new_folder_id,
                    )
                    .await
                    .map_err(action_fail)?;
                    Ok(Value::Null)
                }
            }
        }
        .await,
    )
}
