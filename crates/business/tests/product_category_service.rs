//! Port of `packages/business/__tests__/product-category-service.test.ts`.
//! Adapted: the mocked repository calls become real rows in a migrated
//! database; "was called with" becomes the stored rows. Template locks, delete
//! guards and the cache are not ported (n-a rows in the test map).

mod common;

use chatbotx_business::BusinessError;
use chatbotx_business::product_category::{
    self as svc, CategoryPath, CreateCategory, LevelInput, ListFilter, UpdateCategory,
};
use chatbotx_business::tx;
use common::{
    OTHER_WS, WS, category, discount_code, product, scalar_i64, scalar_text, setup, two_workspaces,
    vendor,
};

fn create(name: &str, parent_id: Option<i64>) -> CreateCategory {
    CreateCategory {
        workspace_id: WS,
        name: name.to_string(),
        parent_id,
        ..CreateCategory::default()
    }
}

fn update(category_id: i64) -> UpdateCategory {
    UpdateCategory {
        workspace_id: WS,
        category_id,
        ..UpdateCategory::default()
    }
}

fn message<T: std::fmt::Debug>(result: Result<T, BusinessError>) -> String {
    result.expect_err("expected an error").message()
}

async fn count_in(db: &chatbotx_db::Db, workspace_id: i64) -> i64 {
    scalar_i64(
        db,
        r#"SELECT count(*) FROM "ProductCategory" WHERE "workspaceId" = ?1"#,
        workspace_id,
    )
    .await
    .unwrap_or(0)
}

async fn parent_of(db: &chatbotx_db::Db, id: i64) -> Option<i64> {
    scalar_i64(
        db,
        r#"SELECT "parentId" FROM "ProductCategory" WHERE "id" = ?1"#,
        id,
    )
    .await
}

async fn filing(db: &chatbotx_db::Db, table: &'static str, id: i64) -> (Option<i64>, Option<i64>) {
    let (cat, sub) = match table {
        "Product" => (
            r#"SELECT "categoryId" FROM "Product" WHERE "id" = ?1"#,
            r#"SELECT "subcategoryId" FROM "Product" WHERE "id" = ?1"#,
        ),
        _ => (
            r#"SELECT "categoryId" FROM "Vendor" WHERE "id" = ?1"#,
            r#"SELECT "subcategoryId" FROM "Vendor" WHERE "id" = ?1"#,
        ),
    };
    (scalar_i64(db, cat, id).await, scalar_i64(db, sub, id).await)
}

// --- name handling -------------------------------------------------------

#[tokio::test]
async fn product_category_service_name_handling_trims_the_name_before_it_reaches_the_database() {
    let db = two_workspaces().await;
    let row = svc::create(&db, create("  Shoes  ", None))
        .await
        .expect("create");
    let stored = scalar_text(
        &db,
        r#"SELECT "name" FROM "ProductCategory" WHERE "id" = ?1"#,
        row.id,
    )
    .await;
    assert_eq!(
        (row.name.as_str(), stored.as_deref()),
        ("Shoes", Some("Shoes"))
    );
}

#[tokio::test]
async fn product_category_service_name_handling_rejects_a_name_that_is_only_whitespace() {
    let db = two_workspaces().await;
    let error = svc::create(&db, create("   ", None))
        .await
        .expect_err("blank");
    assert_eq!(
        (error.message(), error.code()),
        (
            "Product category name is required".to_string(),
            Some("productCategoryNameRequired")
        )
    );
    assert_eq!(count_in(&db, WS).await, 0);
}

#[tokio::test]
async fn product_category_service_name_handling_reports_a_unique_violation_as_a_duplicate_rather_than_a_raw_db_error()
 {
    let db = two_workspaces().await;
    svc::create(&db, create("Shoes", None))
        .await
        .expect("first");
    let error = svc::create(&db, create("Shoes", None))
        .await
        .expect_err("duplicate");
    assert_eq!(
        (error.message(), error.code(), error.status()),
        (
            "Product category already exists".to_string(),
            Some("productCategoryDuplicated"),
            Some(400)
        )
    );
}

// --- depth limit ---------------------------------------------------------

