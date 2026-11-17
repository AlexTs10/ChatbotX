//! `media-library/service.ts` (`mediaLibraryService`) and
//! `media-library-file/service.ts` (`mediaLibraryFileService`).
//!
//! `storage_url` is the tenant `storageUrl` (`resolveTenantSettings`); file
//! URLs are `getPublicFileUrl(path, storageUrl)`. Storage objects go through
//! the shared files service (`chatbotx_files::Uploader`).

use base64::Engine as _;
use chatbotx_core::utils::storage::get_public_file_url;
use chatbotx_db::Db;
use chatbotx_db::repo::media_library_file::{self as files, ListFilesInput, NewMediaLibraryFile};
use chatbotx_db::repo::media_library_folder as folders;
use chatbotx_db::repo::models::{MediaLibraryFile, MediaLibraryFolder};
use chatbotx_db::repo::new_id;
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_files::Uploader;

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `MEDIA_LIBRARY_FILES_PAGE_SIZE`.
pub const MEDIA_LIBRARY_FILES_PAGE_SIZE: i64 = 60;

/// `READABLE_IMAGE_TYPES`.
const READABLE_IMAGE_TYPES: &[&str] = &["image/jpeg", "image/png", "image/webp", "image/gif"];

/// `MediaLibraryFileWithUrl`.
#[derive(Debug, Clone, PartialEq)]
pub struct FileWithUrl {
    pub file: MediaLibraryFile,
    pub url: String,
}

/// `FolderWithFileCount`.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderWithCount {
    pub folder: MediaLibraryFolder,
    pub file_count: i64,
}

/// `CreateFileInput`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreateFile {
    pub workspace_id: i64,
    pub folder_id: Option<i64>,
    pub name: String,
    pub path: String,
    pub mime_type: String,
    pub size: i64,
}

/// The `presignUpload` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresignedUpload {
    pub path: String,
    pub upload_url: String,
    pub public_url: String,
}

/// The `list` result.
#[derive(Debug, Clone, PartialEq)]
pub struct FileList {
    pub data: Vec<FileWithUrl>,
    pub page_count: i64,
}

fn url_of(path: &str, storage_url: &str) -> String {
    get_public_file_url(path, storage_url).unwrap_or_else(|_| format!("{storage_url}{path}"))
}

fn with_url(file: MediaLibraryFile, storage_url: &str) -> FileWithUrl {
    let url = url_of(&file.path, storage_url);
    FileWithUrl { file, url }
}

fn not_found(file_id: i64) -> BusinessError {
    BusinessError::not_found(format!("MediaLibraryFile {file_id} not found"))
}

/// `listFolders`: the folders (by name) with their file counts.
pub async fn list_folders(db: &Db, workspace_id: i64) -> Result<Vec<FolderWithCount>> {
    tx::read(db, move |conn| {
        let counts = files::count_by_folder(conn, workspace_id)?;
        Ok(folders::list_by_workspace(conn, workspace_id)?
            .into_iter()
            .map(|folder| {
                let file_count = counts
                    .iter()
                    .find(|row| row.folder_id == Some(folder.id))
                    .map_or(0, |row| row.count);
                FolderWithCount { folder, file_count }
            })
            .collect())
    })
    .await
}

/// `createFolder`.
pub async fn create_folder(db: &Db, workspace_id: i64, name: &str) -> Result<MediaLibraryFolder> {
    let name = name.to_string();
    tx::write(db, move |tx| {
        Ok(folders::create(tx, None, workspace_id, &name)?)
    })
    .await
}

/// `renameFolder` (a foreign / unknown folder changes nothing).
pub async fn rename_folder(db: &Db, workspace_id: i64, folder_id: i64, name: &str) -> Result<()> {
    let name = name.to_string();
    tx::write(db, move |tx| {
        Ok(folders::rename(tx, workspace_id, folder_id, &name)?)
    })
    .await
}

/// `moveFiles` (`None` = root).
pub async fn move_files(
    db: &Db,
    workspace_id: i64,
    file_ids: Vec<i64>,
    folder_id: Option<i64>,
) -> Result<()> {
    tx::write(db, move |tx| {
        Ok(files::move_to_folder(
            tx,
            workspace_id,
            &file_ids,
            folder_id,
        )?)
    })
    .await
}

