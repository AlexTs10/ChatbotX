//! `packages/business/src/import` + `packages/imports` (registry, file
//! validation, contacts header match) (P3.6-D2 slice contact imports, part
//! one): the import upload, the header peek, the column-map suggestion, the
//! import list / get, the contacts CSV template. The import run (worker)
//! is part two.

pub mod csv;
pub mod template;

use chatbotx_db::Db;
use chatbotx_files::Uploader;
use serde_json::Value;

use crate::errors::Result;

/// `importTypes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportType {
    Contacts,
    Coupons,
    Products,
    Flow,
}

impl ImportType {
    /// The stored `type` / `File.subType`.
    pub fn as_str(self) -> &'static str {
        match self {
            ImportType::Contacts => "contacts",
            ImportType::Coupons => "coupons",
            ImportType::Products => "products",
            ImportType::Flow => "flow",
        }
    }

    /// `importTypes.safeParse`.
    pub fn parse(text: &str) -> Option<ImportType> {
        match text {
            "contacts" => Some(ImportType::Contacts),
            "coupons" => Some(ImportType::Coupons),
            "products" => Some(ImportType::Products),
            "flow" => Some(ImportType::Flow),
            _ => None,
        }
    }
}

/// `ImportConfig` of the registry.
#[derive(Debug)]
pub struct ImportConfig {
    pub max_file_size_mb: i64,
    pub accepted_formats: &'static [&'static str],
    /// `acceptedExtensions`: MIME type -> extensions (with the dot).
    pub accepted_extensions: &'static [(&'static str, &'static [&'static str])],
    /// `paths.storageUrl` (`:workspaceId`, `:fileName`).
    pub storage_path: &'static str,
}

/// The registry config of `import_type`.
pub fn config(import_type: ImportType) -> &'static ImportConfig {
    let _ = import_type;
    todo!("P3.6-D2")
}

/// `resolveImportFileFormat`: the format when the MIME type and the
/// extension are accepted, else `None`.
pub fn resolve_import_file_format(
    config: &ImportConfig,
    file_name: &str,
    mime_type: &str,
) -> Option<&'static str> {
    let _ = (config, file_name, mime_type);
    todo!("P3.6-D2")
}

/// `CreateImportUploadInput`.
#[derive(Debug, Clone)]
pub struct CreateImportUpload {
    pub workspace_id: i64,
    pub user_id: Option<i64>,
    pub import_type: ImportType,
    pub file_name: String,
    pub mime_type: String,
    pub file_size: i64,
}

/// `ImportUpload`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportUpload {
    pub file_id: i64,
    pub presigned_post_url: String,
    pub public_url: String,
    pub path: String,
}

/// `uploader.getPresignedUpload(path)`; a local URL also carries
/// `X-Amz-Expires=300` (not signed; clients read the expiry from it).
pub async fn presigned_upload_url(uploader: &Uploader, key: &str) -> Result<String> {
    let _ = (uploader, key);
    todo!("P3.6-D2")
}

/// `createImportUpload`: validates the declared file (400
/// `importUnsupportedFileType` / `importFileTooLarge`), records a pending
/// `File` and returns the presigned upload. `storage_url` is the tenant
/// storage URL (with a trailing `/`).
pub async fn create_import_upload(
    db: &Db,
    uploader: &Uploader,
    storage_url: &str,
    input: CreateImportUpload,
) -> Result<ImportUpload> {
    let _ = (db, uploader, storage_url, input);
    todo!("P3.6-D2")
}

/// `peekImportHeaders`: the header row of an uploaded import file (400
/// `importUnableToReadHeaders` / `importUnsupportedFileType` /
/// `importFileTooLarge` with `data: { size }`). XLSX header reading belongs
/// to the product import slice: an XLSX file is unreadable here.
pub async fn peek_import_headers(
    db: &Db,
    uploader: &Uploader,
    workspace_id: i64,
    file_id: i64,
    import_type: Option<ImportType>,
) -> Result<Vec<String>> {
    let _ = (db, uploader, workspace_id, file_id, import_type);
    todo!("P3.6-D2")
}

/// `suggestContactImportColumnMap` (`matchContactImportHeaders`): `(field,
/// header)` in `contactImportFields` order; a header is used once.
pub fn suggest_contact_import_column_map(headers: &[String]) -> Vec<(&'static str, String)> {
    let _ = headers;
    todo!("P3.6-D2")
}

/// One `importListSelection` row.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRow {
    pub id: i64,
    pub workspace_id: i64,
    pub user_id: Option<i64>,
    pub file_id: i64,
    pub file_name: String,
    pub import_type: String,
    pub status: String,
    pub total_count: i64,
    pub processed_count: i64,
    pub success_count: i64,
    pub failed_count: i64,
    pub error_message: Option<String>,
    pub error_sample: Value,
    pub completed_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `importService.list` input.
#[derive(Debug, Clone, Default)]
pub struct ListImports {
    pub import_type: Option<ImportType>,
    pub status: Option<String>,
    pub keyword: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub sort: Option<Vec<chatbotx_db::repo::SortItem>>,
}

/// `importService.list`: `(rows, pageCount)`, newest first by default.
pub async fn list(db: &Db, workspace_id: i64, input: ListImports) -> Result<(Vec<ImportRow>, i64)> {
    let _ = (db, workspace_id, input);
    todo!("P3.6-D2")
}

/// `importService.find` (scoped to the workspace and the type).
pub async fn find(
    db: &Db,
    workspace_id: i64,
    id: i64,
    import_type: ImportType,
) -> Result<Option<ImportRow>> {
    let _ = (db, workspace_id, id, import_type);
    todo!("P3.6-D2")
}
