//! Port of `packages/database/__tests__/contact-export-page-repository.test.ts` (P3.2).
//! "The `with` loads X" becomes "the row carries relation X" on real rows (adapted).

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;

use chatbotx_db::repo::contact::{ExportInboxes, ExportRelations, ExportRow, list_for_export_page};
use chatbotx_db::repo::sql::Sql;
use chatbotx_db::repo::{self};
use repo_common::ok;

fn none() -> ExportRelations {
    ExportRelations {
        custom_field_ids: vec![],
        tag_ids: vec![],
        contact_inboxes: ExportInboxes::None,
    }
}

/// Contacts 10, 11, 12 of 2001; custom fields 1, 2, 3 on contact 10; tags 1, 2; two inbox rows.
async fn seeded() -> common::TestDb {
    let db = common::fresh_db().await;
    common::seed(&db).await;
    repo_common::setup(&db, |tx| {
        for id in [10_i64, 11, 12] {
            repo_common::insert_contact(tx, id, 2001)?;
        }
        for f in [1_i64, 2, 3] {
            ins!(tx, "CustomField", { "id" => f, "name" => format!("f{f}"), "type" => "shortText", "workspaceId" => 2001_i64 })?;
            ins!(tx, "ContactCustomField", { "id" => f, "value" => format!("v{f}"), "contactId" => 10_i64, "customFieldId" => f })?;
        }
        for t in [1_i64, 2] {
            ins!(tx, "Tag", { "id" => t, "name" => format!("t{t}"), "workspaceId" => 2001_i64 })?;
            ins!(tx, "ContactToTag", { "contactId" => 10_i64, "tagId" => t })?;
        }
        repo_common::insert_ci(tx, 20, 10, 3001, "messenger", "psid-a")?;
        repo_common::insert_ci(tx, 21, 10, 3001, "messenger", "psid-b")?;
        tx.execute(r#"UPDATE "ContactInbox" SET "sourceUserId" = 'wa-user' WHERE "id" = 21"#, [])?;
        Ok(())
    })
    .await;
    db
}

async fn page(
    db: &chatbotx_db::Db,
    filter: Sql,
    limit: i64,
    rel: ExportRelations,
) -> Vec<ExportRow> {
    ok(repo::read(db, move |c| list_for_export_page(c, &filter, limit, &rel)).await)
}

fn ws() -> Sql {
    Sql::bind("c.\"workspaceId\" = ", 2001_i64)
}

/// Port of: packages/database/__tests__/contact-export-page-repository.test.ts:49 "contactRepository.listForExportPage > passes the caller's where and limit through and keys the page on id asc"
#[tokio::test]
async fn contact_repository_list_for_export_page_passes_the_caller_s_where_and_limit_through_and_keys_the_page_on_id_asc()
 {
    let db = seeded().await;
    let filter = Sql::and([Some(ws()), Some(Sql::bind("c.\"id\" > ", 10_i64))]);
    let rows = page(&db, filter, 2, none()).await;
    assert_eq!(
        rows.iter().map(|r| r.contact.id).collect::<Vec<_>>(),
        vec![11, 12]
    );
}

/// Port of: packages/database/__tests__/contact-export-page-repository.test.ts:67 "contactRepository.listForExportPage > loads no relation when no column needs one"
#[tokio::test]
async fn contact_repository_list_for_export_page_loads_no_relation_when_no_column_needs_one() {
    let db = seeded().await;
    let rows = page(&db, ws(), 2, none()).await;
    assert!(rows.iter().all(|r| r.contact_custom_fields.is_none()
        && r.tags.is_none()
        && r.contact_inboxes.is_none()));
}

/// Port of: packages/database/__tests__/contact-export-page-repository.test.ts:71 "contactRepository.listForExportPage > loads only the selected custom fields, with just the columns a cell renders"
#[tokio::test]
async fn contact_repository_list_for_export_page_loads_only_the_selected_custom_fields_with_just_the_columns_a_cell_renders()
 {
    let db = seeded().await;
    let mut rel = none();
    rel.custom_field_ids = vec![1, 2];
    let rows = page(&db, ws(), 1, rel).await;
    assert_eq!(
        rows[0].contact_custom_fields,
        Some(vec![(1, Some("v1".to_owned())), (2, Some("v2".to_owned()))])
    );
    assert!(rows[0].tags.is_none());
}

/// Port of: packages/database/__tests__/contact-export-page-repository.test.ts:82 "contactRepository.listForExportPage > loads only the selected tags"
#[tokio::test]
async fn contact_repository_list_for_export_page_loads_only_the_selected_tags() {
    let db = seeded().await;
    let mut rel = none();
    rel.tag_ids = vec![1];
    let rows = page(&db, ws(), 1, rel).await;
    assert_eq!(rows[0].tags, Some(vec![1]));
    assert!(rows[0].contact_custom_fields.is_none());
}

/// Port of: packages/database/__tests__/contact-export-page-repository.test.ts:90 "contactRepository.listForExportPage > loads the earliest inbox row for the Contact Id column"
#[tokio::test]
async fn contact_repository_list_for_export_page_loads_the_earliest_inbox_row_for_the_contact_id_column()
 {
    let db = seeded().await;
    let mut rel = none();
    rel.contact_inboxes = ExportInboxes::Earliest;
    let rows = page(&db, ws(), 1, rel).await;
    assert_eq!(
        rows[0].contact_inboxes,
        Some(vec![("psid-a".to_owned(), None)])
    );
}

/// Port of: packages/database/__tests__/contact-export-page-repository.test.ts:100 "contactRepository.listForExportPage > loads every inbox row for the WhatsApp User ID column"
#[tokio::test]
async fn contact_repository_list_for_export_page_loads_every_inbox_row_for_the_whats_app_user_id_column()
 {
    let db = seeded().await;
    let mut rel = none();
    rel.contact_inboxes = ExportInboxes::All;
    let rows = page(&db, ws(), 1, rel).await;
    assert_eq!(rows[0].contact_inboxes.as_ref().map(Vec::len), Some(2));
}
