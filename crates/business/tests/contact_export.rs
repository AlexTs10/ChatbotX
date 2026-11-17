//! Ports (P3.6-D2 slice contact export) of
//! `packages/business/__tests__/contact-export-service.test.ts`,
//! `apps/worker/__tests__/export-contacts-{csv,datetime,fields,where,handler}.test.ts`
//! and `packages/database/__tests__/{contact-export-page,file-export}-repository.test.ts`.
//! Adapted: the mocked repositories / uploader become a real database and an
//! in-memory store; `buildBaseWhere` assertions on the Drizzle where object
//! become the contacts the where matches; the queue payload is the returned
//! [`ExportJob`].

mod common;

use chatbotx_business::contact_export::{
    EXPORT_PAGE_SIZE, ExportContact, ExportFileCursor, ExportFilter, ExportJob, ExportRelations,
    FieldKind, InboxRelation, SelectedField, StartExport, build_base_where, build_csv_chunk,
    build_header_line, build_selected_fields, get_file, list_export_page, list_files,
    resolve_export_relations, run_export, start, strip_contact_pii_fields,
};
use chatbotx_business::contact_filter::to_filter_criteria;
use chatbotx_db::Db;
use chatbotx_db::rusqlite::params;
use chatbotx_files::{LocalUrlSigner, Uploader};
use common::{
    OTHER_WS, WS, contact_inbox, contact_with, conversation, custom_field, inbox, setup, tag,
    tag_link, two_workspaces,
};
use serde_json::json;

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| (*x).to_owned()).collect()
}

fn field(kind: FieldKind, value: &str, header: &str) -> SelectedField {
    SelectedField {
        kind,
        value: value.into(),
        header: header.into(),
    }
}

fn contact_cols(cols: &[(&str, &str)]) -> ExportContact {
    ExportContact {
        id: 1,
        columns: cols
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        ..ExportContact::default()
    }
}

// ── contactExportService.start ───────────────────────────────────────────

fn start_input(fields: &[&str], can: bool) -> StartExport {
    StartExport {
        workspace_id: WS,
        requested_user_id: Some(1001),
        can_export_email_and_phone: can,
        fields: s(fields),
        export_all: true,
        ..StartExport::default()
    }
}

#[tokio::test]
async fn start_strips_email_phone_fields_when_can_export_email_and_phone_is_false() {
    let db = two_workspaces().await;
    let job = start(
        &db,
        start_input(&["sys:firstName", "sys:email", "sys:phoneNumber"], false),
    )
    .await
    .expect("start");
    assert_eq!(job.fields, s(&["sys:firstName"]));
}

#[tokio::test]
async fn start_keeps_pii_fields_when_can_export_email_and_phone_is_true() {
    let db = two_workspaces().await;
    let job = start(&db, start_input(&["sys:firstName", "sys:email"], true))
        .await
        .expect("start");
    assert_eq!(job.fields, s(&["sys:firstName", "sys:email"]));
}

#[tokio::test]
async fn start_sends_a_filter_payload_no_contact_ids_when_export_all_is_true() {
    let db = two_workspaces().await;
    let mut input = start_input(&["sys:firstName"], true);
    input.filter = Some(ExportFilter {
        keyword: Some("vip".into()),
        contact_filter: None,
    });
    let job = start(&db, input).await.expect("start");
    assert_eq!(
        job.filter,
        Some(ExportFilter {
            keyword: Some("vip".into()),
            contact_filter: None
        })
    );
    assert!(job.contact_ids.is_empty());
}

#[tokio::test]
async fn start_sends_a_contact_ids_payload_no_filter_when_export_all_is_false() {
    let db = two_workspaces().await;
    let mut input = start_input(&["sys:firstName"], true);
    input.export_all = false;
    input.contact_ids = vec![1, 2];
    let job = start(&db, input).await.expect("start");
    assert_eq!(job.contact_ids, vec![1, 2]);
    assert_eq!(job.filter, None);
}

/// Own case: the pending File row `start` records (`fileRepository.create`).
#[tokio::test]
async fn start_records_a_pending_contacts_export_file() {
    let db = two_workspaces().await;
    let job = start(&db, start_input(&["sys:firstName"], true))
        .await
        .expect("start");
    let row = db
        .read(move |c| {
            Ok(c.query_row(
                r#"SELECT "workspaceId", "userId", "contextType", "subType", "path", "fileName", "mimeType", "status" FROM "File" WHERE "id" = ?1"#,
                [job.file_id],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                    ))
                },
            )?)
        })
        .await
        .expect("row");
    assert_eq!((row.0, row.1), (WS, Some(1001)));
    assert_eq!((row.2.as_str(), row.3.as_str()), ("export", "contacts"));
    assert_eq!(
        row.4,
        format!(
            "workspaces/{WS}/exports/contacts/contact_{}.csv",
            job.file_id
        )
    );
    assert!(row.5.starts_with("contacts-") && row.5.ends_with(".csv") && row.5.len() == 23);
    assert_eq!((row.6.as_str(), row.7.as_str()), ("text/csv", "pending"));
}

// ── contactExportService.getFile ─────────────────────────────────────────

async fn file_row(db: &Db, id: i64, ws: i64, status: &str, meta: Option<&str>) {
    let (status, meta) = (status.to_owned(), meta.map(str::to_owned));
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType", "status", "meta")
               VALUES (?1, ?2, 'export', 'contacts', ?3, 'contacts.csv', 'text/csv', ?4, ?5)"#,
            params![id, ws, format!("workspaces/{ws}/exports/contacts/contact_{id}.csv"), status, meta],
        )?;
        Ok(())
    })
    .await;
}

