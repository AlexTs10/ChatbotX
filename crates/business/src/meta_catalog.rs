//! `meta-catalog/{integration,operation,sync-run}.service.ts` (the parts the
//! public routes use), the Graph calls of `integration-meta-catalog/apis/catalogs`
//! and the builder `features/products/lib/meta-catalog-operations.ts`.
//!
//! The import and push jobs (`importMetaCatalogProducts`,
//! `submitMetaCatalogSync`) are queued on the default queue with the original
//! payloads; their handlers belong to the worker port.

use std::future::Future;
use std::sync::OnceLock;
use std::time::Duration;

use chatbotx_core::encryption::{EncryptedData, Encryption};
use chatbotx_db::Db;
use chatbotx_db::repo::models::{IntegrationMetaCatalog, MetaCatalogSyncRun};
use chatbotx_db::repo::{self, new_id, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension, params};
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `GRAPH_API_URL`.
pub const GRAPH_API_URL: &str = "https://graph.facebook.com";
/// `DEFAULT_API_VERSION`.
pub const DEFAULT_API_VERSION: &str = "v24.0";

const NOT_FOUND: &str = "Meta Catalog integration not found";
const ALREADY_RUNNING: &str = "metaCatalogSyncAlreadyRunning";
const IMPORT_JOB: &str = "importMetaCatalogProducts";
const SUBMIT_JOB: &str = "submitMetaCatalogSync";

/// `ENGLISH_META_CATALOG_REASONS`. `queueImport` / `queueSync` are left out:
/// the original passes them as plain strings to `failImport` / `fail`, whose
/// `toPublicErrorMessage` stores the generic failure text instead.
pub struct Reasons {
    pub already_running: &'static str,
}

pub const ENGLISH_REASONS: Reasons = Reasons {
    already_running: "A catalog sync or import is already running for this workspace.",
};

/// `MetaCatalogStoredAuth`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredAuth {
    pub access_token: String,
    pub version: Option<String>,
}

impl StoredAuth {
    fn version(&self) -> &str {
        self.version.as_deref().unwrap_or(DEFAULT_API_VERSION)
    }
}

/// `MetaCatalog`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    pub id: String,
    pub name: Option<String>,
    pub business_id: Option<String>,
}

/// `MetaCatalogBusiness`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Business {
    pub id: String,
    pub name: Option<String>,
}

/// The Graph calls the routes make (`apis/catalogs.ts`).
pub trait MetaGraph: Send + Sync {
    /// `getCatalog`.
    fn get_catalog(
        &self,
        auth: &StoredAuth,
        catalog_id: &str,
    ) -> impl Future<Output = Result<Catalog>> + Send;
    /// `listBusinesses`.
    fn list_businesses(
        &self,
        auth: &StoredAuth,
    ) -> impl Future<Output = Result<Vec<Business>>> + Send;
    /// `createCatalog` (vertical `commerce`).
    fn create_catalog(
        &self,
        auth: &StoredAuth,
        business_id: &str,
        name: &str,
    ) -> impl Future<Output = Result<Catalog>> + Send;
}

/// The Graph API over HTTPS (`metaCatalogGraphClient`: bearer header, 30 s
/// timeout). GET retries are not ported.
pub struct HttpGraph {
    pub base_url: String,
}

impl Default for HttpGraph {
    fn default() -> Self {
        HttpGraph {
            base_url: GRAPH_API_URL.to_string(),
        }
    }
}

fn http_client() -> Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<std::result::Result<reqwest::Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| graph_error(502, e))
}

fn graph_error(status: u16, message: &str) -> BusinessError {
    BusinessError::exception(message, "metaCatalogError", status)
}

