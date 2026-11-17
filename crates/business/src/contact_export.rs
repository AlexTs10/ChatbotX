//! Contact export (P3.6-D2 slice contact export): `contactExportService`
//! `start` / `getFile` / `listFiles` and the worker handler
//! `loopableExportContacts` (`export-contacts.ts`: selected fields, CSV
//! rows, the base where, the keyset page loop).
//!
//! No default-queue worker is ported: the API runs [`run_export`] in the
//! request after [`start`] (the `bulkTagContacts` precedent), so the
//! throttled mid-run progress write is not needed. No audit record (the
//! token path has no user). `purgeExpired` and its schedule are not ported
//! (no scheduler).

use std::collections::{BTreeMap, HashMap};

use chatbotx_core::utils::csv::{CsvCell, CsvDialect, UTF8_BOM, to_csv_row};
use chatbotx_core::utils::datetime::format_custom_field_value_in_time_zone;
use chatbotx_core::variables::system_field::{CONTACT_AD_IDS_LIMIT, normalize_stored_timezone};
use chatbotx_db::Db;
use chatbotx_db::repo::contact_referral_event::list_ad_ids_by_contacts;
use chatbotx_db::repo::queries::contact_filter::{
    FilterCriteria, apply_contact_filter, prune_email_phone_filter_conditions,
};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::{iso, new_id, now_ms};
use chatbotx_db::rusqlite::types::ValueRef;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};
use chatbotx_files::{PutOptions, Uploader};
use serde_json::Value;

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `CONTACT_EXPORT_RETENTION_DAYS`.
pub const CONTACT_EXPORT_RETENTION_DAYS: i64 = 7;
const RETENTION_MS: i64 = CONTACT_EXPORT_RETENTION_DAYS * 24 * 60 * 60 * 1000;
/// `loopableItemsCount` (the page size of the keyset loop).
pub const EXPORT_PAGE_SIZE: usize = 100;
/// `MAX_EXPORT_ROWS`.
const MAX_EXPORT_ROWS: usize = 10_000_000;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `stripContactPIIFields`.
pub fn strip_contact_pii_fields(fields: &[String], can_view_pii: bool) -> Vec<String> {
    fields
        .iter()
        .filter(|f| can_view_pii || !matches!(f.as_str(), "sys:email" | "sys:phoneNumber"))
        .cloned()
        .collect()
}

/// The kind of one selected column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldKind {
    Contact,
    /// A custom field and its `customFieldType`.
    Custom(String),
    Tag,
}

/// `SelectedField`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedField {
    pub kind: FieldKind,
    pub value: String,
    pub header: String,
}

/// One contact row of a page (`ContactRow`): `columns` holds the rendered
/// `Contact` columns (a date as ISO text, a boolean as `true`/`false`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExportContact {
    pub id: i64,
    pub columns: HashMap<String, String>,
    /// `(customFieldId, value)`.
    pub custom_fields: Vec<(String, String)>,
    pub tag_ids: Vec<String>,
    /// `(sourceId, sourceUserId)` in id order.
    pub inboxes: Vec<(Option<String>, Option<String>)>,
    pub ad_ids: Vec<String>,
}

fn header_name(column: &str) -> Option<&'static str> {
    Some(match column {
        "contactId" => "Contact ID",
        "firstName" => "First Name",
        "lastName" => "Last Name",
        "fullName" => "Full Name",
        "email" => "Email",
        "phoneNumber" => "Phone Number",
        "gender" => "Gender",
        "source" => "Source",
        "lastReadAt" => "Last Read At",
        "blockedAt" => "Blocked At",
        "sourceUserId" => "WhatsApp User ID",
        "adIds" => "Ad IDs",
        _ => return None,
    })
}

fn contact_value(contact: &ExportContact, column: &str) -> String {
    match column {
        "contactId" => contact
            .inboxes
            .first()
            .and_then(|(source, _)| source.clone())
            .unwrap_or_default(),
        "sourceUserId" => contact
            .inboxes
            .iter()
            .find_map(|(_, user)| user.clone().filter(|u| !u.is_empty()))
            .unwrap_or_default(),
        "adIds" => contact.ad_ids.join(", "),
        _ => contact.columns.get(column).cloned().unwrap_or_default(),
    }
}

