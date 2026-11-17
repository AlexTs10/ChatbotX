//! The media library: `features/media-library/api/private.ts` (the two
//! lists) and `features/media-library/actions` (8 actions). Any member of
//! the workspace (`workspaceAuthorizedMidddleware` / `workspaceActionClient`).

use chatbotx_business::media_library::{self as service, CreateFile};
use chatbotx_db::repo::media_library_file::ListFilesInput;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, fail};
use crate::handlers::media_library::{file_json, folder_json, folder_with_count};
use crate::orpc::OrpcError;

pub const ACTIONS: &[&str] = &[
    "createMediaLibraryFileAction",
    "createMediaLibraryFolderAction",
    "deleteMediaLibraryFileAction",
    "deleteMediaLibraryFolderAction",
    "moveMediaLibraryFilesAction",
    "recordMediaLibraryFileAccessAction",
    "renameMediaLibraryFolderAction",
    "toggleMediaLibraryFavouriteAction",
];

fn storage_url(ctx: &Ctx<'_>) -> String {
    crate::handlers::contacts::tenant_urls(ctx.api).storage_url
}

pub(super) async fn procedure(ctx: &Ctx<'_>, path: &str, input: &Value) -> Outcome {
    Some(match path {
        // All folders (no paging), with their file counts.
        "mediaLibraryAPI.listMediaLibraryFolders" => service::list_folders(
            &ctx.api.db,
            ctx.workspace_id,
        )
        .await
        .map(|rows| json!({ "data": rows.iter().map(folder_with_count).collect::<Vec<_>>() }))
        .map_err(fail),
        "mediaLibraryAPI.listMediaLibraryFiles" => list_files(ctx, input).await,
        _ => return None,
    })
}

/// `mediaLibraryFileService.list`: 60 per page (the business default).
async fn list_files(ctx: &Ctx<'_>, input: &Value) -> Result<Value, OrpcError> {
    let input = In(input);
    let one_of = |key: &str, allowed: &[&str]| match input.opt_str(key)? {
        Some(v) if !allowed.contains(&v.as_str()) => Err(super::invalid()),
        v => Ok(v),
    };
    let list = service::list_files(
        &ctx.api.db,
        &storage_url(ctx),
        ListFilesInput {
            workspace_id: ctx.workspace_id,
            filter: one_of("filter", &["recent", "favourite"])?,
            folder_id: input.opt_id("folderId")?,
            search: input.opt_str("search")?,
            file_type: one_of("fileType", &["image"])?,
            page: input.opt_positive("page")?,
            per_page: 0,
        },
    )
    .await
    .map_err(fail)?;
    Ok(json!({
        "data": list.data.iter().map(file_json).collect::<Vec<_>>(),
        "pageCount": list.page_count,
    }))
}

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let uploader = ctx.api.files.as_deref();
    let input = arg(args, 1);
    let body = In(input);
    // The actions that take a bare id (`zodBigintAsString()`).
    let bare_id = || In(&json!({ "id": input })).id("id");
    Some(
        async {
            match name {
                "createMediaLibraryFileAction" => service::create_file(
                    db,
                    &storage_url(ctx),
                    CreateFile {
                        workspace_id: ws,
                        folder_id: body.opt_id("folderId")?,
                        name: body.str("name", false, 0, usize::MAX)?,
                        path: body.str("path", false, 0, usize::MAX)?,
                        mime_type: body.str("mimeType", false, 0, usize::MAX)?,
                        size: body
                            .get("size")
                            .and_then(Value::as_f64)
                            .ok_or_else(super::invalid)? as i64,
                    },
                )
                .await
                .map(|file| file_json(&file)),
                "createMediaLibraryFolderAction" => {
                    let name = body.str("name", false, 1, usize::MAX)?;
                    service::create_folder(db, ws, &name)
                        .await
                        .map(|folder| Value::Object(folder_json(&folder)))
                }
                "renameMediaLibraryFolderAction" => {
                    let folder = body.id("folderId")?;
                    let name = body.str("name", false, 1, usize::MAX)?;
                    service::rename_folder(db, ws, folder, &name)
                        .await
                        .map(|()| Value::Null)
                }
                "moveMediaLibraryFilesAction" => {
                    let ids = body.ids("fileIds")?;
                    if ids.is_empty() {
                        return Err(super::invalid());
                    }
                    service::move_files(db, ws, ids, body.opt_id("folderId")?)
                        .await
                        .map(|()| Value::Null)
                }
                "deleteMediaLibraryFileAction" => {
                    service::delete_file(db, uploader, ws, bare_id()?)
                        .await
                        .map(|()| Value::Null)
                }
                "deleteMediaLibraryFolderAction" => {
                    service::delete_folder(db, uploader, ws, bare_id()?)
                        .await
                        .map(|()| Value::Null)
                }
                "recordMediaLibraryFileAccessAction" => {
                    service::record_file_access(db, ws, bare_id()?)
                        .await
                        .map(|()| Value::Null)
                }
                _ => service::toggle_favourite(db, &storage_url(ctx), ws, bare_id()?)
                    .await
                    .map(|file| file_json(&file)),
            }
            .map_err(action_fail)
        }
        .await,
    )
}