/// `recordFileAccess`: stamps `lastAccessedAt` (scoped by workspace).
pub async fn record_file_access(db: &Db, workspace_id: i64, file_id: i64) -> Result<()> {
    tx::write(db, move |tx| {
        Ok(files::touch_last_accessed_at(tx, workspace_id, file_id)?)
    })
    .await
}

/// Best-effort `uploader.deleteObject` (a failure is logged).
async fn delete_object(uploader: Option<&Uploader>, path: &str, what: &str) {
    let Some(uploader) = uploader else {
        return;
    };
    if let Err(error) = uploader.delete_object(path).await {
        tracing::warn!(err = %error, "{what}: S3 delete failed for {path}");
    }
}

/// `deleteFolder`: deletes the folder's storage objects (best effort), then
/// its files and the folder in one transaction.
pub async fn delete_folder(
    db: &Db,
    uploader: Option<&Uploader>,
    workspace_id: i64,
    folder_id: i64,
) -> Result<()> {
    let rows = tx::read(db, move |conn| {
        Ok(files::list_by_folder(conn, workspace_id, folder_id)?)
    })
    .await?;
    for (_, path) in &rows {
        delete_object(uploader, path, "deleteMediaLibraryFolder").await;
    }
    tx::write(db, move |tx| {
        files::delete_by_folder(tx, workspace_id, folder_id)?;
        folders::delete_by_id(tx, workspace_id, folder_id)?;
        Ok(())
    })
    .await
}

/// `createFile`: `path` must be under `workspaces/<id>/` or
/// `public/space/<id>/` (400 `invalidPath`).
pub async fn create_file(db: &Db, storage_url: &str, input: CreateFile) -> Result<FileWithUrl> {
    let ws = input.workspace_id;
    let scoped = input.path.starts_with(&format!("workspaces/{ws}/"))
        || input.path.starts_with(&format!("public/space/{ws}/"));
    if !scoped {
        return Err(BusinessError::exception(
            "Invalid file path",
            "invalidPath",
            400,
        ));
    }
    let file = tx::write(db, move |tx| {
        Ok(files::create(
            tx,
            &NewMediaLibraryFile {
                id: None,
                name: input.name,
                path: input.path,
                mime_type: input.mime_type,
                size: input.size,
                is_favourite: false,
                workspace_id: ws,
                folder_id: input.folder_id,
            },
        )?)
    })
    .await?;
    Ok(with_url(file, storage_url))
}

/// `presignUpload`: a key under `public/space/<id>/media-library/`, its
/// presigned `PUT` URL (5 minutes) and a pending generic `File` row
/// (`userId` null).
pub async fn presign_upload(
    db: &Db,
    uploader: &Uploader,
    storage_url: &str,
    workspace_id: i64,
    file_name: &str,
    mime_type: &str,
) -> Result<PresignedUpload> {
    let path = format!("public/space/{workspace_id}/media-library/{}", new_id());
    let upload_url = crate::import::presigned_upload_url(uploader, &path).await?;
    let public_url = url_of(&path, storage_url);
    let values = vec![
        ("workspaceId", Sql::Integer(workspace_id)),
        ("userId", Sql::Null),
        ("contextType", Sql::Text("generic".into())),
        ("subType", Sql::Text("generic".into())),
        ("path", Sql::Text(path.clone())),
        ("fileName", Sql::Text(file_name.to_string())),
        ("mimeType", Sql::Text(mime_type.to_string())),
        ("status", Sql::Text("pending".into())),
    ];
    tx::write(db, move |tx| {
        chatbotx_db::repo::file::create(tx, &values)?;
        Ok(())
    })
    .await?;
    Ok(PresignedUpload {
        path,
        upload_url,
        public_url,
    })
}

async fn find_row(db: &Db, workspace_id: i64, file_id: i64) -> Result<MediaLibraryFile> {
    tx::read(db, move |conn| {
        Ok(files::find_by_id(conn, workspace_id, file_id)?)
    })
    .await?
    .ok_or_else(|| not_found(file_id))
}