#[tokio::test]
async fn product_category_service_depth_limit_rejects_a_category_that_would_be_its_own_parent() {
    let db = two_workspaces().await;
    setup(&db, |tx| category(tx, 1, WS, None, "Shoes")).await;
    let result = svc::update(
        &db,
        UpdateCategory {
            parent_id: Some(Some(1)),
            ..update(1)
        },
    )
    .await;
    assert!(message(result).contains("cannot be its own parent"));
    assert_eq!(parent_of(&db, 1).await, None);
}

#[tokio::test]
async fn product_category_service_depth_limit_rejects_a_parent_that_lives_in_another_workspace() {
    let db = two_workspaces().await;
    setup(&db, |tx| category(tx, 9, OTHER_WS, None, "Foreign")).await;
    let result = svc::create(&db, create("Sneakers", Some(9))).await;
    assert_eq!(message(result), "Parent product category not found");
    assert_eq!(count_in(&db, WS).await, 0);
}

#[tokio::test]
async fn product_category_service_depth_limit_rejects_a_sub_category_used_as_a_parent_keeping_the_tree_two_deep()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, 1, WS, None, "Parent")?;
        category(tx, 2, WS, Some(1), "Sub")
    })
    .await;
    let result = svc::create(&db, create("Running", Some(2))).await;
    assert!(message(result).contains("cannot contain further sub-categories"));
    assert_eq!(count_in(&db, WS).await, 2);
}

#[tokio::test]
async fn product_category_service_depth_limit_rejects_demoting_a_category_that_already_has_children()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, 1, WS, None, "Shoes")?;
        category(tx, 2, WS, Some(1), "Child")?;
        category(tx, 3, WS, None, "Parent")
    })
    .await;
    let result = svc::update(
        &db,
        UpdateCategory {
            parent_id: Some(Some(3)),
            ..update(1)
        },
    )
    .await;
    assert!(message(result).contains("cannot itself become a sub-category"));
    assert_eq!(parent_of(&db, 1).await, None);
}

#[tokio::test]
async fn product_category_service_depth_limit_accepts_a_top_level_parent_for_a_childless_category()
{
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, 1, WS, None, "Sneakers")?;
        category(tx, 3, WS, None, "Parent")
    })
    .await;
    let row = svc::update(
        &db,
        UpdateCategory {
            name: Some("Sneakers".into()),
            parent_id: Some(Some(3)),
            ..update(1)
        },
    )
    .await
    .expect("update");
    assert_eq!((row.id, row.parent_id), (1, Some(3)));
}

#[tokio::test]
async fn product_category_service_depth_limit_skips_the_parentage_checks_when_a_rename_carries_no_parent()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, 1, WS, None, "Parent")?;
        category(tx, 2, WS, Some(1), "Child")
    })
    .await;
    let row = svc::update(
        &db,
        UpdateCategory {
            name: Some("Renamed".into()),
            ..update(2)
        },
    )
    .await
    .expect("rename");
    assert_eq!((row.name.as_str(), row.parent_id), ("Renamed", Some(1)));
}

#[tokio::test]
async fn product_category_service_depth_limit_promotes_a_sub_category_back_to_the_top_level_on_an_explicit_null()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, 1, WS, None, "Parent")?;
        category(tx, 2, WS, Some(1), "Shoes")
    })
    .await;
    svc::update(
        &db,
        UpdateCategory {
            parent_id: Some(None),
            ..update(2)
        },
    )
    .await
    .expect("promote");
    assert_eq!(parent_of(&db, 2).await, None);
}

// --- re-filing on a level change ----------------------------------------

#[tokio::test]
async fn product_category_service_re_filing_on_a_level_change_files_products_and_vendors_of_a_category_moved_under_a_parent()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, 1, WS, None, "Parent")?;
        category(tx, 2, WS, None, "Moved")?;
        product(tx, 10, WS, Some(2), None)?;
        vendor(tx, 20, WS, "Acme")?;
        tx.execute(
            r#"UPDATE "Vendor" SET "categoryId" = 2 WHERE "id" = 20"#,
            [],
        )?;
        Ok(())
    })
    .await;
    svc::update(
        &db,
        UpdateCategory {
            parent_id: Some(Some(1)),
            ..update(2)
        },
    )
    .await
    .expect("move");
    assert_eq!(filing(&db, "Product", 10).await, (Some(1), Some(2)));
    assert_eq!(filing(&db, "Vendor", 20).await, (Some(1), Some(2)));
}