fn signed_uploader() -> Uploader {
    Uploader::in_memory()
        .with_url_signer(LocalUrlSigner::new("secret", "http://app/files").expect("signer"))
}

#[tokio::test]
async fn get_file_404s_when_the_file_doesn_t_resolve_in_this_workspace() {
    let db = two_workspaces().await;
    file_row(&db, 7, OTHER_WS, "uploaded", None).await;
    let error = get_file(&db, Some(&signed_uploader()), WS, 7, None)
        .await
        .expect_err("404");
    assert_eq!(error.message(), "Export file not found");
}

#[tokio::test]
async fn get_file_returns_a_presigned_url_only_when_the_file_has_uploaded() {
    let db = two_workspaces().await;
    file_row(&db, 7, WS, "uploaded", Some(r#"{"totalRecords":42}"#)).await;
    let file = get_file(&db, Some(&signed_uploader()), WS, 7, None)
        .await
        .expect("file");
    assert_eq!(file.status, "uploaded");
    assert_eq!(file.file_name, "contacts.csv");
    assert_eq!(file.total_records, Some(42));
    let url = file.download_url.expect("url");
    assert!(
        url.contains(&format!("workspaces/{WS}/exports/contacts/contact_7.csv")),
        "{url}"
    );
}

#[tokio::test]
async fn get_file_returns_no_download_url_while_the_file_is_still_pending() {
    let db = two_workspaces().await;
    file_row(&db, 7, WS, "pending", None).await;
    // No uploader: a pending file never asks storage for a link.
    let file = get_file(&db, None, WS, 7, None).await.expect("file");
    assert_eq!(
        (file.status.as_str(), file.download_url, file.total_records),
        ("pending", None, None)
    );
}

// ── contactExportService.listFiles / fileRepository.listExportsForWorkspace ─

async fn export_at(db: &Db, id: i64, ws: i64, user: Option<i64>, created_at: i64, sub: &str) {
    let sub = sub.to_owned();
    setup(db, move |tx| {
        tx.execute(
            r#"INSERT INTO "File" ("id", "createdAt", "workspaceId", "userId", "contextType", "subType", "path", "fileName", "mimeType", "status", "meta")
               VALUES (?1, ?2, ?3, ?4, 'export', ?5, ?6, ?7, 'text/csv', 'uploaded', '{"totalRecords":12}')"#,
            params![id, created_at, ws, user, sub, format!("p/{id}"), format!("contacts-{id}.csv")],
        )?;
        Ok(())
    })
    .await;
}

/// 2026-10-20T10:00:00.123Z.
const AT: i64 = 1_792_490_400_123;

#[tokio::test]
async fn list_files_lists_the_workspace_s_contact_exports_with_their_expiry_and_requester() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(r#"UPDATE "User" SET "name" = 'Lan' WHERE "id" = 1001"#, [])?;
        Ok(())
    })
    .await;
    export_at(&db, 1, WS, Some(1001), AT, "contacts").await;
    export_at(&db, 2, OTHER_WS, None, AT, "contacts").await;
    export_at(&db, 3, WS, None, AT, "coupons").await;
    let (data, next) = list_files(&db, WS, Some(1001), 20, None)
        .await
        .expect("list");
    assert_eq!(data.len(), 1);
    let row = &data[0];
    assert_eq!((row.id, row.status.as_str()), (1, "uploaded"));
    assert_eq!(row.total_records, Some(12));
    assert_eq!(row.created_at, AT);
    assert_eq!(row.expires_at, AT + 7 * 24 * 60 * 60 * 1000);
    assert_eq!(row.requested_by, Some((1001, Some("Lan".to_owned()))));
    assert_eq!(next, None);
}

#[tokio::test]
async fn list_files_returns_a_cursor_only_when_a_row_past_the_page_exists() {
    let db = two_workspaces().await;
    export_at(&db, 1, WS, None, AT, "contacts").await;
    export_at(&db, 2, WS, None, AT, "contacts").await;
    let (data, next) = list_files(&db, WS, None, 1, None).await.expect("list");
    assert_eq!(data.iter().map(|r| r.id).collect::<Vec<_>>(), vec![2]);
    // The cursor keeps the column's microseconds.
    let next = next.expect("cursor");
    assert_eq!(
        next,
        ExportFileCursor {
            created_at: "2026-10-20T10:00:00.123000Z".into(),
            id: 2
        }
    );
    let (data, next) = list_files(&db, WS, None, 1, Some(next))
        .await
        .expect("page 2");
    assert_eq!(data.iter().map(|r| r.id).collect::<Vec<_>>(), vec![1]);
    assert_eq!(next, None);
}

#[tokio::test]
async fn list_files_an_export_started_with_an_api_token_has_no_requester() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType", "status")
               VALUES (5, ?1, 'export', 'contacts', 'p/5', 'c.csv', 'text/csv', 'pending')"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    let (data, _) = list_files(&db, WS, None, 20, None).await.expect("list");
    assert_eq!(
        (
            &data[0].requested_by,
            data[0].total_records,
            data[0].status.as_str()
        ),
        (&None, None, "pending")
    );
}

