//! `apps/builder/src/features/ai-files/api/public.ts` (`aiFilesPublicRouter`)
//! without `aiFiles.search` (it embeds the query: the `packages/ai`
//! retrieval, slice 6).
//!
//! `aiFiles.create` takes JSON (`{ name?, url }`) or `multipart/form-data`
//! (`name`, `file`, `url`). [`Call`] carries no headers, so a body that does
//! not parse as JSON and starts with a `--boundary` line is read as
//! multipart (Q.D6 decision).

use std::sync::Arc;

use axum::http::{HeaderValue, header};
use axum::response::IntoResponse;
use bytes::Bytes;
use chatbotx_business::BusinessError;
use chatbotx_business::ai_file::{
    self as service, AI_FILE_MAX_UPLOAD_BYTES, AiFileDeps, AiFileWithProcessing, CreateAiFileInput,
    ListChunksInput, ReindexConfig,
};
use chatbotx_db::repo::ai_file_embedding::AiFileChunkRow;
use chatbotx_db::repo::now_ms;
use chatbotx_db::repo::partials::ai::parse_ai_file_chunking_config;
use chatbotx_db::repo::utils::PaginationInput;
use chatbotx_files::{InputFile, Uploader};
use chatbotx_kv::rate_limit::{ApiRateLimitInput, assert_api_not_rate_limited};
use serde_json::{Map, Value, json};

use super::input::{Input, Num, Seg, Str, key};
use super::qr_codes::public_paging;
use super::{
    ApiState, CREATING, Call, ErrorMap, FINDING, LISTING, Reply, business_error, date, id, json,
    no_content,
};
use crate::auth::rate_limited_to_orpc;
use crate::orpc::OrpcError;

/// The ported routes.
pub const ROUTES: &[&str] = &[
    "aiFiles.list",
    "aiFiles.get",
    "aiFiles.create",
    "aiFiles.listChunks",
    "aiFiles.reindex",
    "aiFiles.delete",
];

/// `possibleErrorsOnDeletingResource`.
const DELETING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
/// `possibleErrorsOnFindingWithCursor`.
const FINDING_WITH_CURSOR: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("invalidCursor", 400),
];
/// `possibleErrorsOnReindexingResource`.
const REINDEXING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("conflict", 409),
];

/// `AI_FILE_OPERATION_LIMIT`: 5 per 10-second window (about 30 a minute).
pub const AI_FILE_OPERATION_LIMIT: u64 = 5;

/// The bucket scope of `operation` (`search` / `reindex`).
pub fn rate_limit_scope(operation: &str) -> &'static str {
    if operation == "search" {
        "ai-file-search-rate-limit"
    } else {
        "ai-file-reindex-rate-limit"
    }
}

/// `assertAIFile<Operation>NotRateLimited`: `Err((error, retryAfter))`
/// when the workspace's bucket is full. No kv store = no limit.
pub fn check_rate_limit(
    state: &ApiState,
    operation: &str,
    workspace_id: i64,
) -> Result<(), (OrpcError, u64)> {
    let Some(kv) = &state.kv else {
        return Ok(());
    };
    let key = workspace_id.to_string();
    assert_api_not_rate_limited(
        ApiRateLimitInput {
            scope: rate_limit_scope(operation),
            key: &key,
            store: &kv.mem,
            now_ms: now_ms(),
            limit: Some(AI_FILE_OPERATION_LIMIT),
        },
        None,
    )
    .map_err(|limited| (rate_limited_to_orpc(&limited), limited.retry_after))
}

/// `aiFileProcessingRun` + `stale` (as stored), `null` when absent.
fn run_json(run: Option<&Value>) -> Value {
    run.cloned().unwrap_or(Value::Null)
}

