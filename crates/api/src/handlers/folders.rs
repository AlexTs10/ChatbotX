//! `features/folders/api/public.ts` (P3.6-D2 slice folders):
//! `folders.list / create / update / delete`, limited to the contacts-owned
//! folder types (`tag`, `customField`).

use chatbotx_business::BusinessError;
use chatbotx_business::folder::{ROOT_FOLDER_ID, bulk_delete, create, find_or_fail, list, update};
use chatbotx_db::repo::models::Folder;
use serde_json::{Value, json};

use super::input::{Input, Str, key};
use super::{
    ApiState, Call, ErrorMap, LISTING, MUTATING, Reply, business_error, date, id, json, no_content,
    opt_id,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "folders.list",
    "folders.create",
    "folders.update",
    "folders.delete",
];

/// `contactsFolderTypes`.
const CONTACTS_FOLDER_TYPES: &[&str] = &["tag", "customField"];

/// `possibleErrorsOnCreatingInFolder`.
const CREATING_IN_FOLDER: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("notFound", 404),
];

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    match route_id {
        "folders.list" => list_folders(state, call).await,
        "folders.create" => create_folder(state, call).await,
        "folders.update" => update_folder(state, call).await,
        "folders.delete" => delete_folder(state, call).await,
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// `folderResource`.
pub fn folder_json(folder: &Folder) -> Value {
    let paths: Vec<Value> = folder
        .paths
        .0
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| match item {
                    Value::Number(n) => json!(n.to_string()),
                    other => other.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "id": id(folder.id),
        "createdAt": date(folder.created_at),
        "updatedAt": date(folder.updated_at),
        "name": folder.name,
        "folderType": folder.folder_type,
        "parentId": opt_id(folder.parent_id),
        "workspaceId": id(folder.workspace_id),
        "isTrash": folder.is_trash,
        "paths": paths,
    })
}

/// `createFolderSchema.shape.name`: `z.string().trim().min(1).max(255)`.
fn name(input: &mut Input) -> Option<String> {
    input.string(
        "name",
        Str {
            trim: true,
            min: Some(1),
            max: Some(255),
            ..Str::default()
        },
    )
}

fn folder_type(input: &mut Input) -> Option<String> {
    let value = input.get("folderType").cloned();
    input.enum_at(
        &key("folderType"),
        value.as_ref(),
        CONTACTS_FOLDER_TYPES,
        true,
    )
}

/// `requireContactsFolder`: the workspace's folder of a contacts type, else
/// 404 "Folder not found".
async fn require_contacts_folder(
    state: &ApiState,
    workspace_id: i64,
    folder_id: i64,
    errors: ErrorMap,
) -> Result<Folder, crate::orpc::OrpcError> {
    let folder = find_or_fail(&state.db, workspace_id, folder_id, None)
        .await
        .map_err(|e| business_error(e, errors))?;
    if !CONTACTS_FOLDER_TYPES.contains(&folder.folder_type.as_str()) {
        return Err(business_error(
            BusinessError::not_found("Folder not found"),
            errors,
        ));
    }
    Ok(folder)
}

async fn list_folders(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let folder_type = folder_type(&mut input);
    let parent_id = input.opt_string("parentId", Str::default());
    input.finish()?;
    let folder_type = folder_type.unwrap_or_default();
    // The route passes `parentId ?? null`: no parent narrows nothing.
    let folders = list(
        &state.db,
        call.workspace_id,
        &folder_type,
        parent_id.as_deref(),
        None,
    )
    .await
    .map_err(|e| business_error(e, LISTING))?;
    let data: Vec<Value> = folders.iter().map(folder_json).collect();
    Ok(json(200, json!({ "data": data })))
}

async fn create_folder(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let name = name(&mut input);
    let folder_type = folder_type(&mut input);
    let parent = match input.get("parentId").cloned() {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text),
        Some(other) => {
            input.issue(
                &key("parentId"),
                "invalid_type",
                &format!(
                    "Invalid input: expected string, received {}",
                    if other.is_number() {
                        "number"
                    } else {
                        "object"
                    }
                ),
            );
            None
        }
    };
    input.finish()?;
    let parent_id = match parent.as_deref() {
        None | Some("") | Some(ROOT_FOLDER_ID) => None,
        // A parent id that is not a bigint is no folder of the workspace.
        Some(text) => Some(text.parse::<i64>().unwrap_or(-1)),
    };
    let folder = create(
        &state.db,
        call.workspace_id,
        &name.unwrap_or_default(),
        &folder_type.unwrap_or_default(),
        parent_id,
    )
    .await
    .map_err(|e| business_error(e, CREATING_IN_FOLDER))?;
    Ok(json(200, folder_json(&folder)))
}

async fn update_folder(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let folder_id = input.id("id");
    let name = name(&mut input);
    input.finish()?;
    let (Some(folder_id), Some(name)) = (folder_id, name) else {
        return Err(crate::routes::not_ported_error());
    };
    require_contacts_folder(state, call.workspace_id, folder_id, MUTATING).await?;
    let folder = update(&state.db, call.workspace_id, folder_id, &name)
        .await
        .map_err(|e| business_error(e, MUTATING))?;
    Ok(json(200, folder_json(&folder)))
}

async fn delete_folder(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_body(call.params, &[]);
    let folder_id = input.id("id");
    input.finish()?;
    let Some(folder_id) = folder_id else {
        return Err(crate::routes::not_ported_error());
    };
    let folder = require_contacts_folder(state, call.workspace_id, folder_id, MUTATING).await?;
    bulk_delete(&state.db, call.workspace_id, &[folder.id])
        .await
        .map_err(|e| business_error(e, MUTATING))?;
    Ok(no_content())
}
