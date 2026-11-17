//! Q.D1i product imports: the product parts of
//! `apps/builder/__tests__/import-upload-url-public-api.test.ts` (moved here
//! from `import_upload_url_public_api.rs`) plus own route tests. Adapted:
//! the mocked `createImportUpload` / `peekImportHeaders` /
//! `resolveProductImportColumnMap` / `startProductImportJob` / template
//! builder become the real services over an in-memory store with seeded
//! rows.

mod handlers_common;

use std::sync::Arc;

use base64::Engine as _;
use chatbotx_api::routes::match_route;
use chatbotx_business::import::xlsx;
use chatbotx_files::{LocalUrlSigner, PutOptions, Uploader};
use handlers_common::{Fixture, WS, call, fixture};
use serde_json::{Value, json};

async fn with_files() -> (Fixture, Arc<Uploader>) {
    let mut f = fixture().await;
    let files = Arc::new(
        Uploader::in_memory()
            .with_url_signer(LocalUrlSigner::new("secret", "http://app/files").expect("signer")),
    );
    f.state = f.state.clone().with_files(Arc::clone(&files));
    (f, files)
}

async fn exec(f: &Fixture, sql: &'static str, values: Vec<i64>) {
    f.state
        .db
        .write(move |tx| {
            tx.execute(sql, chatbotx_db::rusqlite::params_from_iter(values.iter()))?;
            Ok(())
        })
        .await
        .expect("exec");
}

/// A products import file `id` in the workspace with `bytes` uploaded.
async fn upload(f: &Fixture, files: &Uploader, id: i64, name: &str, mime: &str, bytes: Vec<u8>) {
    let path = format!("workspaces/{WS}/imports/products/{name}");
    let (name, mime, stored) = (name.to_string(), mime.to_string(), path.clone());
    f.state
        .db
        .write(move |tx| {
            tx.execute(
                r#"INSERT INTO "File" ("id", "workspaceId", "contextType", "subType", "path", "fileName", "mimeType")
                   VALUES (?1, ?2, 'import', 'products', ?3, ?4, ?5)"#,
                chatbotx_db::rusqlite::params![id, WS, stored, name, mime],
            )?;
            Ok(())
        })
        .await
        .expect("file");
    files
        .put_object(&path, bytes, PutOptions::default(), None)
        .await
        .expect("put");
}

