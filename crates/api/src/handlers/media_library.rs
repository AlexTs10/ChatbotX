//! `features/media-library/api/public.ts`: `mediaLibrary.*`.

use chatbotx_business::media_library::{self as service, CreateFile, FileWithUrl, FolderWithCount};
use chatbotx_db::repo::media_library_file::ListFilesInput;
use chatbotx_db::repo::models::MediaLibraryFolder;
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Seg, Str, key};
use super::qr_codes::public_paging;
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, date, id, json,
    no_content, opt_date, opt_id,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "mediaLibrary.listFolders",
    "mediaLibrary.createFolder",
    "mediaLibrary.renameFolder",
    "mediaLibrary.deleteFolder",
    "mediaLibrary.createUploadUrl",
    "mediaLibrary.listFiles",
    "mediaLibrary.getFile",
    "mediaLibrary.createFile",
    "mediaLibrary.deleteFile",
    "mediaLibrary.setFavourite",
    "mediaLibrary.recordAccess",
    "mediaLibrary.moveFiles",
];

const NON_EMPTY: Str = Str {
    trim: false,
    min: Some(1),
    max: None,
    pattern: None,
};

/// `mediaLibraryFolderPublicResource`.
pub(crate) fn folder_json(folder: &MediaLibraryFolder) -> Map<String, Value> {
    let Value::Object(out) = json!({
        "id": id(folder.id),
        "createdAt": date(folder.created_at),
        "updatedAt": date(folder.updated_at),
        "name": folder.name,
    }) else {
        unreachable!("json! object")
    };
    out
}

pub(crate) fn folder_with_count(row: &FolderWithCount) -> Value {
    let mut out = folder_json(&row.folder);
    out.insert("fileCount".into(), Value::from(row.file_count));
    Value::Object(out)
}

/// `mediaLibraryFileListItemPublicResource`.
pub fn file_json(row: &FileWithUrl) -> Value {
    let file = &row.file;
    json!({
        "id": id(file.id),
        "createdAt": date(file.created_at),
        "updatedAt": date(file.updated_at),
        "name": file.name,
        "path": file.path,
        "mimeType": file.mime_type,
        "size": file.size,
        "isFavourite": file.is_favourite,
        "lastAccessedAt": opt_date(file.last_accessed_at),
        "folderId": opt_id(file.folder_id),
        "url": row.url,
    })
}