/// `findFile`: the workspace's file with its URL (404 `notFound`).
pub async fn find_file(
    db: &Db,
    storage_url: &str,
    workspace_id: i64,
    file_id: i64,
) -> Result<FileWithUrl> {
    Ok(with_url(
        find_row(db, workspace_id, file_id).await?,
        storage_url,
    ))
}

/// `readImage`: a JPEG / PNG / WebP / GIF file up to `max_bytes` as
/// `(mimeType, base64)`.
pub async fn read_image(
    db: &Db,
    uploader: &Uploader,
    workspace_id: i64,
    file_id: i64,
    max_bytes: i64,
) -> Result<(String, String)> {
    let file = find_row(db, workspace_id, file_id).await?;
    if !READABLE_IMAGE_TYPES.contains(&file.mime_type.as_str()) {
        return Err(BusinessError::validation(
            "fileId",
            "Only JPEG, PNG, WebP or GIF images can be used",
        ));
    }
    if file.size > max_bytes {
        return Err(BusinessError::validation(
            "fileId",
            "The image is too large",
        ));
    }
    let bytes = uploader
        .get_object(&file.path)
        .await
        .map_err(|e| BusinessError::Repo(chatbotx_db::repo::Error::msg(e.to_string())))?;
    Ok((
        file.mime_type,
        base64::engine::general_purpose::STANDARD.encode(bytes),
    ))
}

/// `deleteFile`: 404 when missing; the storage object (best effort), then
/// the row.
pub async fn delete_file(
    db: &Db,
    uploader: Option<&Uploader>,
    workspace_id: i64,
    file_id: i64,
) -> Result<()> {
    let file = find_row(db, workspace_id, file_id).await?;
    delete_object(uploader, &file.path, "deleteMediaLibraryFile").await;
    tx::write(db, move |tx| {
        Ok(files::delete_by_id(tx, workspace_id, file_id)?)
    })
    .await
}

/// `setFavourite`: the explicit state; returns the file with its URL.
pub async fn set_favourite(
    db: &Db,
    storage_url: &str,
    workspace_id: i64,
    file_id: i64,
    is_favourite: bool,
) -> Result<FileWithUrl> {
    let mut file = find_file(db, storage_url, workspace_id, file_id).await?;
    tx::write(db, move |tx| {
        Ok(files::set_favourite(
            tx,
            workspace_id,
            file_id,
            is_favourite,
        )?)
    })
    .await?;
    file.file.is_favourite = is_favourite;
    Ok(file)
}

/// `toggleFavourite`.
pub async fn toggle_favourite(
    db: &Db,
    storage_url: &str,
    workspace_id: i64,
    file_id: i64,
) -> Result<FileWithUrl> {
    let file = find_file(db, storage_url, workspace_id, file_id).await?;
    let next = !file.file.is_favourite;
    tx::write(db, move |tx| {
        Ok(files::set_favourite(tx, workspace_id, file_id, next)?)
    })
    .await?;
    let mut file = file;
    file.file.is_favourite = next;
    Ok(file)
}

/// `mediaLibraryFileService.list`: one page (default 60 per page) with URLs
/// and the page count.
pub async fn list_files(db: &Db, storage_url: &str, mut input: ListFilesInput) -> Result<FileList> {
    if input.per_page <= 0 {
        input.per_page = MEDIA_LIBRARY_FILES_PAGE_SIZE;
    }
    let per_page = input.per_page;
    let page = tx::read(db, move |conn| Ok(files::list(conn, &input)?)).await?;
    Ok(FileList {
        data: page
            .data
            .into_iter()
            .map(|file| with_url(file, storage_url))
            .collect(),
        page_count: (page.total + per_page - 1) / per_page,
    })
}

/// `mediaLibraryFileService.findByPath`.
pub async fn find_by_path(
    db: &Db,
    workspace_id: i64,
    path: &str,
) -> Result<Option<MediaLibraryFile>> {
    let path = path.to_string();
    tx::read(db, move |conn| {
        Ok(files::find_by_path(conn, workspace_id, &path)?)
    })
    .await
}

/// `mediaLibraryFileService.findById`.
pub async fn find_by_id(db: &Db, workspace_id: i64, id: i64) -> Result<Option<MediaLibraryFile>> {
    tx::read(db, move |conn| {
        Ok(files::find_by_id(conn, workspace_id, id)?)
    })
    .await
}