#[tokio::test]
async fn file_export_repository_lists_newest_first_and_pages_by_created_at_then_id() {
    let db = two_workspaces().await;
    export_at(&db, 1, WS, None, AT - 1, "contacts").await;
    export_at(&db, 2, WS, None, AT, "contacts").await;
    export_at(&db, 3, WS, None, AT, "contacts").await;
    let (data, _) = list_files(&db, WS, None, 20, None).await.expect("list");
    assert_eq!(data.iter().map(|r| r.id).collect::<Vec<_>>(), vec![3, 2, 1]);
    let cursor = ExportFileCursor::parse("2026-10-20T10:00:00.123000Z", "3").expect("cursor");
    let (data, _) = list_files(&db, WS, None, 20, Some(cursor))
        .await
        .expect("list");
    assert_eq!(data.iter().map(|r| r.id).collect::<Vec<_>>(), vec![2, 1]);
}

#[tokio::test]
async fn contact_export_file_cursor_rejects_a_bad_created_at_or_id() {
    let ok = "2026-10-20T10:00:00.123456Z";
    assert!(ExportFileCursor::parse(ok, "42").is_some());
    for (created_at, id) in [
        ("'; drop", "42"),
        ("2026-02-30T10:00:00.000000Z", "42"),
        ("0000-01-01T00:00:00.000000Z", "42"),
        ("2026-10-20T10:00:00.123Z", "42"),
        (ok, "9223372036854775808"),
        (ok, "abc"),
        (ok, ""),
    ] {
        assert_eq!(
            ExportFileCursor::parse(created_at, id),
            None,
            "{created_at} {id}"
        );
    }
}

// ── buildHeaderLine / buildCsvChunk ──────────────────────────────────────

#[test]
fn build_header_line_joins_escaped_headers_with_commas_and_appends_a_trailing_newline() {
    let fields = [
        field(FieldKind::Contact, "fullName", "Full Name"),
        field(FieldKind::Contact, "email", "Email"),
    ];
    assert_eq!(build_header_line(&fields), "\"Full Name\",\"Email\"\n");
}

#[test]
fn build_header_line_escapes_double_quotes_that_appear_in_a_header_label() {
    let fields = [field(
        FieldKind::Custom("shortText".into()),
        "c1",
        "Size \"S\"",
    )];
    assert_eq!(build_header_line(&fields), "\"Size \"\"S\"\"\"\n");
}

#[test]
fn build_header_line_produces_a_single_field_header_line() {
    assert_eq!(
        build_header_line(&[field(FieldKind::Tag, "t1", "VIP")]),
        "\"VIP\"\n"
    );
}

fn chunk(contacts: &[ExportContact], fields: &[SelectedField]) -> String {
    build_csv_chunk(contacts, fields, "UTC")
}

#[test]
fn build_csv_chunk_returns_an_empty_string_when_the_contacts_array_is_empty() {
    assert_eq!(
        chunk(&[], &[field(FieldKind::Contact, "fullName", "Full Name")]),
        ""
    );
}

#[test]
fn build_csv_chunk_produces_one_row_with_a_trailing_newline_for_a_single_contact() {
    let fields = [
        field(FieldKind::Contact, "fullName", "Full Name"),
        field(FieldKind::Contact, "email", "Email"),
    ];
    let contacts = [contact_cols(&[
        ("fullName", "Jane Doe"),
        ("email", "jane@example.com"),
    ])];
    assert_eq!(
        chunk(&contacts, &fields),
        "\"Jane Doe\",\"jane@example.com\"\n"
    );
}

#[test]
fn build_csv_chunk_serializes_a_date_field_value_as_iso_string() {
    // Adapted: the loader renders a date column as ISO text; see
    // `run_export_serializes_date_as_iso_and_null_as_an_empty_string`.
    let contacts = [contact_cols(&[("blockedAt", "2026-01-02T03:04:05.000Z")])];
    assert_eq!(
        chunk(
            &contacts,
            &[field(FieldKind::Contact, "blockedAt", "Blocked At")]
        ),
        "\"2026-01-02T03:04:05.000Z\"\n"
    );
}

#[test]
fn build_csv_chunk_renders_null_and_undefined_contact_fields_as_an_empty_quoted_string() {
    let contacts = [contact_cols(&[])];
    assert_eq!(
        chunk(&contacts, &[field(FieldKind::Contact, "email", "Email")]),
        "\"\"\n"
    );
    assert_eq!(
        chunk(
            &contacts,
            &[field(FieldKind::Contact, "phoneNumber", "Phone Number")]
        ),
        "\"\"\n"
    );
}

#[test]
fn build_csv_chunk_picks_a_custom_field_value_by_custom_field_id_with_an_empty_fallback() {
    let fields = [field(FieldKind::Custom("shortText".into()), "11", "Plan")];
    let with = ExportContact {
        custom_fields: vec![("11".into(), "Pro".into())],
        ..ExportContact::default()
    };
    assert_eq!(chunk(&[with], &fields), "\"Pro\"\n");
    assert_eq!(chunk(&[ExportContact::default()], &fields), "\"\"\n");
}

#[test]
fn build_csv_chunk_renders_yes_or_no_for_tag_membership() {
    let fields = [field(FieldKind::Tag, "9", "VIP")];
    let tagged = ExportContact {
        tag_ids: vec!["9".into()],
        ..ExportContact::default()
    };
    assert_eq!(chunk(&[tagged], &fields), "\"Yes\"\n");
    assert_eq!(chunk(&[ExportContact::default()], &fields), "\"No\"\n");
}