/// `fileIds`: `z.array(zodBigintAsString()).min(1)`.
fn file_ids(input: &mut Input) -> Option<Vec<i64>> {
    let path = key("fileIds");
    match input.get("fileIds").cloned() {
        Some(Value::Array(items)) => {
            if items.is_empty() {
                input.issue(
                    &path,
                    "too_small",
                    "Too small: expected array to have >=1 items",
                );
                return None;
            }
            let mut out = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let mut at = path.clone();
                at.push(Seg::Index(index));
                out.push(input.id_at(&at, Some(item), true));
            }
            out.into_iter().collect()
        }
        other => {
            let received = match other {
                None => "undefined",
                Some(Value::Null) => "null",
                Some(Value::String(_)) => "string",
                Some(Value::Number(_)) => "number",
                Some(Value::Bool(_)) => "boolean",
                Some(_) => "object",
            };
            input.issue(
                &path,
                "invalid_type",
                &format!("Invalid input: expected array, received {received}"),
            );
            None
        }
    }
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let storage_url = super::contacts::tenant_urls(state).storage_url;
    let uploader = state.files.as_deref();
    match route_id {
        "mediaLibrary.listFolders" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let folders = service::list_folders(db, ws)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            let page_count = ((folders.len() as i64 + per_page - 1) / per_page).max(1);
            let data: Vec<Value> = folders
                .iter()
                .skip(((page - 1) * per_page) as usize)
                .take(per_page as usize)
                .map(folder_with_count)
                .collect();
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "mediaLibrary.createFolder" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", NON_EMPTY);
            input.finish()?;
            service::create_folder(db, ws, &name.unwrap_or_default())
                .await
                .map(|folder| json(201, Value::Object(folder_json(&folder))))
                .map_err(|e| business_error(e, CREATING))
        }
        "mediaLibrary.renameFolder" => {
            let mut input = Input::from_body(call.params, call.body);
            let folder_id = input.id("folderId");
            let name = input.string("name", NON_EMPTY);
            input.finish()?;
            service::rename_folder(
                db,
                ws,
                folder_id.unwrap_or_default(),
                &name.unwrap_or_default(),
            )
            .await
            .map(|()| no_content())
            .map_err(|e| business_error(e, MUTATING))
        }
        "mediaLibrary.deleteFolder" => {
            let mut input = Input::from_query(call.params, call.query);
            let folder_id = input.id("folderId");
            input.finish()?;
            service::delete_folder(db, uploader, ws, folder_id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "mediaLibrary.createUploadUrl" => {
            let mut input = Input::from_body(call.params, call.body);
            let file_name = input.string("fileName", NON_EMPTY);
            let mime_type = input.string("mimeType", NON_EMPTY);
            input.finish()?;
            let Some(uploader) = uploader else {
                return Err(super::internal_error("file storage is off"));
            };
            service::presign_upload(
                db,
                uploader,
                &storage_url,
                ws,
                &file_name.unwrap_or_default(),
                &mime_type.unwrap_or_default(),
            )
            .await
            .map(|out| {
                json(
                    201,
                    json!({
                        "path": out.path,
                        "uploadUrl": out.upload_url,
                        "publicUrl": out.public_url,
                    }),
                )
            })
            .map_err(|e| business_error(e, CREATING))
        }
        "mediaLibrary.listFiles" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            let folder_id = input.nullish_id("folderId").flatten();
            let search = input.opt_string("search", Str::default());
            let filter = input.get("filter").cloned();
            let filter = input.enum_at(
                &key("filter"),
                filter.as_ref(),
                &["all", "recent", "favourite"],
                false,
            );
            let file_type = input.get("fileType").cloned();
            let file_type = input.enum_at(&key("fileType"), file_type.as_ref(), &["image"], false);
            input.finish()?;
            let list = service::list_files(
                db,
                &storage_url,
                ListFilesInput {
                    workspace_id: ws,
                    filter,
                    folder_id,
                    search,
                    file_type,
                    page: Some(page),
                    per_page,
                },
            )
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = list.data.iter().map(file_json).collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": list.page_count }),
            ))
        }
        "mediaLibrary.getFile" => {
            let mut input = Input::from_query(call.params, call.query);
            let file_id = input.id("fileId");
            input.finish()?;
            service::find_file(db, &storage_url, ws, file_id.unwrap_or_default())
                .await
                .map(|file| json(200, file_json(&file)))
                .map_err(|e| business_error(e, FINDING))
        }
        "mediaLibrary.createFile" => {
            let mut input = Input::from_body(call.params, call.body);
            let folder_id = input.nullish_id("folderId").flatten();
            let name = input.string("name", Str::default());
            let path = input.string("path", Str::default());
            let mime_type = input.string("mimeType", Str::default());
            let size = input.number("size", Num::default());
            input.finish()?;
            service::create_file(
                db,
                &storage_url,
                CreateFile {
                    workspace_id: ws,
                    folder_id,
                    name: name.unwrap_or_default(),
                    path: path.unwrap_or_default(),
                    mime_type: mime_type.unwrap_or_default(),
                    size: size.unwrap_or_default() as i64,
                },
            )
            .await
            .map(|file| json(201, file_json(&file)))
            .map_err(|e| business_error(e, CREATING))
        }
        "mediaLibrary.deleteFile" => {
            let mut input = Input::from_query(call.params, call.query);
            let file_id = input.id("fileId");
            input.finish()?;
            service::delete_file(db, uploader, ws, file_id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "mediaLibrary.setFavourite" => {
            let mut input = Input::from_body(call.params, call.body);
            let file_id = input.id("fileId");
            let is_favourite = input.bool("isFavourite");
            input.finish()?;
            service::set_favourite(
                db,
                &storage_url,
                ws,
                file_id.unwrap_or_default(),
                is_favourite.unwrap_or_default(),
            )
            .await
            .map(|file| json(200, file_json(&file)))
            .map_err(|e| business_error(e, MUTATING))
        }
        "mediaLibrary.recordAccess" => {
            let mut input = Input::from_body(call.params, call.body);
            let file_id = input.id("fileId");
            input.finish()?;
            service::record_file_access(db, ws, file_id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        "mediaLibrary.moveFiles" => {
            let mut input = Input::from_body(call.params, call.body);
            let ids = file_ids(&mut input);
            let folder_id = input.nullish_id("folderId").flatten();
            input.finish()?;
            service::move_files(db, ws, ids.unwrap_or_default(), folder_id)
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, MUTATING))
        }
        _ => Err(super::internal_error("unknown media library route")),
    }
}