#[tokio::test]
async fn product_category_service_re_filing_on_a_level_change_files_them_back_to_the_top_level_on_promotion()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, 1, WS, None, "Parent")?;
        category(tx, 2, WS, Some(1), "Child")?;
        product(tx, 10, WS, Some(1), Some(2))
    })
    .await;
    svc::update(
        &db,
        UpdateCategory {
            parent_id: Some(None),
            ..update(2)
        },
    )
    .await
    .expect("promote");
    assert_eq!(filing(&db, "Product", 10).await, (Some(2), None));
}

#[tokio::test]
async fn product_category_service_re_filing_on_a_level_change_leaves_references_alone_when_the_parent_did_not_change()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, 1, WS, None, "Parent")?;
        category(tx, 2, WS, Some(1), "Child")?;
        // Filed through the wrong column: a re-file would move it.
        product(tx, 10, WS, Some(2), None)
    })
    .await;
    svc::update(
        &db,
        UpdateCategory {
            parent_id: Some(Some(1)),
            ..update(2)
        },
    )
    .await
    .expect("same parent");
    assert_eq!(filing(&db, "Product", 10).await, (Some(2), None));
}

// --- delete and discount codes ------------------------------------------

#[tokio::test]
async fn product_category_service_delete_and_discount_codes_deactivates_category_codes_of_the_category_and_its_children_before_deleting()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, 1, WS, None, "Cat")?;
        category(tx, 2, WS, Some(1), "Child")?;
        category(tx, 3, WS, None, "Other")?;
        discount_code(tx, 31, WS, "category", Some(1), "[]")?;
        discount_code(tx, 32, WS, "category", Some(2), "[]")?;
        discount_code(tx, 33, WS, "category", Some(3), "[]")?;
        Ok(())
    })
    .await;
    svc::delete(&db, WS, 1).await.expect("delete");
    let active = r#"SELECT "isActive" FROM "DiscountCode" WHERE "id" = ?1"#;
    assert_eq!(
        (
            scalar_i64(&db, active, 31).await,
            scalar_i64(&db, active, 32).await,
            scalar_i64(&db, active, 33).await
        ),
        (Some(0), Some(0), Some(1))
    );
    assert_eq!(count_in(&db, WS).await, 1);
}

// --- missing rows ---------------------------------------------------------

#[tokio::test]
async fn product_category_service_missing_rows_reports_a_missing_category_on_update_rather_than_reporting_success()
 {
    let db = two_workspaces().await;
    let result = svc::update(
        &db,
        UpdateCategory {
            name: Some("Shoes".into()),
            ..update(404)
        },
    )
    .await;
    assert_eq!(message(result), "Product category not found");
}

#[tokio::test]
async fn product_category_service_missing_rows_reports_a_missing_category_on_delete() {
    let db = two_workspaces().await;
    setup(&db, |tx| category(tx, 9, OTHER_WS, None, "Foreign")).await;
    let error = svc::delete(&db, WS, 9).await.expect_err("foreign");
    assert_eq!(
        (error.message(), error.code()),
        ("Product category not found".to_string(), Some("notFound"))
    );
    assert_eq!(count_in(&db, OTHER_WS).await, 1);
}

// --- resolveByNames -------------------------------------------------------

async fn by_names(
    db: &chatbotx_db::Db,
    names: &[&str],
    create_missing: bool,
) -> std::collections::HashMap<String, i64> {
    let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
    tx::write(db, move |tx| {
        svc::resolve_by_names(tx, WS, &names, create_missing)
    })
    .await
    .expect("resolve")
}

#[tokio::test]
async fn product_category_service_resolve_by_names_keys_the_result_by_lower_cased_name_so_importers_can_match_either_casing()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| category(tx, 1, WS, None, "Shoes")).await;
    assert_eq!(
        by_names(&db, &["Shoes"], false).await.get("shoes"),
        Some(&1)
    );
}

#[tokio::test]
async fn product_category_service_resolve_by_names_matches_an_existing_category_regardless_of_casing_without_creating_a_duplicate()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| category(tx, 1, WS, None, "Electronics")).await;
    assert_eq!(
        by_names(&db, &["electronics"], true)
            .await
            .get("electronics"),
        Some(&1)
    );
    assert_eq!(count_in(&db, WS).await, 1);
}