impl HttpGraph {
    async fn call(
        &self,
        auth: &StoredAuth,
        path: &str,
        query: &[(&str, &str)],
        form: Option<&[(&str, &str)]>,
    ) -> Result<Value> {
        let mut url = url::Url::parse(&format!(
            "{}/{}/{path}",
            self.base_url.trim_end_matches('/'),
            auth.version()
        ))
        .map_err(|e| graph_error(400, &e.to_string()))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let client = http_client()?;
        let request = match form {
            Some(fields) => client
                .post(url.as_str())
                .header("content-type", "application/x-www-form-urlencoded")
                .body(
                    url::form_urlencoded::Serializer::new(String::new())
                        .extend_pairs(fields)
                        .finish(),
                ),
            None => client.get(url.as_str()),
        };
        let response = request
            .header("authorization", format!("Bearer {}", auth.access_token))
            .send()
            .await
            .map_err(|e| graph_error(502, &e.to_string()))?;
        let status = response.status().as_u16();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| graph_error(502, &e.to_string()))?;
        let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        if !(200..300).contains(&status) {
            let message = chatbotx_core::utils::graph_error::format_graph_error(body.get("error"))
                .unwrap_or_else(|| format!("Meta Graph request failed: {}/{path}", auth.version()));
            return Err(graph_error(status, &message));
        }
        Ok(body)
    }
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

impl MetaGraph for HttpGraph {
    async fn get_catalog(&self, auth: &StoredAuth, catalog_id: &str) -> Result<Catalog> {
        let body = self
            .call(auth, catalog_id, &[("fields", "id,name,business")], None)
            .await?;
        Ok(Catalog {
            id: text(&body, "id").unwrap_or_default(),
            name: text(&body, "name"),
            business_id: body.get("business").and_then(|b| text(b, "id")),
        })
    }

    async fn list_businesses(&self, auth: &StoredAuth) -> Result<Vec<Business>> {
        let body = self
            .call(
                auth,
                "me/businesses",
                &[("fields", "id,name"), ("limit", "100")],
                None,
            )
            .await?;
        Ok(body
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| {
                Some(Business {
                    id: text(row, "id").filter(|id| !id.is_empty())?,
                    name: text(row, "name"),
                })
            })
            .collect())
    }

    async fn create_catalog(
        &self,
        auth: &StoredAuth,
        business_id: &str,
        name: &str,
    ) -> Result<Catalog> {
        let body = self
            .call(
                auth,
                &format!("{business_id}/owned_product_catalogs"),
                &[],
                Some(&[("name", name), ("vertical", "commerce")]),
            )
            .await?;
        let id = text(&body, "id")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| graph_error(502, "Meta did not return an ID for the new catalog"))?;
        Ok(Catalog {
            id,
            name: Some(name.to_string()),
            business_id: Some(business_id.to_string()),
        })
    }
}

/// The process `encryptUtils` (`ENCRYPTION_KEY`; the avatar signer's key).
pub fn process_encryption() -> &'static Encryption {
    crate::contact::avatar::signer()
}

// ---- integrationMetaCatalogService -----------------------------------------

fn db_error(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn already_running(message: &str, status: u16) -> BusinessError {
    BusinessError::exception(message, ALREADY_RUNNING, status)
}

/// `findByWorkspaceId`: the active (not deleted) connection.
pub fn find_by_workspace_id(
    conn: &Connection,
    workspace_id: i64,
) -> Result<Option<IntegrationMetaCatalog>> {
    Ok(repo::find_one(
        conn,
        "\"workspaceId\" = ?1 AND \"deletedAt\" IS NULL",
        &[&workspace_id],
    )?)
}

/// `findByWorkspaceIdOrFail` (404 `Meta Catalog integration not found`).
pub fn find_or_fail(conn: &Connection, workspace_id: i64) -> Result<IntegrationMetaCatalog> {
    find_by_workspace_id(conn, workspace_id)?.ok_or_else(|| BusinessError::not_found(NOT_FOUND))
}

/// One `searchProductsForSend` item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendItem {
    pub retailer_id: String,
    pub name: String,
    pub image_url: Option<String>,
}