/// `publicAIFileResource` (`workspace = false`: `reindexAIFileResponse`
/// without `workspaceId`).
pub fn ai_file_json(row: &AiFileWithProcessing, workspace: bool) -> Value {
    let file = &row.file;
    let config = &row.chunking_config;
    let mut out = json!({
        "id": id(file.id),
        "createdAt": date(file.created_at),
        "updatedAt": date(file.updated_at),
        "name": file.name,
        "path": file.path,
        "size": file.size,
        "mimeType": file.mime_type,
        "chunking": row.chunking.clone().unwrap_or(Value::Null),
        "chunkingConfig": {
            "strategy": config.strategy,
            "chunkSize": config.chunk_size,
            "overlapSize": config.overlap_size,
            "tabularHeaderRow": config.tabular_header_row,
            "isDefault": row.chunking_config_is_default,
        },
        "indexOutdated": row.index_outdated,
        "chunkingOutdated": row.chunking_outdated,
        "processingRun": run_json(row.processing_run.as_ref()),
        "url": row.url,
        "chunksCount": row.chunks_count,
        "processingStatus": row.processing_status,
    });
    if workspace {
        out["workspaceId"] = id(file.workspace_id);
    }
    out
}

/// One `listAIFileChunksResponse` row.
pub fn chunk_json(row: &AiFileChunkRow) -> Value {
    json!({
        "id": id(row.id),
        "chunkIndex": row.chunk_index,
        "content": row.content,
        "status": row.status,
        "createdAt": date(row.created_at),
    })
}

// ---------------------------------------------------------------------------
// Request schemas
// ---------------------------------------------------------------------------

/// One `multipart/form-data` part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn header_param(line: &str, name: &str) -> Option<String> {
    line.split(';').skip(1).find_map(|param| {
        let (key, value) = param.trim().split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case(name)
            .then(|| value.trim().trim_matches('"').to_owned())
    })
}

/// Parses a `multipart/form-data` body whose boundary is its first line
/// (`--<boundary>`). `None` = not multipart.
pub fn parse_multipart(body: &[u8]) -> Option<Vec<Part>> {
    let first_end = find(body, b"\r\n", 0)?;
    let first = body.get(..first_end)?;
    if !first.starts_with(b"--") || first.len() <= 2 {
        return None;
    }
    let delimiter = [b"\r\n".as_slice(), first].concat();
    let mut parts = Vec::new();
    let mut at = first_end + 2;
    loop {
        let head_end = find(body, b"\r\n\r\n", at)?;
        let head = std::str::from_utf8(body.get(at..head_end)?).ok()?;
        let data_start = head_end + 4;
        let data_end = find(body, &delimiter, data_start)?;
        let (mut name, mut filename, mut content_type) = (None, None, None);
        for line in head.split("\r\n") {
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("content-disposition:") {
                name = header_param(line, "name");
                filename = header_param(line, "filename");
            } else if let Some(value) = lower.strip_prefix("content-type:") {
                content_type = Some(value.trim().to_owned());
            }
        }
        parts.push(Part {
            name: name?,
            filename,
            content_type,
            data: body.get(data_start..data_end)?.to_vec(),
        });
        at = data_end + delimiter.len();
        if body.get(at..at + 2) == Some(b"--") {
            return Some(parts);
        }
        if body.get(at..at + 2) != Some(b"\r\n") {
            return None;
        }
        at += 2;
    }
}

/// The `createAIFilePublicRequest` body: the fields and an uploaded file.
pub struct CreateBody {
    pub fields: Map<String, Value>,
    pub file: Option<InputFile>,
    /// A `file` field that is not an upload (a JSON value or a text part).
    pub file_not_a_file: bool,
}

