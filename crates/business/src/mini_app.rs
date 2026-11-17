//! `packages/business/src/mini-app`: Mini App CRUD (`service.ts`,
//! `definition.ts`, `names.ts`), submissions (`submission-service.ts`),
//! uploads (`upload-service.ts`) and the publication rows
//! (`publication-service.ts`). The definition logic is
//! `chatbotx_core::mini_app`.

use std::collections::{BTreeMap, HashSet};

use chatbotx_core::mini_app::{
    self as app, Definition, UPLOAD_SNIFF_BYTES, apply_custom_field_mappings, build_upload_path,
    collect_custom_field_mappings, collect_file_input_names, collect_input_names, find_screen,
    format_answer_for_custom_field, resolve_upload_rules, sanitize_answers, verify_upload,
};
use chatbotx_core::utils::sanitize_file_name::sanitize_file_name;
use chatbotx_core::utils::storage::get_public_file_url;
use chatbotx_db::Db;
use chatbotx_db::repo::mini_app::{self as repo, MiniAppListInput, MiniAppListRow, MiniAppWithPublications};
use chatbotx_db::repo::models::{MiniApp, MiniAppPublication, MiniAppSubmission};
use chatbotx_db::repo::{Json, Model as _, SortItem, is_unique_violation_error, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_files::{PutOptions, Uploader};
use serde_json::{Value, json};

use crate::contact_custom_field::{FieldChange, SetValuesInput, set_values_in_transaction};
use crate::copy_name::{CopyNameOptions, CopyNameStyle, copy_name_prefix, insert_with_free_copy_name};
use crate::errors::{BusinessError, Result};
use crate::tx;

pub const NAME_TAKEN: &str = "Name is already taken";
const COPY_NAME: CopyNameOptions = CopyNameOptions { style: CopyNameStyle::Dash, max_length: app::NAME_MAX_LENGTH };

fn db_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn not_found() -> BusinessError {
    BusinessError::not_found("Mini App not found")
}

/// `rethrowNameTaken`.
fn name_taken(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    let error = chatbotx_db::repo::Error::from(error);
    if is_unique_violation_error(&error, None) {
        BusinessError::validation("name", NAME_TAKEN)
    } else {
        BusinessError::Repo(error)
    }
}

/// `MiniAppDefinitionInput`: where a write takes the screens from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DefinitionInput {
    /// The editor's definition, stored as is.
    pub definition: Option<Value>,
    /// Meta Flow JSON; replaces the whole app.
    pub flow_json: Option<Value>,
    /// Input name → custom field id; `None` removes a mapping.
    pub custom_field_mappings: Option<BTreeMap<String, Option<String>>>,
}

/// `parseDefinition`.
pub fn parse_definition(value: &Value) -> Result<Definition> {
    app::parse_definition(value).ok_or_else(|| BusinessError::validation("definition", "Invalid Mini App definition"))
}

/// `importMiniAppFlowJson`: errors carry `{ code, path }` on `flowJson`.
pub fn import_flow_json(flow_json: &Value) -> Result<Definition> {
    match app::from_flow_json(flow_json) {
        Ok(definition) => parse_definition(&definition),
        Err(error) => Err(BusinessError::Exception {
            message: "Unsupported Flow JSON".into(),
            code: "validation".into(),
            status: 422,
            field: Some("flowJson".into()),
            data: Some(json!({ "code": error.code, "path": error.path })),
        }),
    }
}

fn incoming(input: &DefinitionInput) -> Result<Option<Definition>> {
    if let Some(flow_json) = &input.flow_json {
        return import_flow_json(flow_json).map(Some);
    }
    input.definition.as_ref().map(parse_definition).transpose()
}

/// `buildNewDefinition`.
pub fn build_new_definition(input: &DefinitionInput) -> Result<Definition> {
    let mut definition = match incoming(input)? {
        Some(definition) => definition,
        None => parse_definition(&Value::Null)?,
    };
    apply_custom_field_mappings(&mut definition, &input.custom_field_mappings.clone().unwrap_or_default());
    Ok(definition)
}