#[test]
fn build_csv_chunk_renders_the_contact_id_from_the_first_contact_inbox_source_id() {
    let fields = [field(FieldKind::Contact, "contactId", "Contact ID")];
    let with = ExportContact {
        inboxes: vec![(Some("psid-123".into()), None)],
        ..ExportContact::default()
    };
    assert_eq!(chunk(&[with], &fields), "\"psid-123\"\n");
    assert_eq!(chunk(&[ExportContact::default()], &fields), "\"\"\n");
}

#[test]
fn build_csv_chunk_renders_the_whatsapp_user_id_from_the_first_inbox_row_that_carries_one() {
    let fields = [
        field(FieldKind::Contact, "contactId", "Contact ID"),
        field(FieldKind::Contact, "sourceUserId", "WhatsApp User ID"),
    ];
    let multi = ExportContact {
        inboxes: vec![
            (Some("psid-1".into()), None),
            (Some("84912345678".into()), Some("user.937292738".into())),
        ],
        ..ExportContact::default()
    };
    assert_eq!(chunk(&[multi], &fields), "\"psid-1\",\"user.937292738\"\n");
    let none = ExportContact {
        inboxes: vec![(Some("psid-1".into()), None)],
        ..ExportContact::default()
    };
    assert_eq!(
        chunk(&[none, ExportContact::default()], &fields[1..]),
        "\"\"\n\"\"\n"
    );
}

#[test]
fn build_csv_chunk_renders_the_ad_ids_column_as_a_comma_separated_list() {
    let fields = [field(FieldKind::Contact, "adIds", "Ad IDs")];
    let with = ExportContact {
        ad_ids: s(&["1001", "1002", "1004"]),
        ..ExportContact::default()
    };
    assert_eq!(
        chunk(&[with, ExportContact::default()], &fields),
        "\"1001, 1002, 1004\"\n\"\"\n"
    );
}

#[test]
fn build_csv_chunk_produces_multiple_rows_separated_by_newlines() {
    let contacts = [
        contact_cols(&[("fullName", "Alice")]),
        contact_cols(&[("fullName", "Bob")]),
    ];
    assert_eq!(
        chunk(
            &contacts,
            &[field(FieldKind::Contact, "fullName", "Full Name")]
        ),
        "\"Alice\"\n\"Bob\"\n"
    );
}

#[test]
fn export_contacts_formats_custom_date_and_datetime_fields_in_the_workspace_timezone() {
    let fields = [
        field(FieldKind::Custom("date".into()), "1", "Date"),
        field(FieldKind::Custom("datetime".into()), "2", "Datetime"),
    ];
    let contact = ExportContact {
        custom_fields: vec![
            ("1".into(), "2026-07-22T00:00:00+07:00".into()),
            ("2".into(), "2026-07-22T08:30:00.000Z".into()),
        ],
        ..ExportContact::default()
    };
    assert_eq!(
        build_csv_chunk(&[contact], &fields, "Asia/Ho_Chi_Minh"),
        "\"2026-07-22\",\"2026-07-22 15:30:00\"\n"
    );
}

// ── buildSelectedFields ──────────────────────────────────────────────────

async fn selected(db: &Db, fields: &[&str]) -> Vec<SelectedField> {
    let fields = s(fields);
    chatbotx_db::repo::read(db, move |c| {
        Ok(build_selected_fields(c, WS, &fields).expect("fields"))
    })
    .await
    .expect("read")
}

#[tokio::test]
async fn build_selected_fields_maps_contact_columns_to_their_display_headers() {
    let db = two_workspaces().await;
    for (value, header) in [
        ("contactId", "Contact ID"),
        ("firstName", "First Name"),
        ("lastName", "Last Name"),
        ("fullName", "Full Name"),
        ("email", "Email"),
        ("phoneNumber", "Phone Number"),
        ("gender", "Gender"),
        ("source", "Source"),
        ("lastReadAt", "Last Read At"),
        ("blockedAt", "Blocked At"),
        ("sourceUserId", "WhatsApp User ID"),
        ("adIds", "Ad IDs"),
        ("weirdCol", "weirdCol"),
    ] {
        assert_eq!(
            selected(&db, &[&format!("sys:{value}")]).await,
            vec![field(FieldKind::Contact, value, header)]
        );
    }
}

#[tokio::test]
async fn build_selected_fields_drops_unprefixed_strings() {
    let db = two_workspaces().await;
    assert!(
        selected(&db, &["noPrefix", "unknown:field", "justAWord"])
            .await
            .is_empty()
    );
    assert_eq!(
        selected(&db, &["garbage", "sys:email", "alsoGarbage"]).await,
        vec![field(FieldKind::Contact, "email", "Email")]
    );
    assert!(selected(&db, &[]).await.is_empty());
}

#[tokio::test]
async fn build_selected_fields_resolves_tags_and_custom_fields_of_the_workspace_only() {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tag(tx, 9, WS, "VIP")?;
        tag(tx, 8, OTHER_WS, "Other")?;
        custom_field(tx, 11, WS, "Plan", "number")?;
        custom_field(tx, 12, OTHER_WS, "Theirs", "date")?;
        Ok(())
    })
    .await;
    assert_eq!(
        selected(
            &db,
            &["sys:email", "tag:9", "cus:11", "tag:8", "cus:12", "tag:x"]
        )
        .await,
        vec![
            field(FieldKind::Contact, "email", "Email"),
            field(FieldKind::Tag, "9", "VIP"),
            field(FieldKind::Custom("number".into()), "11", "Plan"),
            field(FieldKind::Tag, "8", "8"),
            field(FieldKind::Custom("shortText".into()), "12", "12"),
            field(FieldKind::Tag, "x", "x"),
        ]
    );
}