fn cell_value(contact: &ExportContact, field: &SelectedField, timezone: &str) -> String {
    match &field.kind {
        FieldKind::Contact => contact_value(contact, &field.value),
        FieldKind::Custom(kind) => contact
            .custom_fields
            .iter()
            .find(|(id, _)| *id == field.value)
            .map(|(_, value)| format_custom_field_value_in_time_zone(kind, Some(value), timezone))
            .unwrap_or_default(),
        FieldKind::Tag => if contact.tag_ids.contains(&field.value) {
            "Yes"
        } else {
            "No"
        }
        .into(),
    }
}

/// `buildCsvChunk`.
pub fn build_csv_chunk(
    contacts: &[ExportContact],
    fields: &[SelectedField],
    timezone: &str,
) -> String {
    contacts
        .iter()
        .map(|contact| {
            let cells: Vec<CsvCell> = fields
                .iter()
                .map(|field| CsvCell::Text(cell_value(contact, field, timezone)))
                .collect();
            to_csv_row(&cells, CsvDialect::Always)
        })
        .collect()
}

/// `buildHeaderLine`.
pub fn build_header_line(fields: &[SelectedField]) -> String {
    let cells: Vec<CsvCell> = fields
        .iter()
        .map(|f| CsvCell::Text(f.header.clone()))
        .collect();
    to_csv_row(&cells, CsvDialect::Always)
}

/// `buildSelectedFields`: the prefixed keys as typed columns with their
/// headers (a tag / custom field not found keeps its id as the header; an
/// unprefixed key is dropped). Ids that are not numbers match no row.
pub fn build_selected_fields(
    conn: &Connection,
    workspace_id: i64,
    fields: &[String],
) -> Result<Vec<SelectedField>> {
    let raw: Vec<(u8, &str)> = fields
        .iter()
        .filter_map(|f| {
            [(b't', "tag:"), (b'c', "cus:"), (b's', "sys:")]
                .into_iter()
                .find_map(|(kind, prefix)| f.strip_prefix(prefix).map(|rest| (kind, rest)))
        })
        .collect();
    let ids_of = |kind: u8| -> Vec<i64> {
        raw.iter()
            .filter(|(k, _)| *k == kind)
            .filter_map(|(_, v)| v.parse().ok())
            .collect()
    };
    let lookup = |sql: &str, ids: Vec<i64>| -> Result<HashMap<String, (String, String)>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }
        let list = serde_json::to_string(&ids).unwrap_or_else(|_| "[]".into());
        let mut stmt = conn.prepare(sql).map_err(repo_err)?;
        let rows = stmt
            .query_map(params![workspace_id, list], |r| {
                Ok((r.get::<_, i64>(0)?.to_string(), (r.get(1)?, r.get(2)?)))
            })
            .map_err(repo_err)?
            .collect::<std::result::Result<HashMap<_, _>, _>>()
            .map_err(repo_err)?;
        Ok(rows)
    };
    let tags = lookup(
        r#"SELECT "id", "name", '' FROM "Tag" WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL AND "id" IN (SELECT value FROM json_each(?2))"#,
        ids_of(b't'),
    )?;
    let customs = lookup(
        r#"SELECT "id", "name", "type" FROM "CustomField" WHERE "workspaceId" = ?1 AND "id" IN (SELECT value FROM json_each(?2))"#,
        ids_of(b'c'),
    )?;
    Ok(raw
        .into_iter()
        .map(|(kind, value)| {
            let value = value.to_owned();
            match kind {
                b's' => SelectedField {
                    kind: FieldKind::Contact,
                    header: header_name(&value).map_or_else(|| value.clone(), str::to_owned),
                    value,
                },
                b'c' => {
                    let (header, kind) = customs
                        .get(&value)
                        .cloned()
                        .unwrap_or_else(|| (value.clone(), "shortText".into()));
                    SelectedField {
                        kind: FieldKind::Custom(kind),
                        header,
                        value,
                    }
                }
                _ => SelectedField {
                    kind: FieldKind::Tag,
                    header: tags
                        .get(&value)
                        .map_or_else(|| value.clone(), |(n, _)| n.clone()),
                    value,
                },
            }
        })
        .collect())
}

/// Which `contactInboxes` rows a page loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboxRelation {
    None,
    Earliest,
    All,
}

/// `ContactExportRelations`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRelations {
    pub custom_field_ids: Vec<String>,
    pub tag_ids: Vec<String>,
    pub contact_inboxes: InboxRelation,
}