/// JSON first, then multipart; anything else is an empty object.
pub fn create_body(body: &[u8]) -> CreateBody {
    if let Ok(Value::Object(fields)) = serde_json::from_slice::<Value>(body) {
        let file_not_a_file = fields.contains_key("file");
        return CreateBody {
            fields,
            file: None,
            file_not_a_file,
        };
    }
    let mut out = CreateBody {
        fields: Map::new(),
        file: None,
        file_not_a_file: false,
    };
    for part in parse_multipart(body).unwrap_or_default() {
        match (part.name.as_str(), part.filename) {
            ("file", Some(filename)) => {
                out.file = Some(InputFile {
                    name: filename,
                    mime_type: part.content_type.unwrap_or_default(),
                    bytes: Bytes::from(part.data),
                });
            }
            ("file", None) => out.file_not_a_file = true,
            (name, _) => {
                out.fields.insert(
                    name.to_owned(),
                    Value::String(String::from_utf8_lossy(&part.data).into_owned()),
                );
            }
        }
    }
    out
}

/// `createAIFilePublicRequest`: `name` (trimmed, min 1), `file` (up to
/// 100 MB) or `url` (`z.url()`), exactly one of the two.
pub fn parse_create(input: &mut Input, body: CreateBody) -> Option<CreateAiFileInput> {
    let name = input.opt_string("name", Str::trimmed(1, usize::MAX));
    if body.file_not_a_file {
        input.issue(&key("file"), "custom", "Input not instance of File");
    }
    if let Some(file) = &body.file
        && file.bytes.len() as u64 > AI_FILE_MAX_UPLOAD_BYTES
    {
        input.issue(&key("file"), "custom", "Max file size is 100MB.");
    }
    let url = input.opt_string("url", Str::default());
    if let Some(text) = &url
        && url::Url::parse(text).is_err()
    {
        input.issue(&key("url"), "invalid_format", "Invalid URL");
    }
    if input.failed() {
        return None;
    }
    match (body.file, url) {
        (Some(_), Some(_)) => {
            input.issue(&[], "custom", "Provide either 'file' or 'url', not both.");
            None
        }
        (None, None) => {
            input.issue(&[], "custom", "Provide either 'file' or 'url'.");
            None
        }
        (Some(file), None) => Some(CreateAiFileInput::File { name, file }),
        (None, Some(url)) => Some(CreateAiFileInput::Url { name, url }),
    }
}

/// `aiFileChunkingConfig.nullable().optional()` at `chunkingConfig`.
pub fn parse_reindex_config(input: &mut Input) -> ReindexConfig {
    match input.get("chunkingConfig").cloned() {
        None => ReindexConfig::Keep,
        Some(Value::Null) => ReindexConfig::Reset,
        Some(value) => match parse_ai_file_chunking_config(&value) {
            Ok(_) => ReindexConfig::Set(value),
            Err(error) => {
                for issue in error.issues {
                    let mut path = key("chunkingConfig");
                    if !issue.path.is_empty() {
                        path.push(Seg::Key(issue.path.clone()));
                    }
                    input.issue(&path, "invalid_value", &issue.message);
                }
                ReindexConfig::Keep
            }
        },
    }
}

/// `listAIFileChunksRequest` (without the id).
pub fn parse_list_chunks(input: &mut Input) -> (i64, Option<String>, Option<i64>) {
    let limit = input.opt_coerce_int("limit", Num::int(1.0, 100.0));
    let cursor = input.opt_string(
        "cursor",
        Str {
            min: Some(1),
            ..Str::default()
        },
    );
    let from = input.opt_coerce_int(
        "fromChunkIndex",
        Num {
            min: Some(0.0),
            ..Num::default()
        },
    );
    (limit.unwrap_or(50), cursor, from)
}

fn storage(state: &ApiState) -> Result<&Arc<Uploader>, OrpcError> {
    state.files.as_ref().ok_or_else(|| {
        OrpcError::new(
            "INTERNAL_SERVER_ERROR",
            Some(500),
            Some("File storage is not configured"),
        )
    })
}

/// The deps of a call.
pub fn deps<'a>(state: &'a ApiState, files: &'a Arc<Uploader>) -> AiFileDeps<'a> {
    AiFileDeps {
        db: &state.db,
        files,
        jobs: state.jobs.as_ref(),
        audit: &state.audit,
        fetcher: None,
    }
}