// ── resolveExportRelations ───────────────────────────────────────────────

#[test]
fn resolve_export_relations_picks_only_the_relations_the_columns_render() {
    let c = |v: &str| field(FieldKind::Contact, v, v);
    assert_eq!(
        resolve_export_relations(&[c("fullName")]),
        ExportRelations {
            custom_field_ids: vec![],
            tag_ids: vec![],
            contact_inboxes: InboxRelation::None
        }
    );
    assert_eq!(
        resolve_export_relations(&[c("contactId")]).contact_inboxes,
        InboxRelation::Earliest
    );
    assert_eq!(
        resolve_export_relations(&[c("contactId"), c("sourceUserId")]).contact_inboxes,
        InboxRelation::All
    );
    let r = resolve_export_relations(&[
        field(FieldKind::Custom("shortText".into()), "cf-1", "x"),
        field(FieldKind::Tag, "t-1", "x"),
        field(FieldKind::Tag, "t-2", "x"),
    ]);
    assert_eq!(
        (r.custom_field_ids, r.tag_ids),
        (s(&["cf-1"]), s(&["t-1", "t-2"]))
    );
}

#[test]
fn strip_contact_pii_fields_removes_email_and_phone_only_when_denied() {
    let fields = s(&["sys:firstName", "sys:email", "sys:phoneNumber"]);
    assert_eq!(
        strip_contact_pii_fields(&fields, false),
        s(&["sys:firstName"])
    );
    assert_eq!(strip_contact_pii_fields(&fields, true), fields);
}

// ── buildBaseWhere (by the contacts it matches) ──────────────────────────

async fn seed_people(db: &Db) {
    setup(db, |tx| {
        contact_with(tx, 1, WS, Some("Acme Ann"), None, None)?;
        contact_with(tx, 2, WS, Some("Bob"), Some("bob@acme.test"), None)?;
        contact_with(tx, 3, WS, Some("Cat"), None, Some("+84acme"))?;
        contact_with(tx, 4, WS, Some("Dan"), None, None)?;
        contact_with(tx, 5, OTHER_WS, Some("Acme Other"), None, None)?;
        Ok(())
    })
    .await;
}

fn job(filter: Option<ExportFilter>, ids: &[i64], can: bool) -> ExportJob {
    ExportJob {
        workspace_id: WS,
        filter,
        contact_ids: ids.to_vec(),
        can_export_email_and_phone: can,
        output_format: "csv".into(),
        ..ExportJob::default()
    }
}

async fn matched(db: &Db, job: ExportJob) -> Vec<i64> {
    chatbotx_db::repo::read(db, move |c| {
        let relations = resolve_export_relations(&[]);
        let page =
            list_export_page(c, &build_base_where(&job, 0), None, 100, &relations).expect("page");
        Ok(page.iter().map(|p| p.id).collect())
    })
    .await
    .expect("read")
}

fn keyword(k: &str) -> Option<ExportFilter> {
    Some(ExportFilter {
        keyword: Some(k.into()),
        contact_filter: None,
    })
}

#[tokio::test]
async fn build_base_where_scopes_by_contact_ids_in_the_workspace_when_no_filter() {
    let db = two_workspaces().await;
    seed_people(&db).await;
    assert_eq!(matched(&db, job(None, &[1, 3, 5], true)).await, vec![1, 3]);
    assert!(matched(&db, job(None, &[], true)).await.is_empty());
}

#[tokio::test]
async fn build_base_where_keyword_matches_name_email_and_phone_case_insensitively() {
    let db = two_workspaces().await;
    seed_people(&db).await;
    // `AcMe` -> `%acme%` over firstName, lastName, email, phoneNumber.
    assert_eq!(
        matched(&db, job(keyword("AcMe"), &[], true)).await,
        vec![1, 2, 3]
    );
}

#[tokio::test]
async fn build_base_where_keyword_omits_email_and_phone_when_email_phone_export_is_denied() {
    let db = two_workspaces().await;
    seed_people(&db).await;
    assert_eq!(
        matched(&db, job(keyword("acme"), &[], false)).await,
        vec![1]
    );
}

#[tokio::test]
async fn build_base_where_an_empty_filter_is_the_workspace_and_ignores_contact_ids() {
    let db = two_workspaces().await;
    seed_people(&db).await;
    let empty = Some(ExportFilter::default());
    assert_eq!(
        matched(&db, job(empty.clone(), &[], true)).await,
        vec![1, 2, 3, 4]
    );
    assert_eq!(matched(&db, job(empty, &[4], true)).await, vec![1, 2, 3, 4]);
}

fn criteria(value: serde_json::Value) -> Option<ExportFilter> {
    Some(ExportFilter {
        keyword: None,
        contact_filter: Some(to_filter_criteria(&value)),
    })
}

