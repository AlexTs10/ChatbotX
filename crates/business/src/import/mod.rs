//! `packages/business/src/import` + `packages/imports` (registry, file
//! validation, contacts header match) (P3.6-D2 slice contact imports, part
//! one): the import upload, the header peek, the column-map suggestion, the
//! import list / get, the contacts CSV template. The import run (worker)
//! is part two.

pub mod contacts;
pub mod csv;
pub mod template;

use chatbotx_db::Db;
use chatbotx_db::repo::models::Import;
use chatbotx_db::repo::utils::{order_by_sql, parse_order_by, with_id_tie_break};
use chatbotx_db::repo::{
    self, Model, PaginationInput, get_pagination_with_defaults, like_contains, new_id,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};
use chatbotx_files::{FilesError, Uploader};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

pub(crate) const BYTES_PER_MB: i64 = 1024 * 1024;

/// `importFormats`.
const IMPORT_FORMATS: &[&str] = &["csv", "xlsx", "xls", "json"];

const XLSX_MIME: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

static CONTACTS: ImportConfig = ImportConfig {
    max_file_size_mb: 20,
    accepted_formats: &["csv"],
    accepted_extensions: &[("text/csv", &[".csv"])],
    storage_path: "workspaces/:workspaceId/imports/contacts/:fileName",
};
static COUPONS: ImportConfig = ImportConfig {
    max_file_size_mb: 10,
    accepted_formats: &["csv"],
    accepted_extensions: &[("text/csv", &[".csv"])],
    storage_path: "workspaces/:workspaceId/imports/coupons/:fileName",
};
static PRODUCTS: ImportConfig = ImportConfig {
    max_file_size_mb: 10,
    accepted_formats: &["csv", "xlsx"],
    accepted_extensions: &[("text/csv", &[".csv"]), (XLSX_MIME, &[".xlsx"])],
    storage_path: "workspaces/:workspaceId/imports/products/:fileName",
};
static FLOW: ImportConfig = ImportConfig {
    max_file_size_mb: 5,
    accepted_formats: &["json"],
    accepted_extensions: &[("application/json", &[".json"])],
    storage_path: "workspaces/:workspaceId/imports/flows/:fileName",
};

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn files_err(error: &FilesError) -> BusinessError {
    BusinessError::Repo(repo::Error::msg(error.to_string()))
}

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
    match import_type {
        ImportType::Contacts => &CONTACTS,
        ImportType::Coupons => &COUPONS,
        ImportType::Products => &PRODUCTS,
        ImportType::Flow => &FLOW,
    }
}

/// `resolveImportFileFormat`: the format when the MIME type and the
/// extension are accepted, else `None`.
pub fn resolve_import_file_format(
    config: &ImportConfig,
    file_name: &str,
    mime_type: &str,
) -> Option<&'static str> {
    let (_, extensions) = config
        .accepted_extensions
        .iter()
        .find(|(mime, _)| *mime == mime_type)?;
    // `lastIndexOf(".") > 0`: a leading dot is not an extension.
    let extension = match file_name.rfind('.') {
        Some(index) if index > 0 => file_name[index..].to_lowercase(),
        _ => String::new(),
    };
    if !extensions.contains(&extension.as_str()) {
        return None;
    }
    let format = IMPORT_FORMATS
        .iter()
        .copied()
        .find(|format| extension.get(1..) == Some(*format))?;
    config.accepted_formats.contains(&format).then_some(format)
}