/// `searchProductsForSend`: `None` = not connected (no connection, or no
/// bound catalog).
pub async fn search_products_for_send(
    db: &Db,
    workspace_id: i64,
    keyword: Option<String>,
    retailer_id: Option<String>,
) -> Result<Option<Vec<SendItem>>> {
    tx::read(db, move |conn| {
        let Some(connection) = find_by_workspace_id(conn, workspace_id)? else {
            return Ok(None);
        };
        let Some(catalog_id) = connection.catalog_id else {
            return Ok(None);
        };
        let scope = repo::meta_catalog_item::CatalogScope {
            integration_meta_catalog_id: connection.id,
            catalog_id,
        };
        let rows = repo::meta_catalog_item::search_with_product_info(
            conn,
            &scope,
            keyword.as_deref(),
            retailer_id.as_deref(),
            None,
        )?;
        Ok(Some(
            rows.into_iter()
                .map(|row| SendItem {
                    image_url: row
                        .images
                        .0
                        .get(0)
                        .and_then(|image| image.get("url"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    retailer_id: row.retailer_id,
                    name: row.name,
                })
                .collect(),
        ))
    })
    .await
}

/// `resolveAuth`: the decrypted credential of an active, valid connection.
pub async fn resolve_auth(
    db: &Db,
    encryption: &Encryption,
    connection_id: i64,
) -> Result<StoredAuth> {
    let row = tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "encryptedAuth", "status" FROM "IntegrationMetaCatalog" WHERE "id" = ?1 AND "deletedAt" IS NULL"#,
            [connection_id],
            |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(db_error)
    })
    .await?;
    let Some((encrypted, status)) = row else {
        return Err(BusinessError::not_found(NOT_FOUND));
    };
    let reconnect = || {
        BusinessError::exception(
            "Meta Catalog connection requires reconnection",
            "metaCatalogReconnectRequired",
            400,
        )
    };
    let Some(encrypted) = encrypted.filter(|_| status != "invalid") else {
        return Err(reconnect());
    };
    let data: EncryptedData = serde_json::from_str(&encrypted).map_err(|_| reconnect())?;
    let value: Value = encryption
        .decrypt_object(&data, None)
        .map_err(|e| BusinessError::Repo(repo::Error::msg(e.to_string())))?;
    let access_token = text(&value, "accessToken")
        .filter(|t| !t.is_empty())
        .ok_or_else(reconnect)?;
    Ok(StoredAuth {
        access_token,
        version: text(&value, "version"),
    })
}

/// `selectCatalog`: binds the catalog and queues the import state.
fn select_catalog(
    conn: &Connection,
    workspace_id: i64,
    catalog: &Catalog,
) -> Result<IntegrationMetaCatalog> {
    let row = repo::returning::<IntegrationMetaCatalog>(
        conn,
        r#"UPDATE "IntegrationMetaCatalog" SET "catalogId" = ?2, "catalogName" = ?3, "businessId" = ?4,
             "importStatus" = 'queued', "importTotalCount" = 0, "importedCount" = 0, "importFailedCount" = 0,
             "importError" = NULL, "updatedAt" = ?5
           WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL AND "importStatus" NOT IN ('queued', 'running')"#,
        &[
            &workspace_id,
            &catalog.id,
            &catalog.name,
            &catalog.business_id,
            &now_ms(),
        ],
    )?
    .into_iter()
    .next();
    if let Some(row) = row {
        return Ok(row);
    }
    match find_by_workspace_id(conn, workspace_id)? {
        Some(existing) if matches!(existing.import_status.as_str(), "queued" | "running") => {
            Err(BusinessError::exception(
                "A Meta Catalog product import is already running",
                "metaCatalogImportAlreadyRunning",
                400,
            ))
        }
        _ => Err(BusinessError::not_found(NOT_FOUND)),
    }
}

/// `bindCatalog`: points the connection at a catalog (import state kept).
/// `None` name / business = unchanged (drizzle skips `undefined`).
fn bind_catalog(
    conn: &Connection,
    workspace_id: i64,
    catalog: &Catalog,
) -> Result<IntegrationMetaCatalog> {
    repo::returning::<IntegrationMetaCatalog>(
        conn,
        r#"UPDATE "IntegrationMetaCatalog" SET "catalogId" = ?2, "catalogName" = coalesce(?3, "catalogName"),
             "businessId" = coalesce(?4, "businessId"), "updatedAt" = ?5
           WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL"#,
        &[
            &workspace_id,
            &catalog.id,
            &catalog.name,
            &catalog.business_id,
            &now_ms(),
        ],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| BusinessError::not_found(NOT_FOUND))
}