/// `resolveExportRelations`.
pub fn resolve_export_relations(fields: &[SelectedField]) -> ExportRelations {
    let values = |pred: fn(&FieldKind) -> bool| -> Vec<String> {
        fields
            .iter()
            .filter(|f| pred(&f.kind))
            .map(|f| f.value.clone())
            .collect()
    };
    let contact = values(|k| *k == FieldKind::Contact);
    let contact_inboxes = if contact.iter().any(|c| c == "sourceUserId") {
        InboxRelation::All
    } else if contact.iter().any(|c| c == "contactId") {
        InboxRelation::Earliest
    } else {
        InboxRelation::None
    };
    ExportRelations {
        custom_field_ids: values(|k| matches!(k, FieldKind::Custom(_))),
        tag_ids: values(|k| *k == FieldKind::Tag),
        contact_inboxes,
    }
}

/// `filter` of the job (`exportAll`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExportFilter {
    pub keyword: Option<String>,
    pub contact_filter: Option<FilterCriteria>,
}

/// `JobExportContacts["data"]` (the in-request job).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ExportJob {
    pub workspace_id: i64,
    pub requested_user_id: Option<i64>,
    pub file_id: i64,
    pub fields: Vec<String>,
    pub can_export_email_and_phone: bool,
    pub output_path: String,
    pub output_format: String,
    pub restrict_to_assigned_user_id: Option<i64>,
    /// `Some` for `exportAll`; otherwise `contact_ids` scope the export.
    pub filter: Option<ExportFilter>,
    pub contact_ids: Vec<i64>,
}

/// `buildBaseWhere`: the `contactIds` list (the filter is ignored), else the
/// keyword (first / last name, plus email / phone when allowed) and the
/// pruned contact filter; then the assigned-user conversation scope.
pub fn build_base_where(job: &ExportJob, now: i64) -> Sql {
    let mut parts = vec![Some(Sql::bind(
        "\"Contact\".\"workspaceId\" = ",
        job.workspace_id,
    ))];
    match &job.filter {
        None => {
            let list = serde_json::to_string(&job.contact_ids).unwrap_or_else(|_| "[]".into());
            parts.push(Some(
                Sql::bind("\"Contact\".\"id\" IN (SELECT value FROM json_each(", list).closed("))"),
            ));
        }
        Some(filter) => {
            if let Some(keyword) = filter.keyword.as_deref().filter(|k| !k.is_empty()) {
                let pattern = format!("%{}%", keyword.to_lowercase());
                let mut columns = vec!["firstName", "lastName"];
                if job.can_export_email_and_phone {
                    columns.extend(["email", "phoneNumber"]);
                }
                parts.push(Some(Sql::or(columns.into_iter().map(|c| {
                    Some(
                        Sql::bind(format!("ilike(\"Contact\".\"{c}\", "), pattern.clone())
                            .closed(")"),
                    )
                }))));
            }
            let pruned = prune_email_phone_filter_conditions(
                filter.contact_filter.as_ref(),
                job.can_export_email_and_phone,
            );
            parts.push(
                pruned
                    .as_ref()
                    .and_then(|f| apply_contact_filter(f, Some(job.workspace_id), now)),
            );
        }
    }
    if let Some(user) = job.restrict_to_assigned_user_id {
        parts.push(Some(Sql::bind(
            "EXISTS (SELECT 1 FROM \"Conversation\" WHERE \"Conversation\".\"contactId\" = \"Contact\".\"id\" AND \"Conversation\".\"assignedUserId\" = ",
            user,
        )
        .closed(")")));
    }
    Sql::and(parts)
}

const DATE_COLUMNS: &[&str] = &[
    "createdAt",
    "updatedAt",
    "lastReadAt",
    "subscribedAt",
    "broadcastSubscribedAt",
    "blockedAt",
];
const BOOL_COLUMNS: &[&str] = &["emailVerified", "emailOptIn"];