#[tokio::test]
async fn build_base_where_applies_the_contact_filter_and_drops_email_phone_conditions_when_denied()
{
    let db = two_workspaces().await;
    seed_people(&db).await;
    let has_email = json!({ "operator": "and", "conditions": [
        { "field": "email", "operator": "isNotEmpty" }
    ] });
    assert_eq!(
        matched(&db, job(criteria(has_email.clone()), &[], true)).await,
        vec![2]
    );
    assert_eq!(
        matched(&db, job(criteria(has_email), &[], false)).await,
        vec![1, 2, 3, 4]
    );
}

#[tokio::test]
async fn build_base_where_combines_keyword_and_contact_filter() {
    let db = two_workspaces().await;
    seed_people(&db).await;
    let mut both = criteria(json!({ "operator": "and", "conditions": [
        { "field": "email", "operator": "isNotEmpty" }
    ] }))
    .expect("filter");
    both.keyword = Some("acme".into());
    assert_eq!(matched(&db, job(Some(both), &[], true)).await, vec![2]);
}

#[tokio::test]
async fn build_base_where_includes_the_assigned_user_conversation_scope_when_provided() {
    let db = two_workspaces().await;
    seed_people(&db).await;
    setup(&db, |tx| {
        conversation(tx, 61, WS, 1)?;
        conversation(tx, 62, WS, 2)?;
        tx.execute(
            r#"UPDATE "Conversation" SET "assignedUserId" = 1001 WHERE "id" = 61"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let mut scoped = job(None, &[1, 2], true);
    scoped.restrict_to_assigned_user_id = Some(1001);
    assert_eq!(matched(&db, scoped.clone()).await, vec![1]);
    scoped.filter = Some(ExportFilter::default());
    assert_eq!(matched(&db, scoped).await, vec![1]);
}

// ── contactRepository.listForExportPage ──────────────────────────────────

#[tokio::test]
async fn list_for_export_page_keys_on_id_asc_and_loads_only_the_selected_relations() {
    let db = two_workspaces().await;
    seed_people(&db).await;
    setup(&db, |tx| {
        inbox(tx, 701, WS, "messenger")?;
        inbox(tx, 702, WS, "whatsapp")?;
        contact_inbox(tx, 801, 1, 701, "messenger", "psid-1")?;
        contact_inbox(tx, 802, 1, 702, "whatsapp", "84900")?;
        tx.execute(r#"UPDATE "ContactInbox" SET "sourceUserId" = 'user.1' WHERE "id" = 802"#, [])?;
        custom_field(tx, 11, WS, "Plan", "shortText")?;
        custom_field(tx, 12, WS, "Other", "shortText")?;
        tx.execute(
            r#"INSERT INTO "ContactCustomField" ("contactId", "customFieldId", "value") VALUES (1, 11, 'Pro'), (1, 12, 'x')"#,
            [],
        )?;
        tag(tx, 9, WS, "VIP")?;
        tag(tx, 10, WS, "Other")?;
        tag_link(tx, 1, 9)?;
        tag_link(tx, 1, 10)?;
        Ok(())
    })
    .await;
    let none = ExportRelations {
        custom_field_ids: vec![],
        tag_ids: vec![],
        contact_inboxes: InboxRelation::None,
    };
    let rows = page(&db, none.clone(), Some(1), 2).await;
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![2, 3]);
    let rows = page(&db, none, None, 1).await;
    assert!(
        rows[0].custom_fields.is_empty()
            && rows[0].tag_ids.is_empty()
            && rows[0].inboxes.is_empty()
    );
    let rows = page(
        &db,
        ExportRelations {
            custom_field_ids: s(&["11"]),
            tag_ids: s(&["9"]),
            contact_inboxes: InboxRelation::Earliest,
        },
        None,
        1,
    )
    .await;
    assert_eq!(
        rows[0].custom_fields,
        vec![("11".to_owned(), "Pro".to_owned())]
    );
    assert_eq!(rows[0].tag_ids, s(&["9"]));
    assert_eq!(rows[0].inboxes, vec![(Some("psid-1".to_owned()), None)]);
    let rows = page(
        &db,
        ExportRelations {
            custom_field_ids: vec![],
            tag_ids: vec![],
            contact_inboxes: InboxRelation::All,
        },
        None,
        1,
    )
    .await;
    assert_eq!(
        rows[0].inboxes,
        vec![
            (Some("psid-1".to_owned()), None),
            (Some("84900".to_owned()), Some("user.1".to_owned()))
        ]
    );
}

async fn page(
    db: &Db,
    relations: ExportRelations,
    last: Option<i64>,
    limit: usize,
) -> Vec<ExportContact> {
    chatbotx_db::repo::read(db, move |c| {
        let base = build_base_where(&job(Some(ExportFilter::default()), &[], true), 0);
        Ok(list_export_page(c, &base, last, limit, &relations).expect("page"))
    })
    .await
    .expect("read")
}

// ── loopableExportContacts ───────────────────────────────────────────────

struct Run {
    db: common::TestDb,
    files: Uploader,
}

async fn run_fixture() -> Run {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "Contact" ("id", "firstName", "lastName", "email", "workspaceId") VALUES (1, 'Jane', 'Doe', 'jane@example.com', ?1)"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    Run {
        db,
        files: Uploader::in_memory(),
    }
}

async fn export(
    run: &Run,
    fields: &[&str],
    filter: Option<ExportFilter>,
    ids: &[i64],
) -> ExportJob {
    let job = start(
        &run.db,
        StartExport {
            workspace_id: WS,
            requested_user_id: None,
            can_export_email_and_phone: true,
            fields: s(fields),
            export_all: filter.is_some(),
            filter,
            contact_ids: ids.to_vec(),
            ..StartExport::default()
        },
    )
    .await
    .expect("start");
    run_export(&run.db, &run.files, &job).await.expect("run");
    job
}

async fn csv(run: &Run, job: &ExportJob) -> Vec<u8> {
    run.files
        .get_object(&job.output_path)
        .await
        .expect("object")
        .to_vec()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec())
        .expect("utf8")
        .trim_start_matches('\u{FEFF}')
        .to_owned()
}

async fn file_state(db: &Db, id: i64) -> (String, Option<i64>, Option<String>) {
    db.read(move |c| {
        Ok(c.query_row(
            r#"SELECT "status", "fileSize", "meta" FROM "File" WHERE "id" = ?1"#,
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    })
    .await
    .expect("file")
}

#[tokio::test]
async fn run_export_returns_early_without_uploading_for_non_csv_format() {
    let run = run_fixture().await;
    let mut job = start(&run.db, start_input(&["sys:fullName"], true))
        .await
        .expect("start");
    job.output_format = "json".into();
    run_export(&run.db, &run.files, &job).await.expect("run");
    assert!(run.files.get_object(&job.output_path).await.is_err());
    assert_eq!(file_state(&run.db, job.file_id).await.0, "pending");
}

#[tokio::test]
async fn run_export_starts_the_file_with_a_utf8_bom_and_reports_its_size() {
    let run = run_fixture().await;
    setup(&run.db, |tx| {
        tx.execute(
            r#"UPDATE "Contact" SET "firstName" = 'Nguyễn Văn', "lastName" = 'Bình'"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let job = export(&run, &["sys:fullName", "sys:email"], None, &[1]).await;
    let bytes = csv(&run, &job).await;
    assert_eq!(&bytes[..3], &[0xef, 0xbb, 0xbf]);
    assert!(text(&bytes).contains("\"Nguyễn Văn Bình\""));
    let size = i64::try_from(bytes.len()).expect("size");
    assert_eq!(file_state(&run.db, job.file_id).await.1, Some(size));
}

#[tokio::test]
async fn run_export_round_trips_through_the_import_parser_without_a_bom_polluted_header() {
    let run = run_fixture().await;
    let job = export(&run, &["sys:fullName", "sys:email"], None, &[1]).await;
    let bytes = csv(&run, &job).await;
    let records =
        chatbotx_business::import::csv::parse_records(std::str::from_utf8(&bytes).expect("utf8"))
            .expect("parse");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0], s(&["Full Name", "Email"]));
    assert_eq!(records[1], s(&["Jane Doe", "jane@example.com"]));
}

#[tokio::test]
async fn run_export_exports_a_single_page_and_marks_the_file_uploaded() {
    let run = run_fixture().await;
    let job = export(&run, &["sys:fullName", "sys:email"], None, &[1]).await;
    assert_eq!(
        text(&csv(&run, &job).await),
        "\"Full Name\",\"Email\"\n\"Jane Doe\",\"jane@example.com\"\n"
    );
    let (status, size, meta) = file_state(&run.db, job.file_id).await;
    assert_eq!(status, "uploaded");
    assert!(size.is_some());
    assert_eq!(meta.as_deref(), Some(r#"{"totalRecords":1}"#));
}

#[tokio::test]
async fn run_export_paginates_with_keyset_across_pages() {
    // Adapted: the run is in the request, so the 5 s progress throttle has
    // no reader; the final count is asserted.
    let run = run_fixture().await;
    let n = i64::try_from(EXPORT_PAGE_SIZE * 2 + 1).expect("n");
    setup(&run.db, move |tx| {
        for id in 2..=n {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (?1, 'P', ?2)"#,
                params![id, WS],
            )?;
        }
        Ok(())
    })
    .await;
    let job = export(&run, &["sys:firstName"], Some(ExportFilter::default()), &[]).await;
    let body = text(&csv(&run, &job).await);
    assert_eq!(
        body.trim().split('\n').count(),
        usize::try_from(n).expect("n") + 1
    );
    let bytes = csv(&run, &job).await;
    assert_eq!(
        bytes
            .windows(3)
            .filter(|w| *w == [0xef, 0xbb, 0xbf])
            .count(),
        1
    );
    let (status, _, meta) = file_state(&run.db, job.file_id).await;
    assert_eq!(status, "uploaded");
    assert_eq!(meta, Some(format!(r#"{{"totalRecords":{n}}}"#)));
}

#[tokio::test]
async fn run_export_stops_paging_when_a_full_page_is_followed_by_an_empty_page() {
    let run = run_fixture().await;
    let n = i64::try_from(EXPORT_PAGE_SIZE).expect("n");
    setup(&run.db, move |tx| {
        for id in 2..=n {
            tx.execute(
                r#"INSERT INTO "Contact" ("id", "firstName", "workspaceId") VALUES (?1, 'P', ?2)"#,
                params![id, WS],
            )?;
        }
        Ok(())
    })
    .await;
    let job = export(&run, &["sys:firstName"], Some(ExportFilter::default()), &[]).await;
    assert_eq!(
        file_state(&run.db, job.file_id).await.2,
        Some(format!(r#"{{"totalRecords":{n}}}"#))
    );
}

#[tokio::test]
async fn run_export_marks_the_file_failed_when_no_contact_matches() {
    let run = run_fixture().await;
    let job = export(&run, &["sys:fullName", "sys:email"], None, &[999]).await;
    assert_eq!(text(&csv(&run, &job).await), "\"Full Name\",\"Email\"\n");
    let (status, _, meta) = file_state(&run.db, job.file_id).await;
    assert_eq!(status, "failed");
    assert_eq!(meta.as_deref(), Some(r#"{"totalRecords":0}"#));
}

#[tokio::test]
async fn run_export_marks_the_file_failed_and_returns_the_error_when_the_query_errors() {
    let run = run_fixture().await;
    let job = start(&run.db, start_input(&["sys:fullName"], true))
        .await
        .expect("start");
    setup(&run.db, |tx| {
        tx.execute(r#"DROP TABLE "ContactToTag""#, [])?;
        Ok(())
    })
    .await;
    let mut broken = job.clone();
    broken.fields = s(&["tag:1"]);
    run_export(&run.db, &run.files, &broken)
        .await
        .expect_err("query error");
    assert_eq!(file_state(&run.db, job.file_id).await.0, "failed");
}

#[tokio::test]
async fn run_export_renders_tag_membership_and_custom_field_values() {
    let run = run_fixture().await;
    setup(&run.db, |tx| {
        contact_with(tx, 2, WS, Some("Ken"), None, None)?;
        tag(tx, 9, WS, "VIP")?;
        tag_link(tx, 1, 9)?;
        custom_field(tx, 11, WS, "Plan", "shortText")?;
        tx.execute(
            r#"INSERT INTO "ContactCustomField" ("contactId", "customFieldId", "value") VALUES (1, 11, 'Pro')"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let job = export(&run, &["sys:firstName", "tag:9", "cus:11"], None, &[1, 2]).await;
    let body = text(&csv(&run, &job).await);
    let rows: Vec<&str> = body.trim().split('\n').collect();
    assert_eq!(
        rows,
        vec![
            "\"First Name\",\"VIP\",\"Plan\"",
            "\"Jane\",\"Yes\",\"Pro\"",
            "\"Ken\",\"No\",\"\""
        ]
    );
}

#[tokio::test]
async fn run_export_serializes_date_as_iso_and_null_as_an_empty_string() {
    let run = run_fixture().await;
    setup(&run.db, |tx| {
        tx.execute(
            r#"UPDATE "Contact" SET "blockedAt" = 1767323045000, "email" = NULL WHERE "id" = 1"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let job = export(&run, &["sys:blockedAt", "sys:email"], None, &[1]).await;
    let body = text(&csv(&run, &job).await);
    assert_eq!(
        body.trim().split('\n').nth(1),
        Some("\"2026-01-02T03:04:05.000Z\",\"\"")
    );
}

#[tokio::test]
async fn run_export_loads_the_ad_ids_of_a_page_only_when_the_column_is_selected() {
    let run = run_fixture().await;
    setup(&run.db, |tx| {
        contact_with(tx, 2, WS, Some("B"), None, None)?;
        inbox(tx, 701, WS, "messenger")?;
        contact_inbox(tx, 801, 1, 701, "messenger", "psid-1")?;
        for (id, ad, at) in [(1, "1001", 1), (2, "1002", 2)] {
            tx.execute(
                r#"INSERT INTO "ContactReferralEvent" ("id", "workspaceId", "contactId", "contactInboxId", "channel", "type", "occurredAt", "adId", "dedupKey")
                   VALUES (?1, ?2, 1, 801, 'messenger', 'ad', ?3, ?4, ?5)"#,
                params![id, WS, at, ad, format!("k{id}")],
            )?;
        }
        Ok(())
    })
    .await;
    let job = export(&run, &["sys:firstName", "sys:adIds"], None, &[1, 2]).await;
    let body = text(&csv(&run, &job).await);
    assert_eq!(
        body.trim().split('\n').skip(1).collect::<Vec<_>>(),
        vec!["\"Jane\",\"1001, 1002\"", "\"B\",\"\""]
    );
}

#[tokio::test]
async fn run_export_applies_keyword_and_contact_filter_when_a_filter_is_supplied() {
    let run = run_fixture().await;
    setup(&run.db, |tx| {
        contact_with(tx, 2, WS, Some("Acme"), None, None)?;
        Ok(())
    })
    .await;
    let filter = ExportFilter {
        keyword: Some("Acme".into()),
        contact_filter: Some(to_filter_criteria(
            &json!({ "operator": "and", "conditions": [] }),
        )),
    };
    let job = export(&run, &["sys:firstName"], Some(filter), &[]).await;
    assert_eq!(text(&csv(&run, &job).await), "\"First Name\"\n\"Acme\"\n");
}

#[tokio::test]
async fn run_export_escapes_a_formula_injection_value_in_an_exported_contact_field() {
    let run = run_fixture().await;
    setup(&run.db, |tx| {
        tx.execute(
            r#"UPDATE "Contact" SET "firstName" = '=cmd|'' /C calc''!A1', "lastName" = NULL WHERE "id" = 1"#,
            [],
        )?;
        Ok(())
    })
    .await;
    let job = export(&run, &["sys:fullName", "sys:email"], None, &[1]).await;
    let body = text(&csv(&run, &job).await);
    assert_eq!(
        body.trim().split('\n').nth(1),
        Some("\"'=cmd|' /C calc'!A1\",\"jane@example.com\"")
    );
}
