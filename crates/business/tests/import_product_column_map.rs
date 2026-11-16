//! Q.D1i product imports: the three `resolveProductImportColumnMap` cases of
//! `packages/business/__tests__/import-column-map.test.ts` (moved from
//! `import_column_map_and_template.rs`), over a migrated database and an
//! in-memory file store instead of the mocked `peekImportHeaders`; plus own
//! extractor and XLSX tests.

mod common;

use chatbotx_business::import::products::{
    ProductColumnMap, extract_product_draft, resolve_product_import_column_map,
};
use chatbotx_business::import::xlsx::{self, Cell};
use chatbotx_files::{PutOptions, Uploader};
use common::{WS, setup, two_workspaces};
use serde_json::{Map, Value, json};

/// A products import file `9` whose header row is `header`.
async fn products_file(header: &str) -> (common::TestDb, Uploader) {
    let db = two_workspaces().await;
    setup(&db, |tx| {
        tx.execute(
            r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType")
               VALUES (9, ?1, 'import', 'products', 'p.csv', 'p.csv', 'text/csv')"#,
            [WS],
        )?;
        Ok(())
    })
    .await;
    let files = Uploader::in_memory();
    files
        .put_object(
            "p.csv",
            format!("{header}\n1,2,3\n").into_bytes(),
            PutOptions::default(),
            None,
        )
        .await
        .expect("put");
    (db, files)
}

/// Port of: packages/business/__tests__/import-column-map.test.ts:18 "resolveProductImportColumnMap > keeps a caller's column map without reading the file"
#[tokio::test]
async fn resolve_product_import_column_map_keeps_a_caller_s_column_map_without_reading_the_file() {
    let db = two_workspaces().await;
    let map = ProductColumnMap {
        name: "Title".into(),
        ..ProductColumnMap::default()
    };
    // No file `1` exists: a read would fail.
    let out =
        resolve_product_import_column_map(&db, &Uploader::in_memory(), WS, 1, Some(map.clone()))
            .await
            .expect("map");
    assert_eq!(out, map);
}

/// Port of: packages/business/__tests__/import-column-map.test.ts:31 "resolveProductImportColumnMap > recognises the columns of a products file of the workspace"
#[tokio::test]
async fn resolve_product_import_column_map_recognises_the_columns_of_a_products_file_of_the_workspace()
 {
    let (db, files) = products_file("Tên sản phẩm,Giá bán,SKU").await;
    let out = resolve_product_import_column_map(&db, &files, WS, 9, None)
        .await
        .expect("map");
    assert_eq!(
        serde_json::to_value(out).expect("json"),
        json!({ "name": "Tên sản phẩm", "price": "Giá bán", "sku": "SKU" })
    );
}

/// Port of: packages/business/__tests__/import-column-map.test.ts:45 "resolveProductImportColumnMap > a file without a recognisable name column is a 422 listing its headers"
#[tokio::test]
async fn resolve_product_import_column_map_a_file_without_a_recognisable_name_column_is_a_422_listing_its_headers()
 {
    let (db, files) = products_file("Foo,Bar,Baz").await;
    let error = resolve_product_import_column_map(&db, &files, WS, 9, None)
        .await
        .expect_err("422");
    let chatbotx_business::BusinessError::Exception {
        code,
        field,
        message,
        status,
        ..
    } = error
    else {
        panic!("exception expected");
    };
    assert_eq!((code.as_str(), status), ("validation", 422));
    assert_eq!(field.as_deref(), Some("columnMap"));
    assert!(message.contains("file headers: Foo, Bar"), "{message}");
}

// ---- own tests ---------------------------------------------------------------

fn row(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("object")
}

#[test]
fn the_extractor_reads_numbers_with_thousands_separators_and_normalizes_urls() {
    let map = ProductColumnMap {
        name: "n".into(),
        price: Some("p".into()),
        inventory_quantity: Some("q".into()),
        product_url: Some("u".into()),
        sku: Some("s".into()),
        ..ProductColumnMap::default()
    };
    let draft = extract_product_draft(
        &row(json!({ "n": "  Mug ", "p": "1,250.5", "q": 3, "u": "HTTPS://Example.com", "s": 7 })),
        &map,
        2,
    )
    .expect("draft");
    assert_eq!(draft.name, "Mug");
    assert_eq!(draft.price, Some(1250.5));
    assert_eq!(draft.inventory_quantity, Some(3.0));
    assert_eq!(draft.product_url.as_deref(), Some("https://example.com/"));
    // `cleanText` keeps strings only: a numeric SKU cell is dropped.
    assert_eq!(draft.sku, None);
    assert_eq!(
        extract_product_draft(&row(json!({ "n": "x", "p": "-1" })), &map, 2),
        Err("Price cannot be negative".into())
    );
    assert_eq!(
        extract_product_draft(&row(json!({ "n": "x", "u": "not a url" })), &map, 2),
        Err("Product URL is invalid".into())
    );
}

#[test]
fn a_workbook_round_trips_text_numbers_and_skips_blank_rows() {
    let bytes = xlsx::write_workbook(
        "Sheet & 1",
        &[
            vec![
                Cell::Text("A <b>".into()),
                Cell::Empty,
                Cell::Text("C".into()),
            ],
            vec![Cell::Empty, Cell::Empty],
            vec![Cell::Number(1.5), Cell::Bool(true), Cell::Text("x".into())],
        ],
        24,
    );
    assert!(bytes.starts_with(b"PK"));
    assert_eq!(
        xlsx::read_headers(&bytes).expect("headers"),
        ["A <b>", "", "C"]
    );
    assert_eq!(
        xlsx::read_records(&bytes).expect("records"),
        vec![row(json!({ "A <b>": 1.5, "C": "x" }))]
    );
    assert!(
        xlsx::read_headers(b"not a zip")
            .expect_err("corrupt")
            .starts_with("Invalid XLSX file: ")
    );
}