const GENERIC_IMPORT_FAILURE: &str =
    "Meta Catalog import failed. Please try again or contact support.";
const GENERIC_SYNC_FAILURE: &str = "Meta Catalog sync failed. Please try again or contact support.";

/// `failImport(id, <string>)`: an active import ends `failed` with the
/// generic text (a string is not a `ChatbotXException`).
pub async fn fail_import(db: &Db, connection_id: i64) -> Result<()> {
    let message = GENERIC_IMPORT_FAILURE;
    tx::write(db, move |tx| {
        let now = now_ms();
        tx.execute(
            r#"UPDATE "IntegrationMetaCatalog" SET "importStatus" = 'failed', "importError" = ?2, "lastImportedAt" = ?3, "updatedAt" = ?3
               WHERE "id" = ?1 AND "deletedAt" IS NULL AND "importStatus" IN ('queued', 'running')"#,
            params![connection_id, message, now],
        )
        .map_err(db_error)?;
        Ok(())
    })
    .await
}

/// `disconnect`: soft-deletes the credential; 409 while a run or a legacy
/// import is active; nothing to do without a connection.
pub async fn disconnect(db: &Db, workspace_id: i64) -> Result<()> {
    tx::write(db, move |tx| {
        let Some(existing) = find_by_workspace_id(tx, workspace_id)? else {
            return Ok(());
        };
        if has_active_run(tx, workspace_id)?
            || matches!(existing.import_status.as_str(), "queued" | "running")
        {
            return Err(already_running(
                "Wait for the active Meta Catalog sync to finish before disconnecting",
                409,
            ));
        }
        let now = now_ms();
        tx.execute(
            r#"UPDATE "IntegrationMetaCatalog" SET "deletedAt" = ?2, "encryptedAuth" = NULL, "tokenExpiresAt" = NULL,
                 "status" = 'invalid', "importStatus" = 'failed', "importError" = 'Meta Catalog was disconnected', "updatedAt" = ?2
               WHERE "id" = ?1 AND "deletedAt" IS NULL"#,
            params![existing.id, now],
        )
        .map_err(db_error)?;
        Ok(())
    })
    .await
}

fn has_active_run(conn: &Connection, workspace_id: i64) -> Result<bool> {
    conn.query_row(
        r#"SELECT EXISTS (SELECT 1 FROM "MetaCatalogSyncRun" WHERE "workspaceId" = ?1 AND "status" IN ('queued', 'running'))"#,
        [workspace_id],
        |r| r.get(0),
    )
    .map_err(db_error)
}

// ---- metaCatalogSyncRunService -----------------------------------------------

/// `metaCatalogSyncScopes`.
pub const SCOPES: &[&str] = &["all", "category", "selected"];

/// The `create` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewRun {
    pub workspace_id: i64,
    pub integration_meta_catalog_id: i64,
    pub scope: String,
    /// `None` = `push`.
    pub direction: Option<String>,
    pub catalog_id: Option<String>,
    pub category_id: Option<i64>,
    pub selected_product_ids: Vec<i64>,
}