/// `String(value)` of one `Contact` column as Drizzle returns it.
fn render_column(name: &str, value: ValueRef<'_>) -> Option<String> {
    Some(match value {
        ValueRef::Null => return None,
        ValueRef::Integer(n) if DATE_COLUMNS.contains(&name) => iso(n),
        ValueRef::Integer(n) if BOOL_COLUMNS.contains(&name) => (n != 0).to_string(),
        ValueRef::Integer(n) => n.to_string(),
        ValueRef::Real(f) => chatbotx_core::js::number_to_string(f),
        // ponytail: `String(object)` of the JSON `location` column.
        ValueRef::Text(t) if name == "location" => match serde_json::from_slice::<Value>(t) {
            Ok(Value::Object(_)) => "[object Object]".into(),
            _ => String::from_utf8_lossy(t).into_owned(),
        },
        ValueRef::Text(t) | ValueRef::Blob(t) => String::from_utf8_lossy(t).into_owned(),
    })
}

fn id_list(ids: &[i64]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".into())
}

/// `(contactId, a, b)` of one relation row.
type RelatedRow = (i64, Option<String>, Option<String>);

/// `contactRepository.listForExportPage`: one keyset page (`id > last_id`,
/// id order) with only the relations the columns render.
pub fn list_export_page(
    conn: &Connection,
    base_where: &Sql,
    last_id: Option<i64>,
    limit: usize,
    relations: &ExportRelations,
) -> Result<Vec<ExportContact>> {
    let mut q = Sql::raw("SELECT \"Contact\".* FROM \"Contact\" WHERE ");
    q.append(Sql::and([
        Some(base_where.clone()),
        last_id.map(|id| Sql::bind("\"Contact\".\"id\" > ", id)),
    ]));
    q.push(" ORDER BY \"Contact\".\"id\" LIMIT ");
    q.push_bind(i64::try_from(limit).unwrap_or(i64::MAX));
    let mut contacts: Vec<ExportContact> = q
        .query_map(conn, |row| {
            let stmt = row.as_ref();
            let mut contact = ExportContact::default();
            let names: Vec<String> = stmt.column_names().map(str::to_owned).collect();
            for (index, name) in names.into_iter().enumerate() {
                let value = row.get_ref(index)?;
                if name == "id" {
                    contact.id = value.as_i64().unwrap_or_default();
                }
                if let Some(text) = render_column(&name, value) {
                    contact.columns.insert(name, text);
                }
            }
            Ok(contact)
        })
        .map_err(BusinessError::Repo)?;
    if contacts.is_empty() {
        return Ok(contacts);
    }
    let ids = id_list(&contacts.iter().map(|c| c.id).collect::<Vec<_>>());
    let index: HashMap<i64, usize> = contacts
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id, i))
        .collect();
    let related = |sql: &str, extra: Option<String>| -> Result<Vec<RelatedRow>> {
        let mut stmt = conn.prepare(sql).map_err(repo_err)?;
        let args: Vec<String> = std::iter::once(ids.clone()).chain(extra).collect();
        let rows = stmt
            .query_map(params_from_iter(args.iter()), |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok(rows)
    };
    let selected = |values: &[String]| {
        let ids: Vec<i64> = values.iter().filter_map(|v| v.parse().ok()).collect();
        (!ids.is_empty()).then(|| id_list(&ids))
    };
    if let Some(field_ids) = selected(&relations.custom_field_ids) {
        for (contact, id, value) in related(
            r#"SELECT "contactId", CAST("customFieldId" AS TEXT), "value" FROM "ContactCustomField"
               WHERE "contactId" IN (SELECT value FROM json_each(?1)) AND "customFieldId" IN (SELECT value FROM json_each(?2))"#,
            Some(field_ids),
        )? {
            contacts[index[&contact]]
                .custom_fields
                .push((id.unwrap_or_default(), value.unwrap_or_default()));
        }
    }
    if let Some(tag_ids) = selected(&relations.tag_ids) {
        for (contact, id, _) in related(
            r#"SELECT "contactId", CAST("tagId" AS TEXT), NULL FROM "ContactToTag"
               WHERE "contactId" IN (SELECT value FROM json_each(?1)) AND "tagId" IN (SELECT value FROM json_each(?2))"#,
            Some(tag_ids),
        )? {
            contacts[index[&contact]]
                .tag_ids
                .push(id.unwrap_or_default());
        }
    }
    if relations.contact_inboxes != InboxRelation::None {
        for (contact, source, user) in related(
            r#"SELECT "contactId", "sourceId", "sourceUserId" FROM "ContactInbox"
               WHERE "contactId" IN (SELECT value FROM json_each(?1)) ORDER BY "id""#,
            None,
        )? {
            let inboxes = &mut contacts[index[&contact]].inboxes;
            if relations.contact_inboxes == InboxRelation::All || inboxes.is_empty() {
                inboxes.push((source, user));
            }
        }
    }
    Ok(contacts)
}