/// `handler.buildPath`: `import_<id>.<extension as given>` under the
/// type's storage path (`fileName.split(".").pop() || "txt"`).
fn build_path(config: &ImportConfig, workspace_id: i64, file_name: &str) -> String {
    let extension = file_name
        .rsplit('.')
        .next()
        .filter(|e| !e.is_empty())
        .unwrap_or("txt");
    config
        .storage_path
        .replace(":workspaceId", &workspace_id.to_string())
        .replace(":fileName", &format!("import_{}.{extension}", new_id()))
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
    let url = uploader
        .get_presigned_upload(key)
        .await
        .map_err(|e| files_err(&e))?;
    if url.contains("X-Amz-Expires=") {
        return Ok(url);
    }
    Ok(url.replacen('?', "?X-Amz-Expires=300&", 1))
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
    let config = config(input.import_type);
    let kind = input.import_type.as_str();
    if resolve_import_file_format(config, &input.file_name, &input.mime_type).is_none() {
        return Err(BusinessError::exception(
            format!(
                "Unsupported file type for a {kind} import: {} ({})",
                input.file_name, input.mime_type
            ),
            "importUnsupportedFileType",
            400,
        ));
    }
    if input.file_size > config.max_file_size_mb * BYTES_PER_MB {
        return Err(BusinessError::exception(
            format!(
                "File exceeds the {} MB limit for {kind} imports",
                config.max_file_size_mb
            ),
            "importFileTooLarge",
            400,
        ));
    }
    let path = build_path(config, input.workspace_id, &input.file_name);
    let presigned_post_url = presigned_upload_url(uploader, &path).await?;
    let patch: chatbotx_db::repo::patch::Patch = vec![
        ("workspaceId", Sql::Integer(input.workspace_id)),
        ("userId", input.user_id.map_or(Sql::Null, Sql::Integer)),
        ("contextType", Sql::Text("import".into())),
        ("subType", Sql::Text(kind.into())),
        ("path", Sql::Text(path.clone())),
        ("fileName", Sql::Text(input.file_name.clone())),
        ("mimeType", Sql::Text(input.mime_type.clone())),
        ("status", Sql::Text("pending".into())),
    ];
    let file = tx::write(db, move |conn| Ok(repo::file::create(conn, &patch)?)).await?;
    Ok(ImportUpload {
        file_id: file.id,
        presigned_post_url,
        public_url: format!("{storage_url}{path}"),
        path,
    })
}

fn unable_to_read() -> BusinessError {
    BusinessError::exception(
        "Unable to read the file headers",
        "importUnableToReadHeaders",
        400,
    )
}

fn unsupported() -> BusinessError {
    BusinessError::exception(
        "Unsupported import file type",
        "importUnsupportedFileType",
        400,
    )
}

fn file_too_large(max_mb: i64) -> BusinessError {
    BusinessError::Exception {
        message: format!("File exceeds the {max_mb} MB import limit"),
        code: "importFileTooLarge".into(),
        status: 400,
        field: None,
        data: Some(json!({ "size": max_mb })),
    }
}

/// `subType`, `fileName`, `mimeType`, `path`, `fileSize` of an import file.
type ImportFile = (Option<String>, String, String, String, Option<i64>);