/// `metaCatalogSyncRunService.create`: checks the category / products, then
/// inserts (one active run per workspace: `metaCatalogSyncAlreadyRunning`).
pub fn create_run(conn: &Connection, input: &NewRun) -> Result<MetaCatalogSyncRun> {
    let mut selected: Vec<i64> = Vec::new();
    if input.scope == "selected" {
        for id in &input.selected_product_ids {
            if !selected.contains(id) {
                selected.push(*id);
            }
        }
    }
    if input.scope == "category"
        && let Some(category_id) = input.category_id
    {
        let exists: bool = conn
            .query_row(
                r#"SELECT EXISTS (SELECT 1 FROM "ProductCategory" WHERE "id" = ?1 AND "workspaceId" = ?2)"#,
                params![category_id, input.workspace_id],
                |r| r.get(0),
            )
            .map_err(db_error)?;
        if !exists {
            return Err(BusinessError::not_found("Product category does not exist."));
        }
    }
    if !selected.is_empty() {
        let found = repo::product::find_by_ids(conn, input.workspace_id, &selected)?;
        if found.len() != selected.len() {
            return Err(BusinessError::not_found(
                "One or more products do not exist.",
            ));
        }
    }
    let ids: Vec<String> = selected.iter().map(i64::to_string).collect();
    let now = now_ms();
    let inserted = repo::returning::<MetaCatalogSyncRun>(
        conn,
        r#"INSERT INTO "MetaCatalogSyncRun" ("id", "createdAt", "updatedAt", "workspaceId", "integrationMetaCatalogId",
             "scope", "direction", "catalogId", "categoryId", "selectedProductIds")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"#,
        &[
            &new_id(),
            &now,
            &input.workspace_id,
            &input.integration_meta_catalog_id,
            &input.scope,
            &input.direction.as_deref().unwrap_or("push"),
            &input.catalog_id,
            &input.category_id.filter(|_| input.scope == "category"),
            &json!(ids).to_string(),
        ],
    );
    match inserted {
        Ok(rows) => rows.into_iter().next().ok_or_else(|| {
            BusinessError::Repo(repo::Error::msg("Failed to create Meta Catalog sync run"))
        }),
        Err(error) if repo::is_unique_violation_error(&error, None) => Err(already_running(
            "A Meta Catalog sync is already running",
            400,
        )),
        Err(error) => Err(error.into()),
    }
}

/// `metaCatalogSyncRunService.fail(id, <string>)`: an active run ends
/// `failed` with the generic text.
pub async fn fail_run(db: &Db, run_id: i64) -> Result<()> {
    let message = GENERIC_SYNC_FAILURE;
    tx::write(db, move |tx| {
        let now = now_ms();
        tx.execute(
            r#"UPDATE "MetaCatalogSyncRun" SET "status" = 'failed', "error" = ?2, "finishedAt" = ?3, "updatedAt" = ?3
               WHERE "id" = ?1 AND "status" IN ('queued', 'running')"#,
            params![run_id, message, now],
        )
        .map_err(db_error)?;
        Ok(())
    })
    .await
}

/// One `list` row: the run and the current names of its skipped products
/// (`skippedItems[].productName`; `None` = gone).
#[derive(Debug, Clone, PartialEq)]
pub struct RunWithNames {
    pub run: MetaCatalogSyncRun,
    pub skipped_items: Vec<Value>,
}

fn list_runs(conn: &Connection, workspace_id: i64, limit: i64) -> Result<Vec<RunWithNames>> {
    let runs: Vec<MetaCatalogSyncRun> = repo::find_many(
        conn,
        "WHERE \"workspaceId\" = ?1 ORDER BY \"createdAt\" DESC LIMIT ?2",
        &[&workspace_id, &limit],
    )?;
    let mut ids: Vec<i64> = Vec::new();
    for run in &runs {
        for item in run.skipped_items.0.as_array().into_iter().flatten() {
            if let Some(id) = product_id_of(item)
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
    }
    let products = repo::product::find_by_ids(conn, workspace_id, &ids)?;
    Ok(runs
        .into_iter()
        .map(|run| {
            let skipped_items = run
                .skipped_items
                .0
                .as_array()
                .into_iter()
                .flatten()
                .map(|item| {
                    let mut item = item.clone();
                    let name = product_id_of(&item)
                        .and_then(|id| products.iter().find(|p| p.id == id))
                        .map(|p| p.name.clone());
                    if let (Value::Object(map), Some(name)) = (&mut item, name) {
                        map.insert("productName".into(), Value::String(name));
                    }
                    item
                })
                .collect();
            RunWithNames { run, skipped_items }
        })
        .collect())
}

fn product_id_of(item: &Value) -> Option<i64> {
    match item.get("productId")? {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64(),
        _ => None,
    }
}

/// `metaCatalogSyncRunService.list` (newest 20).
pub async fn list_history(db: &Db, workspace_id: i64) -> Result<Vec<RunWithNames>> {
    tx::read(db, move |conn| list_runs(conn, workspace_id, 20)).await
}

// ---- metaCatalogOperationService ---------------------------------------------

fn lock_or_fail(conn: &Connection, workspace_id: i64) -> Result<IntegrationMetaCatalog> {
    // ponytail: `SELECT ... FOR UPDATE` is the write transaction itself
    // (SQLite `BEGIN IMMEDIATE` serializes writers).
    let connection = find_or_fail(conn, workspace_id)?;
    if matches!(connection.import_status.as_str(), "queued" | "running") {
        return Err(already_running(
            "A Meta Catalog sync is already running",
            400,
        ));
    }
    Ok(connection)
}

/// `startImport`: reserves an import run, then queues the connection state
/// (one transaction).
pub async fn start_import(
    db: &Db,
    workspace_id: i64,
    catalog: Catalog,
) -> Result<(IntegrationMetaCatalog, MetaCatalogSyncRun)> {
    tx::write(db, move |tx| {
        let connection = lock_or_fail(tx, workspace_id)?;
        let run = create_run(
            tx,
            &NewRun {
                workspace_id,
                integration_meta_catalog_id: connection.id,
                scope: "all".into(),
                direction: Some("import".into()),
                catalog_id: Some(catalog.id.clone()),
                ..NewRun::default()
            },
        )?;
        let selected = select_catalog(tx, workspace_id, &catalog)?;
        Ok((selected, run))
    })
    .await
}

/// The `startPush` input (`MetaCatalogSyncInput` without `catalogId`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PushScope {
    pub scope: String,
    pub category_id: Option<i64>,
    pub selected_product_ids: Vec<i64>,
}