/// `buildUpdatedDefinition`: `None` when the update brings no screens and
/// no mappings. A new Flow JSON keeps the current mappings by input name.
pub fn build_updated_definition(current: &Value, input: &DefinitionInput) -> Result<Option<Definition>> {
    let incoming = incoming(input)?;
    if incoming.is_none() && input.custom_field_mappings.is_none() {
        return Ok(None);
    }
    let current = app::parse_definition(current);
    let mut mappings: BTreeMap<String, Option<String>> = BTreeMap::new();
    if input.flow_json.is_some()
        && let Some(current) = &current
    {
        mappings.extend(collect_custom_field_mappings(current).into_iter().map(|(k, v)| (k, Some(v))));
    }
    mappings.extend(input.custom_field_mappings.clone().unwrap_or_default());
    let Some(mut definition) = incoming.or(current) else {
        return Err(BusinessError::validation("definition", "Invalid Mini App definition"));
    };
    apply_custom_field_mappings(&mut definition, &mappings);
    Ok(Some(definition))
}

/// `miniAppService.list` (newest first): rows and `pageCount`.
pub async fn list(
    db: &Db,
    workspace_id: i64,
    keyword: Option<String>,
    page: i64,
    per_page: i64,
) -> Result<(Vec<MiniAppListRow>, i64)> {
    tx::read(db, move |conn| {
        let input = MiniAppListInput {
            workspace_id,
            keyword: keyword.clone(),
            page,
            per_page,
            sort: Some(vec![SortItem { id: "createdAt".into(), desc: true }]),
        };
        let rows = repo::list_paginated(conn, &input)?;
        let total = repo::count(conn, workspace_id, keyword.as_deref())?;
        Ok((rows, (total + per_page - 1) / per_page.max(1)))
    })
    .await
}

/// `miniAppService.findOrFail` (with the publications).
pub async fn find_or_fail(db: &Db, workspace_id: i64, id: i64) -> Result<MiniAppWithPublications> {
    tx::read(db, move |conn| repo::find_by_id_and_workspace(conn, workspace_id, id)?.ok_or_else(not_found)).await
}

/// `miniAppService.findUnscoped`.
pub async fn find_unscoped(db: &Db, id: i64) -> Result<Option<MiniApp>> {
    tx::read(db, move |conn| Ok(repo::find_by_id(conn, id)?)).await
}

fn insert(conn: &Connection, workspace_id: i64, name: &str, enabled: bool, definition: &Definition) -> std::result::Result<MiniApp, chatbotx_db::rusqlite::Error> {
    let id = new_id();
    let now = now_ms();
    conn.execute(
        r#"INSERT INTO "MiniApp" ("id", "createdAt", "updatedAt", "workspaceId", "name", "enabled", "definition", "flowJson", "submissionsCount")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, 0)"#,
        params![
            id,
            now,
            workspace_id,
            name,
            enabled,
            definition.to_value().to_string(),
            app::to_flow_json(definition).to_string()
        ],
    )?;
    find_row(conn, id)
}