/// `importService.findFile`: an import `File` of the workspace.
fn find_import_file(
    conn: &Connection,
    workspace_id: i64,
    file_id: i64,
) -> Result<Option<ImportFile>> {
    conn.query_row(
        r#"SELECT "subType", "fileName", "mimeType", "path", "fileSize" FROM "File"
           WHERE "id" = ?1 AND "workspaceId" = ?2 AND "contextType" = 'import'"#,
        params![file_id, workspace_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )
    .optional()
    .map_err(repo_err)
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
    let file = tx::read(db, move |conn| {
        find_import_file(conn, workspace_id, file_id)
    })
    .await?;
    let Some((sub_type, file_name, mime_type, path, file_size)) = file else {
        return Err(unable_to_read());
    };
    if import_type.is_some_and(|t| sub_type.as_deref() != Some(t.as_str())) {
        return Err(unable_to_read());
    }
    let Some(kind) = sub_type.as_deref().and_then(ImportType::parse) else {
        return Err(unsupported());
    };
    let config = config(kind);
    let max_bytes = config.max_file_size_mb * BYTES_PER_MB;
    if file_size.is_some_and(|size| size > max_bytes) {
        return Err(file_too_large(config.max_file_size_mb));
    }
    let Some(format) = resolve_import_file_format(config, &file_name, &mime_type) else {
        return Err(unsupported());
    };
    let bytes = match uploader.get_object(&path).await {
        Ok(bytes) => bytes,
        Err(FilesError::NotFound(_)) => return Err(unable_to_read()),
        Err(error) => return Err(files_err(&error)),
    };
    if i64::try_from(bytes.len()).unwrap_or(i64::MAX) > max_bytes {
        return Err(file_too_large(config.max_file_size_mb));
    }
    if format != "csv" {
        // XLSX headers: the product import slice; JSON has no header row.
        return Err(unable_to_read());
    }
    let text = String::from_utf8_lossy(&bytes);
    let records = csv::parse_records(&text).map_err(|_| unable_to_read())?;
    // `columns: true`: the first record names the columns of the first
    // data record; without a data record there is no row to read.
    if records.len() < 2 {
        return Err(unable_to_read());
    }
    let mut headers: Vec<String> = Vec::new();
    for header in &records[0] {
        if !headers.contains(header) {
            headers.push(header.clone());
        }
    }
    Ok(headers)
}

/// `normalizeContactHeader`: diacritics folded (the NFD marks of Latin
/// letters, `đ` = `d`), lower case, only `[a-z0-9]` kept.
pub fn normalize_contact_header(raw: &str) -> String {
    raw.to_lowercase()
        .chars()
        .map(fold_latin)
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

/// The base letter of a lower-case Latin letter with diacritics (the
/// letters whose NFD form is a base letter plus combining marks).
fn fold_latin(ch: char) -> char {
    const TABLE: &[(char, &str)] = &[
        ('a', "àáâãäåāăąạảấầẩẫậắằẳẵặǎ"),
        ('c', "çćĉċč"),
        ('d', "ďđ"),
        ('e', "èéêëēĕėęěẹẻẽếềểễệ"),
        ('g', "ĝğġģ"),
        ('h', "ĥ"),
        ('i', "ìíîïĩīĭįỉịǐ"),
        ('j', "ĵ"),
        ('k', "ķ"),
        ('l', "ĺļľ"),
        ('n', "ñńņňǹ"),
        ('o', "òóôõöōŏőơọỏốồổỗộớờởỡợǒ"),
        ('r', "ŕŗř"),
        ('s', "śŝşš"),
        ('t', "ţť"),
        ('u', "ùúûüũūŭůűųưụủứừửữựǔ"),
        ('w', "ŵẁẃẅ"),
        ('y', "ýÿŷỳỵỷỹ"),
        ('z', "źżž"),
    ];
    if ch.is_ascii() {
        return ch;
    }
    TABLE
        .iter()
        .find(|(_, variants)| variants.contains(ch))
        .map_or(ch, |(base, _)| *base)
}

/// `HEADER_CANDIDATES` in `contactImportFields` order.
const HEADER_CANDIDATES: &[(&str, &[&str])] = &[
    (
        "contactId",
        &["contactid", "id", "idlienhe", "subscriberid", "psid", "uid"],
    ),
    (
        "phoneNumber",
        &[
            "phonenumber",
            "phone",
            "mobile",
            "mobilephone",
            "tel",
            "telephone",
            "sodienthoai",
            "sdt",
            "dienthoai",
        ],
    ),
    ("email", &["email", "emailaddress", "mail", "diachiemail"]),
    ("firstName", &["firstname", "givenname", "ten"]),
    ("lastName", &["lastname", "surname", "familyname", "ho"]),
    (
        "sourceUserId",
        &["whatsappuserid", "wauserid", "bsuid", "idnguoidungwhatsapp"],
    ),
];

/// `suggestContactImportColumnMap` (`matchContactImportHeaders`): `(field,
/// header)` in `contactImportFields` order; a header is used once.
pub fn suggest_contact_import_column_map(headers: &[String]) -> Vec<(&'static str, String)> {
    let mut used: Vec<&String> = Vec::new();
    let mut out = Vec::new();
    for (field, candidates) in HEADER_CANDIDATES {
        let found = headers.iter().find(|header| {
            !used.contains(header)
                && candidates.contains(&normalize_contact_header(header).as_str())
        });
        if let Some(header) = found {
            used.push(header);
            out.push((*field, header.clone()));
        }
    }
    out
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
    tx::read(db, move |conn| {
        let mut filter = String::from(r#"i."workspaceId" = ?"#);
        let mut binds = vec![Sql::Integer(workspace_id)];
        if let Some(kind) = input.import_type {
            filter.push_str(r#" AND i."type" = ?"#);
            binds.push(Sql::Text(kind.as_str().into()));
        }
        if let Some(status) = &input.status {
            filter.push_str(r#" AND i."status" = ?"#);
            binds.push(Sql::Text(status.clone()));
        }
        if let Some(keyword) = input.keyword.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            filter.push_str(r#" AND f."fileName" LIKE ? ESCAPE '\'"#);
            binds.push(Sql::Text(like_contains(keyword)));
        }
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: input.per_page,
        });
        let parsed = parse_order_by(Import::COLUMNS, input.sort.as_deref(), None);
        let order = if parsed.is_empty() {
            vec![("createdAt".to_string(), chatbotx_db::repo::utils::SortDir::Desc)]
        } else {
            parsed
        };
        let order = order_by_sql(&with_id_tie_break(&order), Some("i"));
        let sql = format!(
            "{SELECT} WHERE {filter} {order} LIMIT {} OFFSET {}",
            pagination.limit, pagination.offset
        );
        let rows = select_rows(conn, &sql, &binds)?;
        let total: i64 = conn
            .query_row(
                &format!(
                    r#"SELECT count(*) FROM "Import" i JOIN "File" f ON f."id" = i."fileId" WHERE {filter}"#
                ),
                params_from_iter(binds.iter()),
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        let page_count = if pagination.limit > 0 {
            (total + pagination.limit - 1) / pagination.limit
        } else {
            0
        };
        Ok((rows, page_count))
    })
    .await
}

/// `importListSelection` (`Import` joined with its `File`).
const SELECT: &str = r#"SELECT i."id", i."workspaceId", i."userId", i."fileId", f."fileName", i."type",
    i."status", i."totalCount", i."processedCount", i."successCount", i."failedCount",
    i."errorMessage", i."errorSample", i."completedAt", i."createdAt", i."updatedAt"
    FROM "Import" i JOIN "File" f ON f."id" = i."fileId""#;

fn select_rows(conn: &Connection, sql: &str, binds: &[Sql]) -> Result<Vec<ImportRow>> {
    let mut stmt = conn.prepare(sql).map_err(repo_err)?;
    let rows = stmt
        .query_map(params_from_iter(binds.iter()), |r| {
            let sample: String = r.get(12)?;
            Ok(ImportRow {
                id: r.get(0)?,
                workspace_id: r.get(1)?,
                user_id: r.get(2)?,
                file_id: r.get(3)?,
                file_name: r.get(4)?,
                import_type: r.get(5)?,
                status: r.get(6)?,
                total_count: r.get(7)?,
                processed_count: r.get(8)?,
                success_count: r.get(9)?,
                failed_count: r.get(10)?,
                error_message: r.get(11)?,
                error_sample: serde_json::from_str(&sample).unwrap_or_else(|_| json!([])),
                completed_at: r.get(13)?,
                created_at: r.get(14)?,
                updated_at: r.get(15)?,
            })
        })
        .map_err(repo_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(repo_err)?;
    Ok(rows)
}

/// `importService.find` (scoped to the workspace and the type).
pub async fn find(
    db: &Db,
    workspace_id: i64,
    id: i64,
    import_type: ImportType,
) -> Result<Option<ImportRow>> {
    tx::read(db, move |conn| {
        let sql = format!(
            r#"{SELECT} WHERE i."id" = ? AND i."workspaceId" = ? AND i."type" = ? LIMIT 1"#
        );
        Ok(select_rows(
            conn,
            &sql,
            &[
                Sql::Integer(id),
                Sql::Integer(workspace_id),
                Sql::Text(import_type.as_str().into()),
            ],
        )?
        .pop())
    })
    .await
}