/// `contactExportService.start` input.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StartExport {
    pub workspace_id: i64,
    pub requested_user_id: Option<i64>,
    pub can_export_email_and_phone: bool,
    pub restrict_to_assigned_user_id: Option<i64>,
    pub fields: Vec<String>,
    pub export_all: bool,
    pub filter: Option<ExportFilter>,
    pub contact_ids: Vec<i64>,
}

/// `contacts-YYYY-MM-DD.csv` of `now`.
fn export_file_name(now: i64) -> String {
    format!("contacts-{}.csv", &iso(now)[..10])
}

/// `contactExportService.start`: writes the pending export File row and
/// returns the job to run (the payload `defaultQueue.add` would carry).
pub async fn start(db: &Db, input: StartExport) -> Result<ExportJob> {
    let fields = strip_contact_pii_fields(&input.fields, input.can_export_email_and_phone);
    let (filter, contact_ids) = if input.export_all {
        (Some(input.filter.unwrap_or_default()), Vec::new())
    } else {
        (None, input.contact_ids)
    };
    let file_id = new_id();
    let output_path = format!(
        "workspaces/{}/exports/contacts/contact_{file_id}.csv",
        input.workspace_id
    );
    let file_name = export_file_name(now_ms());
    let (ws, user, path) = (
        input.workspace_id,
        input.requested_user_id,
        output_path.clone(),
    );
    tx::write(db, move |conn| {
        conn.execute(
            r#"INSERT INTO "File" ("id", "workspaceId", "userId", "contextType", "subType", "path", "fileName", "mimeType", "status")
               VALUES (?1, ?2, ?3, 'export', 'contacts', ?4, ?5, 'text/csv', 'pending')"#,
            params![file_id, ws, user, path, file_name],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await?;
    Ok(ExportJob {
        workspace_id: input.workspace_id,
        requested_user_id: input.requested_user_id,
        file_id,
        fields,
        can_export_email_and_phone: input.can_export_email_and_phone,
        output_path,
        output_format: "csv".into(),
        restrict_to_assigned_user_id: input.restrict_to_assigned_user_id,
        filter,
        contact_ids,
    })
}

async fn update_file(
    db: &Db,
    job: &ExportJob,
    status: &str,
    total: usize,
    size: Option<usize>,
) -> Result<()> {
    let (id, ws, status) = (job.file_id, job.workspace_id, status.to_owned());
    let meta = serde_json::json!({ "totalRecords": total }).to_string();
    let size = size.and_then(|s| i64::try_from(s).ok());
    tx::write(db, move |conn| {
        conn.execute(
            r#"UPDATE "File" SET "status" = ?1, "meta" = ?2, "fileSize" = COALESCE(?3, "fileSize"), "updatedAt" = ?4
               WHERE "id" = ?5 AND "workspaceId" = ?6"#,
            params![status, meta, size, now_ms(), id, ws],
        )
        .map_err(repo_err)?;
        Ok(())
    })
    .await
}

/// Builds the CSV body (BOM + header + rows) and its row count.
async fn build_csv(db: &Db, job: &ExportJob) -> Result<(String, usize)> {
    let job = job.clone();
    tx::read(db, move |conn| {
        let fields = strip_contact_pii_fields(&job.fields, job.can_export_email_and_phone);
        let selected = build_selected_fields(conn, job.workspace_id, &fields)?;
        let timezone: Option<String> = conn
            .query_row(
                r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
                [job.workspace_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(repo_err)?;
        let timezone =
            normalize_stored_timezone(timezone.as_deref()).unwrap_or_else(|| "UTC".into());
        let base_where = build_base_where(&job, now_ms());
        let relations = resolve_export_relations(&selected);
        let with_ad_ids = selected
            .iter()
            .any(|f| f.kind == FieldKind::Contact && f.value == "adIds");
        let mut body = format!("{UTF8_BOM}{}", build_header_line(&selected));
        let mut total = 0;
        let mut last_id = None;
        loop {
            let mut page =
                list_export_page(conn, &base_where, last_id, EXPORT_PAGE_SIZE, &relations)?;
            if page.is_empty() {
                break;
            }
            if with_ad_ids {
                let ids: Vec<i64> = page.iter().map(|c| c.id).collect();
                let mut by_contact: BTreeMap<i64, Vec<String>> = list_ad_ids_by_contacts(
                    conn,
                    job.workspace_id,
                    &ids,
                    CONTACT_AD_IDS_LIMIT as i64,
                )
                .map_err(BusinessError::Repo)?;
                for contact in &mut page {
                    contact.ad_ids = by_contact.remove(&contact.id).unwrap_or_default();
                }
            }
            body.push_str(&build_csv_chunk(&page, &selected, &timezone));
            total += page.len();
            last_id = page.last().map(|c| c.id);
            if total >= MAX_EXPORT_ROWS {
                return Err(BusinessError::Repo(chatbotx_db::repo::Error::msg(
                    "Export exceeded the 10,000,000-row limit. Use filters to narrow the export.",
                )));
            }
            if page.len() < EXPORT_PAGE_SIZE {
                break;
            }
        }
        Ok((body, total))
    })
    .await
}

/// `loopableExportContacts`: writes the CSV to `output_path` and marks the
/// File `uploaded` (`failed` when no contact matched, or on any error, which
/// is returned).
pub async fn run_export(db: &Db, uploader: &Uploader, job: &ExportJob) -> Result<()> {
    if job.output_format != "csv" {
        return Ok(());
    }
    let built = build_csv(db, job).await;
    let (body, total) = match built {
        Ok(built) => built,
        Err(error) => {
            update_file(db, job, "failed", 0, None).await?;
            return Err(error);
        }
    };
    let size = body.len();
    let options = PutOptions {
        content_type: Some("text/csv; charset=utf-8".into()),
        ..PutOptions::default()
    };
    if let Err(error) = uploader
        .put_object(&job.output_path, body.into_bytes(), options, None)
        .await
    {
        update_file(db, job, "failed", total, None).await?;
        return Err(BusinessError::Repo(chatbotx_db::repo::Error::msg(
            error.to_string(),
        )));
    }
    let status = if total == 0 { "failed" } else { "uploaded" };
    update_file(db, job, status, total, Some(size)).await
}

/// `contactExportService.getFile` result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFileStatus {
    pub status: String,
    pub file_name: String,
    pub download_url: Option<String>,
    pub total_records: Option<i64>,
}

fn total_records(meta: Option<&str>) -> Option<i64> {
    serde_json::from_str::<Value>(meta?)
        .ok()?
        .get("totalRecords")?
        .as_i64()
}

/// `contactExportService.getFile`: the File of the workspace (any context,
/// as `findByIdForWorkspace`), with a 300 s download URL once uploaded.
pub async fn get_file(
    db: &Db,
    uploader: Option<&Uploader>,
    workspace_id: i64,
    file_id: i64,
    user_id: Option<i64>,
) -> Result<ExportFileStatus> {
    let row: Option<(String, String, String, Option<String>)> = tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "status", "fileName", "path", "meta" FROM "File"
               WHERE "id" = ?1 AND "workspaceId" = ?2 AND (?3 IS NULL OR "userId" = ?3)"#,
            params![file_id, workspace_id, user_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(repo_err)
    })
    .await?;
    let (status, file_name, path, meta) =
        row.ok_or_else(|| BusinessError::not_found("Export file not found"))?;
    let download_url = if status == "uploaded" {
        let uploader = uploader.ok_or_else(|| {
            BusinessError::Repo(chatbotx_db::repo::Error::msg(
                "file storage is not configured",
            ))
        })?;
        Some(
            uploader
                .get_presigned_download(&path, Some(300))
                .await
                .map_err(|e| BusinessError::Repo(chatbotx_db::repo::Error::msg(e.to_string())))?,
        )
    } else {
        None
    };
    Ok(ExportFileStatus {
        status,
        file_name,
        download_url,
        total_records: total_records(meta.as_deref()),
    })
}

/// `ExportFileCursor`: `createdAt` as UTC ISO with microseconds, and the id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFileCursor {
    pub created_at: String,
    pub id: i64,
}

impl ExportFileCursor {
    /// `contactExportFileCursor` (strict): `YYYY-MM-DDTHH:MM:SS.ffffffZ`
    /// of a real date and not year 0; `id` 1-19 digits within the bigint
    /// range.
    pub fn parse(created_at: &str, id: &str) -> Option<ExportFileCursor> {
        let b = created_at.as_bytes();
        let digits = |range: std::ops::Range<usize>| b[range].iter().all(u8::is_ascii_digit);
        let shape = b.len() == 27
            && digits(0..4)
            && b[4] == b'-'
            && digits(5..7)
            && b[7] == b'-'
            && digits(8..10)
            && b[10] == b'T'
            && digits(11..13)
            && b[13] == b':'
            && digits(14..16)
            && b[16] == b':'
            && digits(17..19)
            && b[19] == b'.'
            && digits(20..26)
            && b[26] == b'Z';
        if !shape || created_at.starts_with("0000") {
            return None;
        }
        chrono::NaiveDateTime::parse_from_str(&created_at[..19], "%Y-%m-%dT%H:%M:%S").ok()?;
        if id.is_empty() || id.len() > 19 || !id.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        Some(ExportFileCursor {
            created_at: created_at.to_owned(),
            id: id.parse().ok()?,
        })
    }

    fn created_at_ms(&self) -> Option<i64> {
        let micros: i64 = self.created_at[20..26].parse().ok()?;
        let at = chrono::NaiveDateTime::parse_from_str(&self.created_at[..19], "%Y-%m-%dT%H:%M:%S")
            .ok()?;
        Some(at.and_utc().timestamp_millis() + micros / 1000)
    }
}

/// `createdAtCursor()`: the column as UTC ISO with microseconds.
fn created_at_cursor(ms: i64) -> String {
    format!("{}000Z", iso(ms).trim_end_matches('Z'))
}

/// `ContactExportFileSummary`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFileSummary {
    pub id: i64,
    pub status: String,
    pub file_name: String,
    pub total_records: Option<i64>,
    pub created_at: i64,
    pub expires_at: i64,
    /// `(id, name)` of the member who started it.
    pub requested_by: Option<(i64, Option<String>)>,
}

/// `contactExportService.listFiles`: the workspace's contact exports, newest
/// first, keyset-paged; `user_id` narrows to its creator.
pub async fn list_files(
    db: &Db,
    workspace_id: i64,
    user_id: Option<i64>,
    limit: usize,
    cursor: Option<ExportFileCursor>,
) -> Result<(Vec<ExportFileSummary>, Option<ExportFileCursor>)> {
    let past = cursor
        .as_ref()
        .map(|c| (c.created_at_ms().unwrap_or(i64::MIN), c.id));
    let fetch = i64::try_from(limit).unwrap_or(i64::MAX - 1) + 1;
    let rows: Vec<ExportFileSummary> = tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare(
                r#"SELECT f."id", f."status", f."fileName", f."meta", f."createdAt", f."userId", u."name"
                   FROM "File" f LEFT JOIN "User" u ON u."id" = f."userId"
                   WHERE f."workspaceId" = ?1 AND f."contextType" = 'export' AND f."subType" = 'contacts'
                     AND (?2 IS NULL OR f."userId" = ?2)
                     AND (?3 IS NULL OR (f."createdAt", f."id") < (?3, ?4))
                   ORDER BY f."createdAt" DESC, f."id" DESC LIMIT ?5"#,
            )
            .map_err(repo_err)?;
        let rows = stmt
            .query_map(
                params![workspace_id, user_id, past.map(|p| p.0), past.map(|p| p.1), fetch],
                |r| {
                    let created_at: i64 = r.get(4)?;
                    let user: Option<i64> = r.get(5)?;
                    let meta: Option<String> = r.get(3)?;
                    Ok(ExportFileSummary {
                        id: r.get(0)?,
                        status: r.get(1)?,
                        file_name: r.get(2)?,
                        total_records: total_records(meta.as_deref()),
                        created_at,
                        expires_at: created_at + RETENTION_MS,
                        requested_by: match user {
                            Some(id) => Some((id, r.get(6)?)),
                            None => None,
                        },
                    })
                },
            )
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok(rows)
    })
    .await?;
    let more = rows.len() > limit;
    let page: Vec<ExportFileSummary> = rows.into_iter().take(limit).collect();
    let next = page.last().filter(|_| more).map(|last| ExportFileCursor {
        created_at: created_at_cursor(last.created_at),
        id: last.id,
    });
    Ok((page, next))
}