fn find_row(conn: &Connection, id: i64) -> std::result::Result<MiniApp, chatbotx_db::rusqlite::Error> {
    conn.query_row(
        &format!(r#"SELECT {} FROM "MiniApp" WHERE "id" = ?1"#, MiniApp::columns_sql(None)),
        params![id],
        MiniApp::from_row,
    )
}

/// `miniAppService.create`.
pub async fn create(db: &Db, workspace_id: i64, name: String, input: DefinitionInput) -> Result<MiniApp> {
    let definition = build_new_definition(&input)?;
    tx::write(db, move |tx| insert(tx, workspace_id, &name, true, &definition).map_err(name_taken)).await
}

/// `miniAppService.duplicate`: `<name> - Copy` (then ` - Copy 2`, …);
/// submissions and publications stay with the original.
pub async fn duplicate(db: &Db, workspace_id: i64, id: i64) -> Result<MiniApp> {
    let source = find_or_fail(db, workspace_id, id).await?.mini_app;
    tx::write(db, move |tx| {
        let prefix = copy_name_prefix(&source.name, COPY_NAME);
        let taken: HashSet<String> = repo::list_names_with_prefix(tx, workspace_id, &prefix)?.into_iter().collect();
        let definition = parse_definition(&source.definition.0)?;
        let copy = insert_with_free_copy_name(
            &source.name,
            &taken,
            COPY_NAME,
            None,
            |e: &chatbotx_db::rusqlite::Error, _| {
                matches!(e, chatbotx_db::rusqlite::Error::SqliteFailure(f, _)
                    if f.extended_code == chatbotx_db::rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE)
            },
            |name| insert(tx, workspace_id, name, source.enabled, &definition),
        )
        .map_err(db_error)?;
        copy.ok_or_else(|| BusinessError::validation("name", NAME_TAKEN))
    })
    .await
}

/// `miniAppService.update`: name, enabled state and / or screens.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    name: Option<String>,
    enabled: Option<bool>,
    input: DefinitionInput,
) -> Result<MiniApp> {
    let current = find_or_fail(db, workspace_id, id).await?.mini_app;
    let definition = build_updated_definition(&current.definition.0, &input)?;
    if name.is_none() && enabled.is_none() && definition.is_none() {
        return Ok(current);
    }
    tx::write(db, move |tx| {
        let (definition, flow_json) = match &definition {
            Some(d) => (Some(d.to_value().to_string()), Some(app::to_flow_json(d).to_string())),
            None => (None, None),
        };
        tx.execute(
            r#"UPDATE "MiniApp" SET "name" = COALESCE(?3, "name"), "enabled" = COALESCE(?4, "enabled"),
                 "definition" = COALESCE(?5, "definition"), "flowJson" = COALESCE(?6, "flowJson"), "updatedAt" = ?7
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id, name, enabled, definition, flow_json, now_ms()],
        )
        .map_err(name_taken)?;
        find_row(tx, id).map_err(db_error)
    })
    .await
}

/// `miniAppService.deleteMany`: the number of deleted rows.
pub async fn delete_many(db: &Db, workspace_id: i64, ids: Vec<i64>) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    tx::write(db, move |tx| {
        let mut deleted = 0;
        for id in &ids {
            deleted += tx
                .execute(r#"DELETE FROM "MiniApp" WHERE "id" = ?1 AND "workspaceId" = ?2"#, params![id, workspace_id])
                .map_err(db_error)?;
        }
        Ok(deleted)
    })
    .await
}

// ------------------------------------------------------- submissions

/// `miniAppSubmissionService.list`: rows, `pageCount`, `totalRows`.
pub async fn list_submissions(
    db: &Db,
    workspace_id: i64,
    mini_app_id: i64,
    page: i64,
    per_page: i64,
) -> Result<(Vec<repo::SubmissionWithContact>, i64, i64)> {
    tx::read(db, move |conn| {
        let rows = repo::list_submissions(conn, workspace_id, mini_app_id, page, per_page)?;
        let total = repo::count_submissions(conn, workspace_id, mini_app_id)?;
        Ok((rows, (total + per_page - 1) / per_page.max(1), total))
    })
    .await
}