#[tokio::test]
async fn product_category_service_resolve_by_names_collapses_names_that_differ_only_by_case_into_a_single_creation()
 {
    let db = two_workspaces().await;
    let resolved = by_names(&db, &[" Shoes ", "SHOES", "shoes", "", "   "], true).await;
    assert_eq!(resolved.len(), 1);
    let id = resolved["shoes"];
    let name = scalar_text(
        &db,
        r#"SELECT "name" FROM "ProductCategory" WHERE "id" = ?1"#,
        id,
    )
    .await;
    assert_eq!(
        (count_in(&db, WS).await, name.as_deref()),
        (1, Some("shoes"))
    );
}

#[tokio::test]
async fn product_category_service_resolve_by_names_creates_the_missing_rows_only_when_the_import_asked_it_to()
 {
    let db = two_workspaces().await;
    assert!(by_names(&db, &["Bags"], false).await.is_empty());
    assert_eq!(count_in(&db, WS).await, 0);
    let resolved = by_names(&db, &["Bags"], true).await;
    let name = scalar_text(
        &db,
        r#"SELECT "name" FROM "ProductCategory" WHERE "id" = ?1"#,
        resolved["bags"],
    )
    .await;
    assert_eq!(name.as_deref(), Some("Bags"));
}

#[tokio::test]
async fn product_category_service_resolve_by_names_does_not_create_anything_when_the_name_already_exists()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| category(tx, 1, WS, None, "Shoes")).await;
    by_names(&db, &["Shoes"], true).await;
    assert_eq!(count_in(&db, WS).await, 1);
}

// --- resolvePaths ---------------------------------------------------------

fn path(category_name: &str, subcategory_name: Option<&str>) -> CategoryPath {
    CategoryPath {
        category_name: category_name.to_string(),
        subcategory_name: subcategory_name.map(str::to_string),
    }
}

async fn paths(
    db: &chatbotx_db::Db,
    paths: Vec<CategoryPath>,
    create_missing: bool,
) -> std::collections::HashMap<String, svc::ResolvedPath> {
    tx::write(db, move |tx| {
        svc::resolve_paths(tx, WS, &paths, create_missing)
    })
    .await
    .expect("resolve paths")
}

async fn id_named(db: &chatbotx_db::Db, name: &'static str) -> Option<i64> {
    let sql: &'static str = match name {
        "Home" => {
            r#"SELECT "id" FROM "ProductCategory" WHERE "name" = 'Home' AND "workspaceId" = ?1"#
        }
        "Robot vacuums" => {
            r#"SELECT "id" FROM "ProductCategory" WHERE "name" = 'Robot vacuums' AND "workspaceId" = ?1"#
        }
        _ => {
            r#"SELECT "id" FROM "ProductCategory" WHERE "name" = 'Vacuums' AND "workspaceId" = ?1"#
        }
    };
    scalar_i64(db, sql, WS).await
}

#[tokio::test]
async fn product_category_service_resolve_paths_creates_and_resolves_a_two_level_category_path_in_one_transaction()
 {
    let db = two_workspaces().await;
    let resolved = paths(&db, vec![path(" Home ", Some(" Robot vacuums "))], true).await;
    let home = id_named(&db, "Home").await.expect("Home created");
    let robot = id_named(&db, "Robot vacuums").await.expect("child created");
    assert_eq!(parent_of(&db, robot).await, Some(home));
    assert_eq!(
        resolved.get("home\u{0}robot vacuums"),
        Some(&svc::ResolvedPath {
            category_id: home,
            subcategory_id: Some(robot)
        })
    );
}

#[tokio::test]
async fn product_category_service_resolve_paths_returns_nothing_for_a_path_that_doesn_t_exist_when_create_missing_is_false()
 {
    let db = two_workspaces().await;
    let resolved = paths(&db, vec![path("Ghost", None)], false).await;
    assert_eq!((resolved.len(), count_in(&db, WS).await), (0, 0));
}

#[tokio::test]
async fn product_category_service_resolve_paths_creates_only_the_missing_sub_category_when_the_top_level_already_exists()
 {
    let db = two_workspaces().await;
    setup(&db, |tx| category(tx, 1, WS, None, "Home")).await;
    let resolved = paths(&db, vec![path("Home", Some("Vacuums"))], true).await;
    let vacuums = id_named(&db, "Vacuums").await.expect("child created");
    assert_eq!(
        (count_in(&db, WS).await, parent_of(&db, vacuums).await),
        (2, Some(1))
    );
    assert_eq!(
        resolved.get("home\u{0}vacuums"),
        Some(&svc::ResolvedPath {
            category_id: 1,
            subcategory_id: Some(vacuums)
        })
    );
}