/// A 429 with `Retry-After`.
fn rate_limited(error: OrpcError, retry_after: u64) -> Reply {
    let mut response = error.into_response();
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from(retry_after));
    Ok(response)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "aiFiles.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let files = storage(state)?;
            let page = service::list(
                db,
                files,
                ws,
                PaginationInput {
                    page: Some(page),
                    per_page: Some(per_page),
                },
            )
            .await
            .map_err(|e| business_error(e, LISTING))?;
            let data: Vec<Value> = page.data.iter().map(|r| ai_file_json(r, true)).collect();
            Ok(json(
                200,
                json!({ "data": data, "pageCount": page.page_count }),
            ))
        }
        "aiFiles.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let file_id = input.id("id");
            input.finish()?;
            let files = storage(state)?;
            match service::find_with_processing(db, files, ws, file_id.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, FINDING))?
            {
                Some(row) => Ok(json(200, ai_file_json(&row, true))),
                None => Err(business_error(
                    BusinessError::not_found("AI file not found"),
                    FINDING,
                )),
            }
        }
        "aiFiles.create" => {
            let body = create_body(call.body);
            let mut fields = body_fields(call.params, &body.fields);
            let mut input = Input::from_object(std::mem::take(&mut fields));
            let upload = parse_create(&mut input, body);
            input.finish()?;
            let files = storage(state)?;
            let upload = upload.ok_or_else(|| {
                OrpcError::new("BAD_REQUEST", Some(400), Some("Input validation failed"))
            })?;
            service::create(&deps(state, files), ws, upload)
                .await
                .map(|row| json(201, ai_file_json(&row, true)))
                .map_err(|e| business_error(e, CREATING))
        }
        "aiFiles.listChunks" => {
            let mut input = Input::from_query(call.params, call.query);
            let file_id = input.id("id");
            let (limit, cursor, from_chunk_index) = parse_list_chunks(&mut input);
            input.finish()?;
            let page = service::list_chunks(
                db,
                ListChunksInput {
                    workspace_id: ws,
                    ai_file_id: file_id.unwrap_or_default(),
                    limit,
                    cursor,
                    from_chunk_index,
                },
            )
            .await
            .map_err(|e| business_error(e, FINDING_WITH_CURSOR))?;
            let data: Vec<Value> = page.data.iter().map(chunk_json).collect();
            Ok(json(
                200,
                json!({ "data": data, "nextCursor": page.next_cursor }),
            ))
        }
        "aiFiles.reindex" => {
            let mut input = Input::from_body(call.params, call.body);
            let file_id = input.id("id");
            let config = parse_reindex_config(&mut input);
            input.finish()?;
            if let Err((error, retry_after)) = check_rate_limit(state, "reindex", ws) {
                return rate_limited(error, retry_after);
            }
            let files = storage(state)?;
            let out =
                service::reindex(&deps(state, files), ws, file_id.unwrap_or_default(), config)
                    .await
                    .map_err(|e| business_error(e, REINDEXING))?;
            let mut body = ai_file_json(&out.file, false);
            body["coalesced"] = json!(out.coalesced);
            Ok(json(202, body))
        }
        "aiFiles.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let file_id = input.id("id");
            input.finish()?;
            let files = storage(state)?;
            service::delete(&deps(state, files), ws, file_id.unwrap_or_default())
                .await
                .map(|()| no_content())
                .map_err(|e| business_error(e, DELETING))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// The path params merged over the body fields (as `Input::from_body`).
fn body_fields(
    params: &[(&'static str, String)],
    fields: &Map<String, Value>,
) -> Map<String, Value> {
    let mut out = fields.clone();
    for (name, value) in params {
        out.insert((*name).to_owned(), Value::String(value.clone()));
    }
    out
}