async fn query(f: &Fixture, sql: &'static str) -> Vec<Vec<Value>> {
    f.state
        .db
        .read(move |conn| {
            let mut stmt = conn.prepare(sql)?;
            let width = stmt.column_count();
            let rows = stmt.query_map([], |row| {
                (0..width)
                    .map(|i| {
                        Ok(match row.get_ref(i)? {
                            chatbotx_db::rusqlite::types::ValueRef::Null => Value::Null,
                            chatbotx_db::rusqlite::types::ValueRef::Integer(n) => json!(n),
                            chatbotx_db::rusqlite::types::ValueRef::Real(n) => json!(n),
                            chatbotx_db::rusqlite::types::ValueRef::Text(t) => {
                                json!(String::from_utf8_lossy(t))
                            }
                            chatbotx_db::rusqlite::types::ValueRef::Blob(_) => Value::Null,
                        })
                    })
                    .collect()
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
        .await
        .expect("query")
}

/// Port of: apps/builder/__tests__/import-upload-url-public-api.test.ts:100 "POST %s > mints an upload for the token's workspace as a ${type} import" (products)
#[tokio::test]
async fn post_v1_products_imports_upload_url_mints_an_upload_for_the_token_s_workspace_as_a_products_import()
 {
    let (f, _) = with_files().await;
    let (status, body) = call(
        &f,
        "products.createImportUpload",
        WS,
        &[],
        None,
        Some(json!({ "fileName": "x.csv", "mimeType": "text/csv", "fileSize": 10 })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let path = body["path"].as_str().expect("path");
    assert!(path.starts_with(&format!("workspaces/{WS}/imports/products/import_")));
    assert!(path.ends_with(".csv"), "{path}");
    let stored = query(
        &f,
        r#"SELECT "workspaceId", "userId", "subType" FROM "File""#,
    )
    .await;
    assert_eq!(
        stored,
        vec![vec![json!(WS), Value::Null, json!("products")]]
    );
}

/// Port of: apps/builder/__tests__/import-upload-url-public-api.test.ts:117 "POST %s > is gated by the ${scope} scope and returns 201" (products)
#[tokio::test]
async fn post_v1_products_imports_upload_url_is_gated_by_the_ecommerce_scope_and_returns_201() {
    let matched = match_route("POST", "/v1/products/imports/upload-url").expect("route");
    assert_eq!(matched.route.id, "products.createImportUpload");
    assert_eq!(
        format!("{:?}", matched.route.auth),
        format!(
            "{:?}",
            chatbotx_api::routes::RouteAuth::WorkspaceToken(&["ecommerce"])
        )
    );
}

/// Port of: apps/builder/__tests__/import-upload-url-public-api.test.ts:129 "GET %s > reads the headers of a file in the token's workspace and suggests a column map" (products)
#[tokio::test]
async fn get_v1_products_imports_files_file_id_headers_reads_the_headers_of_a_file_in_the_token_s_workspace_and_suggests_a_column_map()
 {
    let (f, files) = with_files().await;
    upload(
        &f,
        &files,
        9,
        "a.csv",
        "text/csv",
        b"Phone,Name\n1,2\n".to_vec(),
    )
    .await;
    let (status, body) = call(
        &f,
        "products.peekImportHeaders",
        WS,
        &[("fileId", "9")],
        None,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body,
        json!({ "headers": ["Phone", "Name"], "suggestedColumnMap": { "name": "Name" } })
    );
}

fn template_headers(body: &Value) -> Vec<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body["contentBase64"].as_str().expect("base64"))
        .expect("decode");
    xlsx::read_headers(&bytes).expect("xlsx")
}

/// Port of: apps/builder/__tests__/import-upload-url-public-api.test.ts:188 "GET /v1/products/import-template > returns the XLSX as base64 in the requested language"
#[tokio::test]
async fn get_v1_products_import_template_returns_the_xlsx_as_base64_in_the_requested_language() {
    let (f, _) = with_files().await;
    let (status, body) = call(
        &f,
        "products.getImportTemplate",
        WS,
        &[],
        Some("language=vi"),
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    // The real builder stands in for the mocked one: the name, the XLSX MIME
    // type and a workbook whose header row is in Vietnamese.
    assert_eq!(body["fileName"], json!("product-import-vi.xlsx"));
    assert_eq!(
        body["mimeType"],
        json!("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
    );
    assert_eq!(template_headers(&body)[0], "Tên sản phẩm");
}

/// Port of: apps/builder/__tests__/import-upload-url-public-api.test.ts:201 "GET /v1/products/import-template > falls back to the workspace language"
#[tokio::test]
async fn get_v1_products_import_template_falls_back_to_the_workspace_language() {
    let (f, _) = with_files().await;
    exec(
        &f,
        r#"UPDATE "Workspace" SET "language" = 'en' WHERE "id" = ?1"#,
        vec![WS],
    )
    .await;
    let (status, body) = call(&f, "products.getImportTemplate", WS, &[], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["fileName"], json!("product-import-en.xlsx"));
    assert_eq!(
        template_headers(&body),
        [
            "Product name",
            "SKU",
            "Price",
            "Discount",
            "Short description",
            "Category",
            "Vendor",
            "Inventory quantity",
            "Image URL"
        ]
    );
}

/// Port of: apps/builder/__tests__/import-upload-url-public-api.test.ts:233 "POST /v1/products/imports > without columnMap or format, imports with the recognised columns and the file's format"
#[tokio::test]
async fn post_v1_products_imports_without_columnmap_or_format_imports_with_the_recognised_columns_and_the_file_s_format()
 {
    let (f, files) = with_files().await;
    upload(&f, &files, 9, "p.csv", "text/csv", b"Name\nMug\n".to_vec()).await;
    let (status, body) = call(
        &f,
        "products.startImport",
        WS,
        &[],
        None,
        Some(json!({ "fileId": "9", "createMissingCategories": true })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let import_id = body["importId"].as_str().expect("id").to_string();
    let rows = query(
        &f,
        r#"SELECT "id", "format", "meta", "status", "successCount" FROM "Import""#,
    )
    .await;
    assert_eq!(rows[0][0].to_string(), import_id);
    assert_eq!(rows[0][1], json!("csv"));
    let meta: Value = serde_json::from_str(rows[0][2].as_str().expect("meta")).expect("json");
    assert_eq!(
        meta,
        json!({ "columnMap": { "name": "Name" }, "createMissingCategories": true })
    );
    // The run happens in the request.
    assert_eq!(
        (rows[0][3].clone(), rows[0][4].clone()),
        (json!("completed"), json!(1))
    );
}

// ---- own tests ---------------------------------------------------------------

#[tokio::test]
async fn a_csv_import_creates_products_categories_and_vendors_and_samples_the_failed_rows() {
    let (f, files) = with_files().await;
    let csv = "Product name,SKU,Price,Discount,Category,Brand,Quantity,Image\n\
               Tee,S1,\"199,000\",10,Clothing,Acme,25,https://example.com/a.jpg\n\
               ,S2,1,0,,,,\n\
               Hat,S3,abc,0,,,,\n\
               Cap,S4,5,150,,,,\n\
               Tote,S5,9,0,clothing,acme,1.5,\n\
               Bag,S6,9,0,Bags,,2,ftp://x\n";
    upload(&f, &files, 9, "p.csv", "text/csv", csv.as_bytes().to_vec()).await;
    let (status, body) = call(
        &f,
        "products.startImport",
        WS,
        &[],
        None,
        Some(json!({ "fileId": "9" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let id = body["importId"].as_str().expect("id").to_string();
    let (status, body) = call(&f, "products.getImport", WS, &[("id", &id)], None, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["type"], json!("products"));
    assert_eq!(body["status"], json!("completed"));
    assert_eq!(
        (
            &body["totalCount"],
            &body["successCount"],
            &body["failedCount"]
        ),
        (&json!(6), &json!(1), &json!(5))
    );
    assert_eq!(
        body["errorSample"],
        json!([
            { "row": 3, "reason": "Product name is required" },
            { "row": 4, "reason": "Price must be a number" },
            { "row": 5, "reason": "Discount must be between 0 and 100" },
            { "row": 6, "reason": "Inventory quantity must be a non-negative integer" },
            { "row": 7, "reason": "Image URL must use HTTP or HTTPS" },
        ])
    );
    let products = query(
        &f,
        r#"SELECT p."name", p."price", p."discount", p."inventoryQuantity", p."images", c."name", v."name"
           FROM "Product" p LEFT JOIN "ProductCategory" c ON c."id" = p."categoryId" LEFT JOIN "Vendor" v ON v."id" = p."vendorId""#,
    )
    .await;
    assert_eq!(
        products,
        vec![vec![
            json!("Tee"),
            json!(199000.0),
            json!(10.0),
            json!(25),
            json!(r#"[{"type":"link","url":"https://example.com/a.jpg"}]"#),
            json!("Clothing"),
            json!("Acme"),
        ]]
    );
}

#[tokio::test]
async fn an_unknown_category_fails_the_row_when_categories_may_not_be_created() {
    let (f, files) = with_files().await;
    upload(
        &f,
        &files,
        9,
        "p.csv",
        "text/csv",
        b"name,category\nTee,Clothing\nMug,\n".to_vec(),
    )
    .await;
    let (status, body) = call(
        &f,
        "products.startImport",
        WS,
        &[],
        None,
        Some(json!({ "fileId": "9", "createMissingCategories": false, "columnMap": { "name": "name", "category": "category" } })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let rows = query(
        &f,
        r#"SELECT "successCount", "failedCount", "errorSample" FROM "Import""#,
    )
    .await;
    assert_eq!(rows[0][0], json!(1));
    assert_eq!(rows[0][1], json!(1));
    assert_eq!(
        serde_json::from_str::<Value>(rows[0][2].as_str().expect("sample")).expect("json"),
        json!([{ "row": 2, "reason": "Unknown category: Clothing" }])
    );
    assert_eq!(
        query(&f, r#"SELECT count(*) FROM "ProductCategory""#).await,
        vec![vec![json!(0)]]
    );
}

#[tokio::test]
async fn the_xlsx_template_imports_its_two_example_rows() {
    let (f, files) = with_files().await;
    let (_, body) = call(
        &f,
        "products.getImportTemplate",
        WS,
        &[],
        Some("language=en"),
        None,
    )
    .await;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body["contentBase64"].as_str().expect("base64"))
        .expect("decode");
    let mime = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
    upload(&f, &files, 9, "t.xlsx", mime, bytes).await;
    let (status, body) = call(
        &f,
        "products.startImport",
        WS,
        &[],
        None,
        Some(json!({ "fileId": "9", "format": "xlsx" })),
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let products = query(
        &f,
        r#"SELECT "name", "sku", "price", "inventoryQuantity" FROM "Product" ORDER BY "sku""#,
    )
    .await;
    assert_eq!(
        products,
        vec![
            vec![
                json!("Classic T-shirt"),
                json!("SKU-001"),
                json!(199000.0),
                json!(25)
            ],
            vec![
                json!("Canvas tote"),
                json!("SKU-002"),
                json!(99000.0),
                json!(50)
            ],
        ]
    );
}

#[tokio::test]
async fn start_import_refuses_a_running_import_a_format_mismatch_and_a_file_without_a_name_column()
{
    let (f, files) = with_files().await;
    upload(
        &f,
        &files,
        9,
        "p.csv",
        "text/csv",
        b"Foo,Bar\n1,2\n".to_vec(),
    )
    .await;
    let start = |body: Value| call(&f, "products.startImport", WS, &[], None, Some(body));
    let (status, body) = start(json!({ "fileId": "9" })).await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["code"], json!("validation"));
    assert!(
        body["message"]
            .as_str()
            .expect("message")
            .contains("file headers: Foo, Bar"),
        "{body}"
    );
    let (status, body) =
        start(json!({ "fileId": "9", "format": "xlsx", "columnMap": { "name": "Foo" } })).await;
    assert_eq!(
        (status, body["code"].clone()),
        (422, json!("productImportFormatMismatch"))
    );
    let (status, body) = start(json!({ "fileId": "404", "columnMap": { "name": "Foo" } })).await;
    assert_eq!(
        (status, body["code"].clone()),
        (404, json!("productImportFileNotFound"))
    );
    exec(
        &f,
        r#"INSERT INTO "Import" ("id", "workspaceId", "fileId", "type", "format", "status", "meta")
           VALUES (77, ?1, 9, 'products', 'csv', 'processing', '{}')"#,
        vec![WS],
    )
    .await;
    let (status, body) = start(json!({ "fileId": "9", "columnMap": { "name": "Foo" } })).await;
    assert_eq!(
        (status, body["code"].clone()),
        (409, json!("productImportAlreadyRunning"))
    );
    let (status, body) = start(json!({ "fileId": "9", "columnMap": { "name": "" } })).await;
    assert_eq!(
        (status, body["code"].clone()),
        (422, json!("invalidRequestData"))
    );
}
