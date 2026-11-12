//! Port of `packages/database/__tests__/reflink-repository.test.ts` (P3.3). The mocked
//! `findMany` / `findFirst` / `$count` become real `Reflink` rows: the asserted `where`
//! (workspace, `type = refLink`, keyword ilike) and `with` arguments are checked as the
//! rows and relations returned.

mod common;
#[macro_use]
#[path = "common/repo.rs"]
mod repo_common;
#[path = "common/repo3.rs"]
mod repo3;

use chatbotx_db::repo::reflink::{
    ReflinkListInput, count, find_by_id_and_workspace, list_paginated,
};
use chatbotx_db::repo::{self};
use common::{fresh_db, seed, seed_workspace};
use repo_common::{ok, setup};

const WS: i64 = 2001;
const OTHER: i64 = 2002;

/// Ref links 1 ("Promo link", custom field + logo), 2 ("Other"), a QR code 3 ("Promo qr")
/// of 2001, and ref link 4 ("Promo elsewhere") of 2002.
async fn seeded() -> common::TestDb {
    let db = fresh_db().await;
    seed(&db).await;
    seed_workspace(&db, 1009, OTHER).await;
    setup(&db, |tx| {
        repo3::flow(tx, 61, WS)?;
        repo3::flow(tx, 62, OTHER)?;
        ins!(tx, "CustomField", { "id" => 71_i64, "name" => "cf", "type" => "shortText", "workspaceId" => WS })?;
        ins!(tx, "MediaLibraryFile", { "id" => 81_i64, "name" => "logo", "path" => "files/logo.png", "mimeType" => "image/png", "size" => 1_i64, "workspaceId" => WS })?;
        ins!(tx, "Reflink", { "id" => 1_i64, "name" => "Promo link", "flowId" => 61_i64, "workspaceId" => WS, "customFieldId" => 71_i64, "widgetLogoFileId" => 81_i64 })?;
        ins!(tx, "Reflink", { "id" => 2_i64, "name" => "Other", "flowId" => 61_i64, "workspaceId" => WS })?;
        ins!(tx, "Reflink", { "id" => 3_i64, "name" => "Promo qr", "type" => "qrCode", "flowId" => 61_i64, "workspaceId" => WS })?;
        ins!(tx, "Reflink", { "id" => 4_i64, "name" => "Promo elsewhere", "flowId" => 62_i64, "workspaceId" => OTHER })?;
        Ok(())
    })
    .await;
    db
}

fn input(keyword: Option<&str>) -> ReflinkListInput {
    ReflinkListInput {
        workspace_id: WS,
        keyword: keyword.map(str::to_owned),
        page: 1,
        per_page: 50,
        sort: None,
    }
}

/// Port of: packages/database/__tests__/reflink-repository.test.ts:37 "reflinkRepository.listPaginated > scopes the query by workspaceId and forces type=refLink"
#[tokio::test]
async fn reflink_repository_list_paginated_scopes_the_query_by_workspace_id_and_forces_type_ref_link()
 {
    let db = seeded().await;
    let rows = ok(repo::read(&db, |c| list_paginated(c, &input(None))).await);
    let mut ids: Vec<i64> = rows.iter().map(|r| r.reflink.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2]);
    assert!(
        rows.iter()
            .all(|r| r.reflink.workspace_id == WS && r.reflink.r#type == "refLink")
    );
}

/// Port of: packages/database/__tests__/reflink-repository.test.ts:54 "reflinkRepository.listPaginated > includes flow, customField and widget logo file relations"
#[tokio::test]
async fn reflink_repository_list_paginated_includes_flow_custom_field_and_widget_logo_file_relations()
 {
    let db = seeded().await;
    let rows = ok(repo::read(&db, |c| list_paginated(c, &input(None))).await);
    let first = rows.iter().find(|r| r.reflink.id == 1).expect("reflink 1");
    assert_eq!(first.flow.as_ref().map(|f| f.id), Some(61));
    assert_eq!(first.custom_field.as_ref().map(|f| f.id), Some(71));
    assert_eq!(
        first.widget_logo_file_path.as_deref(),
        Some("files/logo.png")
    );
    let second = rows.iter().find(|r| r.reflink.id == 2).expect("reflink 2");
    assert_eq!(second.flow.as_ref().map(|f| f.id), Some(61));
    assert_eq!(second.custom_field, None);
    assert_eq!(second.widget_logo_file_path, None);
}

/// Port of: packages/database/__tests__/reflink-repository.test.ts:74 "reflinkRepository.listPaginated > filters by keyword using a contains-style ilike on name when given"
#[tokio::test]
async fn reflink_repository_list_paginated_filters_by_keyword_using_a_contains_style_ilike_on_name_when_given()
 {
    let db = seeded().await;
    // Case-insensitive contains: "PROMO" matches "Promo link" only (the QR code and the
    // other workspace's row stay out).
    let rows = ok(repo::read(&db, |c| list_paginated(c, &input(Some("PROMO")))).await);
    assert_eq!(
        rows.iter().map(|r| r.reflink.id).collect::<Vec<_>>(),
        vec![1]
    );
    // A LIKE wildcard in the keyword is literal.
    let none = ok(repo::read(&db, |c| list_paginated(c, &input(Some("%")))).await);
    assert!(none.is_empty());
}

/// Port of: packages/database/__tests__/reflink-repository.test.ts:90 "reflinkRepository.count > counts rows scoped to the workspace and type=refLink"
#[tokio::test]
async fn reflink_repository_count_counts_rows_scoped_to_the_workspace_and_type_ref_link() {
    let db = seeded().await;
    assert_eq!(ok(repo::read(&db, |c| count(c, WS, None)).await), 2);
    assert_eq!(
        ok(repo::read(&db, |c| count(c, WS, Some("promo"))).await),
        1
    );
}

/// Port of: packages/database/__tests__/reflink-repository.test.ts:104 "reflinkRepository.findByIdAndWorkspace > scopes the lookup by id, workspaceId, and type=refLink"
#[tokio::test]
async fn reflink_repository_find_by_id_and_workspace_scopes_the_lookup_by_id_workspace_id_and_type_ref_link()
 {
    let db = seeded().await;
    let found = ok(repo::read(&db, |c| find_by_id_and_workspace(c, WS, 1)).await);
    assert_eq!(found.map(|r| r.id), Some(1));
    // Another workspace's row and a QR code are not found.
    assert_eq!(
        ok(repo::read(&db, |c| find_by_id_and_workspace(c, WS, 4)).await),
        None
    );
    assert_eq!(
        ok(repo::read(&db, |c| find_by_id_and_workspace(c, WS, 3)).await),
        None
    );
}

/// Port of: packages/database/__tests__/reflink-repository.test.ts:118 "reflinkRepository.findByIdAndWorkspace > returns undefined when no row matches"
#[tokio::test]
async fn reflink_repository_find_by_id_and_workspace_returns_undefined_when_no_row_matches() {
    let db = seeded().await;
    assert_eq!(
        ok(repo::read(&db, |c| find_by_id_and_workspace(c, WS, 999)).await),
        None
    );
}