/// `startPush`: reserves a push run, then rebinds the catalog when it
/// differs (one transaction).
pub async fn start_push(
    db: &Db,
    workspace_id: i64,
    catalog: Catalog,
    scope: PushScope,
) -> Result<MetaCatalogSyncRun> {
    tx::write(db, move |tx| {
        let connection = lock_or_fail(tx, workspace_id)?;
        let run = create_run(
            tx,
            &NewRun {
                workspace_id,
                integration_meta_catalog_id: connection.id,
                scope: scope.scope,
                direction: None,
                catalog_id: Some(catalog.id.clone()),
                category_id: scope.category_id,
                selected_product_ids: scope.selected_product_ids,
            },
        )?;
        if connection.catalog_id.as_deref() != Some(catalog.id.as_str()) {
            bind_catalog(tx, workspace_id, &catalog)?;
        }
        Ok(run)
    })
    .await
}

// ---- meta-catalog-operations.ts ----------------------------------------------

/// The services the operations use.
pub struct Deps<'a, G: MetaGraph> {
    pub db: &'a Db,
    pub graph: &'a G,
    pub encryption: &'a Encryption,
    pub jobs: Option<&'a dyn Enqueuer>,
}

/// `translateSyncCollision`: `metaCatalogSyncAlreadyRunning` becomes a 409
/// with the caller's sentence.
fn translate_collision(error: BusinessError, reasons: &Reasons) -> BusinessError {
    if error.code() == Some(ALREADY_RUNNING) {
        return already_running(reasons.already_running, 409);
    }
    error
}