#[tokio::test]
async fn product_category_service_resolve_paths_collapses_equivalent_paths_that_differ_only_in_whitespace_into_one_resolution()
 {
    let db = two_workspaces().await;
    let resolved = paths(
        &db,
        vec![
            path("Home", Some("Vacuums")),
            path("  Home  ", Some("  Vacuums  ")),
        ],
        true,
    )
    .await;
    assert_eq!((resolved.len(), count_in(&db, WS).await), (1, 2));
    let home = id_named(&db, "Home").await.expect("Home");
    let vacuums = id_named(&db, "Vacuums").await.expect("Vacuums");
    assert_eq!(
        resolved.get("home\u{0}vacuums"),
        Some(&svc::ResolvedPath {
            category_id: home,
            subcategory_id: Some(vacuums)
        })
    );
}

// --- listing ----------------------------------------------------------------

const MEN: i64 = 1;
const WOMEN: i64 = 2;
const MEN_SHIRTS: i64 = 3;
const MEN_SHOES: i64 = 4;
const WOMEN_SHIRTS: i64 = 5;

async fn tree() -> common::TestDb {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        category(tx, MEN, WS, None, "Men")?;
        category(tx, WOMEN, WS, None, "Women")?;
        category(tx, MEN_SHIRTS, WS, Some(MEN), "Shirts")?;
        category(tx, MEN_SHOES, WS, Some(MEN), "Shoes")?;
        category(tx, WOMEN_SHIRTS, WS, Some(WOMEN), "Shirts")?;
        product(tx, 11, WS, Some(MEN), Some(MEN_SHIRTS))?;
        product(tx, 12, WS, Some(MEN), Some(MEN_SHIRTS))?;
        product(tx, 13, WS, Some(MEN), None)?;
        product(tx, 14, WS, Some(WOMEN), Some(WOMEN_SHIRTS))?;
        Ok(())
    })
    .await;
    db
}

#[tokio::test]
async fn product_category_service_listing_list_returns_the_whole_tree_without_a_filter() {
    let db = tree().await;
    let rows = svc::list(&db, WS, ListFilter::default())
        .await
        .expect("list");
    let mut counts: Vec<(i64, i64)> = rows.iter().map(|r| (r.id, r.product_count)).collect();
    counts.sort_unstable();
    assert_eq!(
        counts,
        vec![
            (MEN, 3),
            (WOMEN, 1),
            (MEN_SHIRTS, 2),
            (MEN_SHOES, 0),
            (WOMEN_SHIRTS, 1)
        ]
    );
}

#[tokio::test]
async fn product_category_service_listing_list_keeps_one_category_s_children_and_matches_names_ignoring_case()
 {
    let db = tree().await;
    let rows = svc::list(
        &db,
        WS,
        ListFilter {
            parent_id: Some(MEN),
            name: Some("SHI".into()),
        },
    )
    .await
    .expect("list");
    assert_eq!(
        rows.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![MEN_SHIRTS]
    );
}

#[tokio::test]
async fn product_category_service_listing_list_level_pages_the_top_level_and_counts_each_row_s_children()
 {
    let db = tree().await;
    let level = svc::list_level(
        &db,
        LevelInput {
            workspace_id: WS,
            page: Some(1),
            per_page: Some(1),
            ..LevelInput::default()
        },
    )
    .await
    .expect("level");
    let data: Vec<(i64, i64)> = level
        .data
        .iter()
        .map(|r| (r.category.id, r.child_count))
        .collect();
    assert_eq!(
        (data, level.page_count, level.parent),
        (vec![(MEN, 2)], 2, None)
    );
}

#[tokio::test]
async fn product_category_service_listing_list_level_lists_a_parent_s_children_with_the_parent_itself()
 {
    let db = tree().await;
    let level = svc::list_level(
        &db,
        LevelInput {
            workspace_id: WS,
            parent_id: Some(WOMEN),
            ..LevelInput::default()
        },
    )
    .await
    .expect("level");
    let data: Vec<(i64, i64)> = level
        .data
        .iter()
        .map(|r| (r.category.id, r.child_count))
        .collect();
    assert_eq!(data, vec![(WOMEN_SHIRTS, 0)]);
    assert_eq!(
        level.parent.map(|p| (p.id, p.name)),
        Some((WOMEN, "Women".to_string()))
    );
}