/// `findVisitorContactId`: the token's contact, if it still exists.
fn visitor_contact(conn: &Connection, workspace_id: i64, contact_id: Option<i64>) -> Result<Option<i64>> {
    let Some(contact_id) = contact_id else { return Ok(None) };
    conn.query_row(
        r#"SELECT "id" FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        params![contact_id, workspace_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(db_error)
}

fn custom_field_values(definition: &Definition, answers: &serde_json::Map<String, Value>) -> Vec<(i64, String)> {
    collect_custom_field_mappings(definition)
        .into_iter()
        .filter_map(|(name, field_id)| {
            let value = format_answer_for_custom_field(answers.get(&name).unwrap_or(&Value::Null))?;
            Some((field_id.parse().ok()?, value))
        })
        .collect()
}

/// `writeFieldsSkippingRejected`: one savepoint for all fields; on a
/// failure, one per field (a rejected field is logged and skipped).
fn write_custom_fields(
    tx: &Connection,
    workspace_id: i64,
    contact_id: i64,
    fields: Vec<(i64, String)>,
    source_timezone: Option<String>,
) -> Result<Vec<FieldChange>> {
    if fields.is_empty() {
        return Ok(Vec::new());
    }
    let write = |fields: Vec<(i64, String)>| -> Result<Vec<FieldChange>> {
        tx.execute_batch("SAVEPOINT mini_app_fields").map_err(db_error)?;
        let input = SetValuesInput {
            workspace_id,
            contact_id,
            fields,
            source_timezone: source_timezone.clone(),
            ..SetValuesInput::default()
        };
        match set_values_in_transaction(tx, &input, now_ms()) {
            Ok(changes) => {
                tx.execute_batch("RELEASE mini_app_fields").map_err(db_error)?;
                Ok(changes)
            }
            Err(error) => {
                tx.execute_batch("ROLLBACK TO mini_app_fields; RELEASE mini_app_fields").map_err(db_error)?;
                Err(error)
            }
        }
    };
    if let Ok(changes) = write(fields.clone()) {
        return Ok(changes);
    }
    let mut written = Vec::new();
    for field in fields {
        let custom_field_id = field.0;
        match write(vec![field]) {
            Ok(changes) => written.extend(changes),
            Err(error) => tracing::warn!(
                contact_id,
                custom_field_id,
                error = %error.message(),
                "Mini App answer not written to its custom field"
            ),
        }
    }
    Ok(written)
}

/// A stored submission and the custom field changes to emit.
pub struct Submitted {
    pub submission: MiniAppSubmission,
    pub changes: Vec<FieldChange>,
}

/// `miniAppSubmissionService.create`: sanitized answers, file answers
/// claimed, the count bumped, then the mapped custom fields written.
pub async fn create_submission(
    db: &Db,
    storage_url: &str,
    mini_app: &MiniApp,
    contact_id: Option<i64>,
    answers: Value,
    source_timezone: Option<String>,
) -> Result<Submitted> {
    let definition = parse_definition(&mini_app.definition.0)?;
    let answers = sanitize_answers(&definition, &answers).filter(|a| !a.is_empty());
    let mut answers = answers.ok_or_else(|| BusinessError::validation("answers", "No valid answers"))?;
    let (workspace_id, mini_app_id) = (mini_app.workspace_id, mini_app.id);
    let storage_url = storage_url.to_owned();
    tx::write(db, move |tx| {
        let contact_id = visitor_contact(tx, workspace_id, contact_id)?;
        let submission_id = new_id();
        tx.execute(
            r#"INSERT INTO "MiniAppSubmission" ("id", "createdAt", "updatedAt", "workspaceId", "miniAppId", "contactId", "source", "answers")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, 'web', '{}')"#,
            params![submission_id, now_ms(), workspace_id, mini_app_id, contact_id],
        )
        .map_err(db_error)?;
        let file_names = collect_file_input_names(&definition);
        for (name, value) in answers.iter_mut() {
            if file_names.contains(name)
                && let Value::Array(ids) = value
            {
                let ids: Vec<String> = ids.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
                *value = claim_uploads(tx, &storage_url, &definition, mini_app_id, contact_id, submission_id, name, ids)?;
            }
        }
        tx.execute(
            r#"UPDATE "MiniAppSubmission" SET "answers" = ?2 WHERE "id" = ?1"#,
            params![submission_id, Value::Object(answers.clone()).to_string()],
        )
        .map_err(db_error)?;
        tx.execute(
            r#"UPDATE "MiniApp" SET "submissionsCount" = "submissionsCount" + 1 WHERE "id" = ?1"#,
            params![mini_app_id],
        )
        .map_err(db_error)?;
        let submission = tx
            .query_row(
                &format!(r#"SELECT {} FROM "MiniAppSubmission" WHERE "id" = ?1"#, MiniAppSubmission::columns_sql(None)),
                params![submission_id],
                MiniAppSubmission::from_row,
            )
            .map_err(db_error)?;
        // ponytail: the original writes the fields in a second transaction
        // after the submission commits; one transaction here.
        let changes = match contact_id {
            Some(contact) => write_custom_fields(
                tx,
                workspace_id,
                contact,
                custom_field_values(&definition, &answers),
                source_timezone,
            )?,
            None => Vec::new(),
        };
        Ok(Submitted { submission, changes })
    })
    .await
}

/// `miniAppSubmissionService.saveScreenAnswers`: writes one screen's
/// mapped answers (no files) for a known contact; `None` when the screen
/// or the contact is unknown.
pub async fn save_screen_answers(
    db: &Db,
    mini_app: &MiniApp,
    contact_id: Option<i64>,
    screen_key: String,
    answers: Value,
    source_timezone: Option<String>,
) -> Result<Option<Vec<FieldChange>>> {
    let definition = parse_definition(&mini_app.definition.0)?;
    let workspace_id = mini_app.workspace_id;
    tx::write(db, move |tx| {
        let Some(screen) = find_screen(&definition, &screen_key) else { return Ok(None) };
        let Some(contact_id) = visitor_contact(tx, workspace_id, contact_id)? else { return Ok(None) };
        let screen_names: HashSet<String> = collect_input_names(screen).into_iter().collect();
        let file_names = collect_file_input_names(&definition);
        let mut screen_answers = sanitize_answers(&definition, &answers).unwrap_or_default();
        screen_answers.retain(|name, _| screen_names.contains(name) && !file_names.contains(name));
        let fields = custom_field_values(&definition, &screen_answers);
        write_custom_fields(tx, workspace_id, contact_id, fields, source_timezone).map(Some)
    })
    .await
}

// ----------------------------------------------------------- uploads

const MAX_ORIGINAL_NAME_LENGTH: usize = 200;

/// `randomToken`: 16 random bytes, base64url without padding (22 chars).
fn random_token() -> String {
    use base64::Engine as _;
    let mut bytes = [0u8; 16];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn url_of(path: &str, storage_url: &str) -> String {
    get_public_file_url(path, storage_url).unwrap_or_else(|_| format!("{storage_url}{path}"))
}

/// `MiniAppUploadError`: 413 for `too_large`, else 422.
pub fn upload_error(reason: &str) -> BusinessError {
    let status = if reason == "too_large" { 413 } else { 422 };
    BusinessError::Exception {
        message: format!("Upload rejected: {reason}"),
        code: "miniAppUploadRejected".into(),
        status,
        field: None,
        data: Some(json!({ "reason": reason })),
    }
}

/// An uploaded file of the public upload route.
pub struct UploadedFile {
    pub name: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

/// `miniAppUploadService.create`: checks the file against the input's
/// rules, stores it, records a pending upload. `{ uploadId, name, size,
/// mimeType, url }`.
pub async fn create_upload(
    db: &Db,
    uploader: &Uploader,
    storage_url: &str,
    mini_app: &MiniApp,
    contact_id: Option<i64>,
    input_name: String,
    file: UploadedFile,
) -> Result<Value> {
    let definition = parse_definition(&mini_app.definition.0)?;
    let rules = resolve_upload_rules(&definition, &input_name).ok_or_else(|| upload_error("unknown_input"))?;
    let size = file.bytes.len();
    let head = &file.bytes[..size.min(UPLOAD_SNIFF_BYTES)];
    let mime_type = verify_upload(&rules, size, head, &file.mime_type).map_err(upload_error)?;
    let workspace_id = mini_app.workspace_id;
    let contact_id = tx::read(db, move |conn| visitor_contact(conn, workspace_id, contact_id)).await?;
    let file_name = sanitize_file_name(&file.name);
    let path = build_upload_path(
        &workspace_id.to_string(),
        &mini_app.id.to_string(),
        contact_id.map(|c| c.to_string()).as_deref(),
        &random_token(),
        &file_name,
    );
    let options = PutOptions {
        acl: Some(chatbotx_files::DEFAULT_ACL),
        content_type: Some(mime_type.clone()),
        content_length: Some(size as u64),
        content_disposition: (!mime_type.starts_with("image/")).then(|| format!("attachment; filename=\"{file_name}\"")),
        ..PutOptions::default()
    };
    uploader
        .put_object(&path, file.bytes, options, None)
        .await
        .map_err(|e| BusinessError::Repo(chatbotx_db::repo::Error::msg(e.to_string())))?;
    let upload_token = random_token();
    let original: String = file.name.chars().take(MAX_ORIGINAL_NAME_LENGTH).collect();
    let original = if original.is_empty() { file_name } else { original };
    let (mini_app_id, row_path, row_name, row_mime, token) =
        (mini_app.id, path.clone(), original.clone(), mime_type.clone(), upload_token.clone());
    tx::write(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "MiniAppUpload" ("id", "createdAt", "updatedAt", "workspaceId", "miniAppId", "contactId", "submissionId",
                 "inputName", "uploadToken", "path", "fileName", "mimeType", "size", "status")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, 'pending')"#,
            params![new_id(), now_ms(), workspace_id, mini_app_id, contact_id, input_name, token, row_path, row_name, row_mime, size as i64],
        )
        .map_err(db_error)?;
        Ok(())
    })
    .await?;
    Ok(json!({
        "uploadId": upload_token,
        "name": original,
        "size": size,
        "mimeType": mime_type,
        "url": url_of(&path, storage_url),
    }))
}

/// `miniAppUploadService.claimForSubmission`: the pending uploads of this
/// visitor become the answer's files.
#[allow(clippy::too_many_arguments)]
fn claim_uploads(
    tx: &Connection,
    storage_url: &str,
    definition: &Definition,
    mini_app_id: i64,
    contact_id: Option<i64>,
    submission_id: i64,
    input_name: &str,
    upload_ids: Vec<String>,
) -> Result<Value> {
    let invalid = || BusinessError::validation("answers", "Invalid uploads");
    let rules = resolve_upload_rules(definition, input_name).ok_or_else(invalid)?;
    let mut unique: Vec<String> = Vec::new();
    for id in upload_ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    if unique.len() > rules.max_files {
        return Err(invalid());
    }
    let mut files = Vec::new();
    for token in &unique {
        let row = tx
            .query_row(
                r#"SELECT "id", "path", "fileName", "mimeType", "size" FROM "MiniAppUpload"
                   WHERE "miniAppId" = ?1 AND "inputName" = ?2 AND "status" = 'pending' AND "uploadToken" = ?3
                     AND "contactId" IS ?4"#,
                params![mini_app_id, input_name, token, contact_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?)),
            )
            .optional()
            .map_err(db_error)?;
        let (id, path, name, mime_type, size) = row.ok_or_else(invalid)?;
        tx.execute(
            r#"UPDATE "MiniAppUpload" SET "status" = 'submitted', "submissionId" = ?2, "updatedAt" = ?3 WHERE "id" = ?1"#,
            params![id, submission_id, now_ms()],
        )
        .map_err(db_error)?;
        files.push(json!({ "uploadId": token, "url": url_of(&path, storage_url), "name": name, "mimeType": mime_type, "size": size }));
    }
    Ok(Value::Array(files))
}

// ------------------------------------------------------ publications

/// `miniAppPublicationService.listForMiniApps`.
pub async fn list_publications(db: &Db, workspace_id: i64, mini_app_ids: Vec<i64>) -> Result<Vec<MiniAppPublication>> {
    tx::read(db, move |conn| {
        let mut out = Vec::new();
        for id in mini_app_ids {
            if let Some(app) = repo::find_by_id_and_workspace(conn, workspace_id, id)? {
                out.extend(app.publications);
            }
        }
        Ok(out)
    })
    .await
}

/// The stored mapping JSON of an id (`Json` row helper for callers).
pub fn definition_of(row: &MiniApp) -> &Value {
    let Json(value) = &row.definition;
    value
}