async fn queue(
    jobs: Option<&dyn Enqueuer>,
    name: &str,
    data: Value,
    job_id: String,
) -> std::result::Result<(), String> {
    let Some(jobs) = jobs else {
        return Err("the job queue is off".into());
    };
    jobs.add(
        QueueName::Default,
        name,
        json!({ "type": name, "data": data }),
        JobOptions {
            job_id: Some(job_id),
            ..JobOptions::default()
        },
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// `getMetaCatalogState`: the connection (no credential) and the history.
pub async fn get_state(
    db: &Db,
    workspace_id: i64,
) -> Result<(Option<IntegrationMetaCatalog>, Vec<RunWithNames>)> {
    tx::read(db, move |conn| {
        Ok((
            find_by_workspace_id(conn, workspace_id)?,
            list_runs(conn, workspace_id, 20)?,
        ))
    })
    .await
}

async fn connection_auth<G: MetaGraph>(
    deps: &Deps<'_, G>,
    workspace_id: i64,
) -> Result<(IntegrationMetaCatalog, StoredAuth)> {
    let connection = tx::read(deps.db, move |conn| find_or_fail(conn, workspace_id)).await?;
    let auth = resolve_auth(deps.db, deps.encryption, connection.id).await?;
    Ok((connection, auth))
}

/// `selectMetaCatalog`: binds an existing catalog and queues its import.
/// A queue failure fails the run and the import.
pub async fn select<G: MetaGraph>(
    deps: &Deps<'_, G>,
    workspace_id: i64,
    catalog_id: &str,
    reasons: &Reasons,
) -> Result<IntegrationMetaCatalog> {
    let (_, auth) = connection_auth(deps, workspace_id).await?;
    let catalog = deps.graph.get_catalog(&auth, catalog_id).await?;
    let (selected, run) = start_import(deps.db, workspace_id, catalog)
        .await
        .map_err(|e| translate_collision(e, reasons))?;
    let data = json!({
        "workspaceId": workspace_id.to_string(),
        "integrationMetaCatalogId": selected.id.to_string(),
        "runId": run.id.to_string(),
    });
    let job_id = format!("mc-import-{}-{}", selected.id, selected.updated_at);
    if let Err(error) = queue(deps.jobs, IMPORT_JOB, data, job_id).await {
        fail_import(deps.db, selected.id).await?;
        fail_run(deps.db, run.id).await?;
        return Err(BusinessError::Repo(repo::Error::msg(error)));
    }
    Ok(selected)
}

/// `listMetaCatalogBusinesses`.
pub async fn list_businesses<G: MetaGraph>(
    deps: &Deps<'_, G>,
    workspace_id: i64,
) -> Result<Vec<Business>> {
    let (_, auth) = connection_auth(deps, workspace_id).await?;
    deps.graph.list_businesses(&auth).await
}

/// `createAndBindMetaCatalog`: creates the catalog on Meta and binds it.
pub async fn create_and_bind<G: MetaGraph>(
    deps: &Deps<'_, G>,
    workspace_id: i64,
    business_id: &str,
    name: &str,
) -> Result<IntegrationMetaCatalog> {
    let (_, auth) = connection_auth(deps, workspace_id).await?;
    let catalog = deps.graph.create_catalog(&auth, business_id, name).await?;
    tx::write(deps.db, move |tx| bind_catalog(tx, workspace_id, &catalog)).await
}

/// `syncProductsToMetaCatalog`: starts a push and queues its submit job.
/// `catalog_id` `None` = the bound catalog (422 `catalogId` when none); a new
/// destination is verified with Meta.
pub async fn sync_products<G: MetaGraph>(
    deps: &Deps<'_, G>,
    workspace_id: i64,
    catalog_id: Option<String>,
    scope: PushScope,
    reasons: &Reasons,
) -> Result<MetaCatalogSyncRun> {
    let connection = tx::read(deps.db, move |conn| find_or_fail(conn, workspace_id)).await?;
    let Some(catalog_id) = catalog_id.or_else(|| connection.catalog_id.clone()) else {
        return Err(BusinessError::validation(
            "catalogId",
            "No catalog is bound yet: send catalogId",
        ));
    };
    let catalog = if connection.catalog_id.as_deref() == Some(catalog_id.as_str()) {
        Catalog {
            id: catalog_id,
            name: None,
            business_id: None,
        }
    } else {
        let auth = resolve_auth(deps.db, deps.encryption, connection.id).await?;
        let found = deps.graph.get_catalog(&auth, &catalog_id).await?;
        Catalog {
            id: catalog_id,
            ..found
        }
    };
    let run = start_push(deps.db, workspace_id, catalog, scope)
        .await
        .map_err(|e| translate_collision(e, reasons))?;
    let data = json!({ "workspaceId": workspace_id.to_string(), "runId": run.id.to_string() });
    if let Err(error) = queue(deps.jobs, SUBMIT_JOB, data, format!("mc-submit-{}", run.id)).await {
        fail_run(deps.db, run.id).await?;
        return Err(BusinessError::Repo(repo::Error::msg(error)));
    }
    Ok(run)
}
