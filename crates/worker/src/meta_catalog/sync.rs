//! `handlers/meta-catalog/{submit,check}.ts` and `apis/items-batch.ts`.

use std::collections::{HashMap, HashSet};

use chatbotx_business::meta_catalog::{find_or_fail, process_encryption, resolve_auth};
use chatbotx_business::tx;
use chatbotx_core::utils::id::create_id;
use chatbotx_db::Db;
use chatbotx_db::repo::meta_catalog_item::{self, CatalogLinkItem, CatalogScope};
use chatbotx_db::repo::models::MetaCatalogSyncRun;
use chatbotx_db::repo::product::{CatalogSyncProduct, list_for_catalog_sync};
use chatbotx_db::repo::{self, now_ms};
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use chatbotx_jobs::{JobError, JobOptions, QueueName};
use serde_json::{Value, json};

use super::import::{job_err, to_db};
use super::items::{
    CATALOG_BATCH_SIZE, CatalogProduct, CatalogSettings, MetaItem, concurrency_for_usage,
    fingerprint_meta_item, parse_buc_header, resolve_retailer_ids, to_meta_item,
};
use super::{
    McError, McResult, StoredAuth, capped, fail_run, first_run, graph, log_failure, mark_invalid,
    outcome, sanitize_item_errors, workspace_name,
};
use crate::{Deps, db_fail, id_of};

const INTERRUPTED_ITEM_REASON: &str = "Submission was interrupted; sync this item again";
const RATE_LIMIT_REASON: &str = "Meta rate limit reached; retry this item later";
const SUBMISSION_STALE_MS: i64 = 2 * 60 * 1000;
const MAX_POLL_ATTEMPTS: i64 = 12;
const CHECK_JOB: &str = "checkMetaCatalogSync";

// ---- apis/items-batch.ts --------------------------------------------------------

/// `submitItemsBatch`: `(handles, usage header)`.
async fn submit_items_batch(
    auth: &StoredAuth,
    catalog_id: &str,
    requests: &[Value],
) -> McResult<(Vec<String>, Option<String>)> {
    let form = [
        ("item_type", "PRODUCT_ITEM".to_owned()),
        ("allow_upsert", "true".to_owned()),
        ("requests", Value::Array(requests.to_vec()).to_string()),
    ];
    let (body, usage) = graph(auth, &format!("{catalog_id}/items_batch"), &[], Some(&form)).await?;
    let handles: Vec<String> = match body.get("handles").and_then(Value::as_array) {
        Some(handles) => handles
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        None => body
            .get("handle")
            .and_then(Value::as_str)
            .filter(|h| !h.is_empty())
            .map(str::to_owned)
            .into_iter()
            .collect(),
    };
    if handles.is_empty() {
        return Err(McError::internal(
            "Meta did not return a catalog batch handle",
        ));
    }
    Ok((handles, usage))
}

/// `MetaCatalogBatchItemResult`.
#[derive(Debug, Clone, PartialEq)]
pub struct BatchItemResult {
    pub retailer_id: String,
    pub error: Option<String>,
}

/// `parseBatchResults`.
pub fn parse_batch_results(statuses: &[Value], retailer_ids: &[String]) -> Vec<BatchItemResult> {
    let requested: HashSet<&str> = retailer_ids.iter().map(String::as_str).collect();
    let invalid: Vec<String> = statuses
        .iter()
        .flat_map(|s| {
            s.get("ids_of_invalid_requests")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .map(|id| match id {
            Value::String(s) => s,
            other => other.to_string(),
        })
        .collect();
    let recognized: HashSet<&str> = invalid
        .iter()
        .map(String::as_str)
        .filter(|id| requested.contains(id))
        .collect();
    let reported: usize = statuses
        .iter()
        .map(|s| {
            s.get("errors_total_count")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .or_else(|| {
                    s.get("ids_of_invalid_requests")
                        .and_then(Value::as_array)
                        .map(Vec::len)
                })
                .or_else(|| s.get("errors").and_then(Value::as_array).map(Vec::len))
                .unwrap_or(0)
        })
        .sum();
    let unresolved = invalid.len() > recognized.len() || reported > recognized.len();
    let message = statuses
        .iter()
        .flat_map(|s| {
            s.get("errors")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .find_map(|e| match e {
            Value::String(s) => Some(s).filter(|s| !s.is_empty()),
            other => other
                .get("message")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        });
    retailer_ids
        .iter()
        .map(|id| {
            let error = if unresolved {
                Some(message.clone().unwrap_or_else(|| {
                    "Meta reported catalog item errors without recognizable retailer IDs".into()
                }))
            } else if recognized.contains(id.as_str()) {
                Some(
                    message
                        .clone()
                        .unwrap_or_else(|| "Meta rejected this catalog item".into()),
                )
            } else {
                None
            };
            BatchItemResult {
                retailer_id: id.clone(),
                error,
            }
        })
        .collect()
}

/// `checkItemsBatch`: `None` while Meta is still working.
async fn check_items_batch(
    auth: &StoredAuth,
    catalog_id: &str,
    handle: &str,
    retailer_ids: &[String],
) -> McResult<Option<Vec<BatchItemResult>>> {
    let (body, _) = graph(
        auth,
        &format!("{catalog_id}/check_batch_request_status"),
        &[
            ("handle", handle),
            (
                "fields",
                "status,errors,errors_total_count,ids_of_invalid_requests,handle",
            ),
        ],
        None,
    )
    .await?;
    let statuses = body
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let completed = !statuses.is_empty()
        && statuses.iter().all(|s| {
            let status = s
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_lowercase();
            status == "completed" || status == "finished"
        });
    Ok(completed.then(|| parse_batch_results(&statuses, retailer_ids)))
}

// ---- shared ---------------------------------------------------------------------

fn catalog_product(row: &CatalogSyncProduct) -> CatalogProduct {
    let p = &row.product;
    let strings = |v: &Value, key: Option<&str>| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|item| match key {
                Some(key) => item.get(key).and_then(Value::as_str),
                None => item.as_str(),
            })
            .map(str::to_owned)
            .collect()
    };
    CatalogProduct {
        id: p.id.to_string(),
        sku: p.sku.clone(),
        name: p.name.clone(),
        short_description: p.short_description.clone(),
        long_description: p.long_description.clone(),
        price: p.price,
        discount: p.discount,
        currency: Some(p.currency.clone()),
        product_url: p.product_url.clone(),
        inventory_policy: p.inventory_policy.clone(),
        inventory_quantity: p.inventory_quantity,
        allow_out_of_stock_purchase: p.allow_out_of_stock_purchase,
        status: p.status.clone(),
        image_urls: strings(&p.images.0, Some("url")),
        tags: strings(&p.tags.0, None),
        vendor: row.vendor.clone(),
        category: row.category.as_ref().map(|c| c.name.clone()),
        subcategory: row.subcategory.as_ref().map(|c| c.name.clone()),
        has_variants: !row.variants.is_empty(),
    }
}

/// A `MetaCatalogBatchHandle`'s retailer ids (`items` or legacy `retailerIds`).
fn batch_retailer_ids(batch: &Value) -> Vec<String> {
    match batch.get("items").and_then(Value::as_array) {
        Some(items) => items
            .iter()
            .filter_map(|i| i["retailerId"].as_str())
            .map(str::to_owned)
            .collect(),
        None => batch["retailerIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    }
}

fn find_run(
    conn: &Connection,
    run_id: i64,
    workspace_id: i64,
) -> McResult<Option<MetaCatalogSyncRun>> {
    Ok(repo::find_one(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&run_id, &workspace_id],
    )?)
}

/// `metaCatalogSyncRunService.complete`: the outcome, or `None` when the run
/// is no longer running.
async fn complete(
    db: &Db,
    run_id: i64,
    scope: CatalogScope,
    succeeded: Vec<CatalogLinkItem>,
    errors: Vec<Value>,
) -> McResult<Option<&'static str>> {
    Ok(tx::write(db, move |tx| {
        let current = tx
            .query_row(
                r#"SELECT "skippedCount", "itemErrors", "failedCount" FROM "MetaCatalogSyncRun" WHERE "id" = ?1 AND "status" = 'running'"#,
                [run_id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)),
            )
            .optional()
            .map_err(|e| chatbotx_business::BusinessError::Repo(e.into()))?;
        let Some((skipped, item_errors, failed)) = current else {
            return Ok(None);
        };
        meta_catalog_item::mark_succeeded(tx, &scope, &succeeded)?;
        let mut all: Vec<Value> = serde_json::from_str::<Vec<Value>>(&item_errors).unwrap_or_default();
        all.extend(errors.iter().cloned());
        let all = sanitize_item_errors(&all);
        let failed = failed + errors.len() as i64;
        let succeeded_count = succeeded.len() as i64;
        let status = outcome(succeeded_count, failed, skipped);
        let now = now_ms();
        tx.execute(
            r#"UPDATE "MetaCatalogSyncRun" SET "status" = ?2, "succeededCount" = ?3, "failedCount" = ?4, "itemErrors" = ?5,
                 "finishedAt" = ?6, "updatedAt" = ?6 WHERE "id" = ?1"#,
            params![run_id, status, succeeded_count, failed, capped(&all), now],
        )
        .map_err(|e| chatbotx_business::BusinessError::Repo(e.into()))?;
        Ok(Some(status))
    })
    .await?)
}

async fn enqueue_check(
    deps: &Deps,
    workspace_id: i64,
    run_id: i64,
    attempt: i64,
    delay_ms: u64,
) -> McResult<()> {
    deps.jobs
        .add(
            QueueName::Default,
            CHECK_JOB,
            json!({
                "type": CHECK_JOB,
                "data": { "workspaceId": workspace_id.to_string(), "runId": run_id.to_string(), "attempt": attempt },
            }),
            JobOptions {
                job_id: Some(format!("mc-check-{run_id}-{attempt}")),
                delay_ms: Some(delay_ms),
                ..JobOptions::default()
            },
        )
        .await
        .map_err(|e| McError::internal(e.to_string()))?;
    Ok(())
}

fn settings(
    connection: &chatbotx_db::repo::models::IntegrationMetaCatalog,
    workspace_name: String,
) -> CatalogSettings {
    CatalogSettings {
        currency: connection.currency.clone(),
        store_url: connection.store_url.clone(),
        workspace_name,
    }
}

// ---- submit.ts ------------------------------------------------------------------

/// `metaCatalogSyncRunService.recordSubmission`: false when the lease is gone.
struct Checkpoint<'a> {
    db: &'a Db,
    run_id: i64,
    lease: String,
    total_count: i64,
    skipped: Vec<Value>,
}

impl Checkpoint<'_> {
    async fn save(
        &self,
        handles: &[Value],
        failed_count: usize,
        item_errors: Vec<Value>,
    ) -> McResult<bool> {
        let (run_id, lease, total, skipped) = (
            self.run_id,
            self.lease.clone(),
            self.total_count,
            self.skipped.clone(),
        );
        let handles = handles.to_vec();
        let item_errors = sanitize_item_errors(&item_errors);
        Ok(self
            .db
            .write(move |tx| {
                let current: Option<String> = tx
                    .query_row(
                        r#"SELECT "handles" FROM "MetaCatalogSyncRun" WHERE "id" = ?1 AND "status" = 'running' AND "submissionLeaseId" = ?2"#,
                        params![run_id, lease],
                        |r| r.get(0),
                    )
                    .optional()?;
                let Some(current) = current else {
                    return Ok(false);
                };
                // `new Map([...current, ...input].map(h => [h.handle, h]))`.
                let mut merged: Vec<Value> = Vec::new();
                for handle in serde_json::from_str::<Vec<Value>>(&current).unwrap_or_default().into_iter().chain(handles) {
                    match merged.iter_mut().find(|h| h["handle"] == handle["handle"]) {
                        Some(existing) => *existing = handle,
                        None => merged.push(handle),
                    }
                }
                tx.execute(
                    r#"UPDATE "MetaCatalogSyncRun" SET "totalCount" = ?2, "handles" = ?3, "skippedItems" = ?4, "skippedCount" = ?5,
                         "itemErrors" = ?6, "failedCount" = ?7, "updatedAt" = ?8 WHERE "id" = ?1"#,
                    params![
                        run_id,
                        total,
                        Value::Array(merged).to_string(),
                        capped(&skipped),
                        skipped.len() as i64,
                        capped(&item_errors),
                        failed_count as i64,
                        now_ms()
                    ],
                )?;
                Ok(true)
            })
            .await?)
    }
}

fn item_error(retailer_id: &str, reason: &str) -> Value {
    json!({ "retailerId": retailer_id, "reason": reason })
}

/// How a submission stops early.
enum Stop {
    LeaseLost,
    Failed { error: McError, indeterminate: bool },
}

impl From<McError> for Stop {
    fn from(error: McError) -> Stop {
        Stop::Failed {
            error,
            indeterminate: false,
        }
    }
}

/// `submitMetaCatalogSync`.
pub async fn submit(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let db = &deps.db;
    let workspace_id =
        id_of(&data["workspaceId"]).ok_or_else(|| crate::fail("workspaceId is required"))?;
    let run_id = id_of(&data["runId"]).ok_or_else(|| crate::fail("runId is required"))?;
    let recovery = data["recovery"].as_bool().unwrap_or(false);
    let lease = create_id();
    let claim_lease = lease.clone();
    let run = db
        .write(move |tx| {
            if recovery {
                first_run(repo::returning(
                    tx,
                    r#"UPDATE "MetaCatalogSyncRun" SET "submissionLeaseId" = ?3, "updatedAt" = ?4
                       WHERE "id" = ?1 AND "workspaceId" = ?2 AND "status" = 'running' AND "submissionLeaseId" IS NOT NULL AND "updatedAt" < ?5"#,
                    &[&run_id, &workspace_id, &claim_lease, &now_ms(), &(now_ms() - SUBMISSION_STALE_MS)],
                ))
            } else {
                super::claim_run(tx, run_id, workspace_id, Some(&claim_lease))
            }
            .map_err(to_db)
        })
        .await
        .map_err(db_fail)?;
    let Some(run) = run else {
        return Ok(());
    };
    let mut handles: Vec<Value> = run.handles.0.as_array().cloned().unwrap_or_default();
    let result: Result<(), Stop> = async {
        let connection = tx::read(db, move |conn| find_or_fail(conn, workspace_id)).await.map_err(McError::from)?;
        let name = db.read(move |conn| Ok(workspace_name(conn, workspace_id)?)).await.map_err(McError::from)?;
        if run.integration_meta_catalog_id != connection.id {
            return Err(McError::internal("Meta Catalog sync run does not match its connection").into());
        }
        let (Some(catalog_id), Some(name)) = (run.catalog_id.clone().or(connection.catalog_id.clone()), name) else {
            return Err(McError::internal("Meta Catalog settings are incomplete").into());
        };
        let scope = CatalogScope { integration_meta_catalog_id: connection.id, catalog_id: catalog_id.clone() };
        let category_id = (run.scope == "category").then_some(run.category_id).flatten();
        let selected: Option<Vec<i64>> = (run.scope == "selected").then(|| {
            run.selected_product_ids.0.as_array().into_iter().flatten().filter_map(id_of).collect()
        });
        let lookup_scope = scope.clone();
        let (products, links, sku_claims) = tx::read(db, move |conn| {
            let products = list_for_catalog_sync(conn, workspace_id, category_id, selected.as_deref())?;
            let ids: Vec<i64> = products.iter().map(|p| p.product.id).collect();
            let links = meta_catalog_item::find_by_product_ids(conn, &lookup_scope, &ids)?;
            let skus: Vec<String> = products
                .iter()
                .filter_map(|p| p.product.sku.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned))
                .collect();
            let refs: Vec<&str> = skus.iter().map(String::as_str).collect();
            let claims = meta_catalog_item::find_by_retailer_ids(conn, &lookup_scope, &refs)?;
            Ok((products, links, claims))
        })
        .await
        .map_err(McError::from)?;
        let products: Vec<CatalogProduct> = products.iter().map(catalog_product).collect();
        let retailer_ids = resolve_retailer_ids(
            &products.iter().map(|p| (p.id.clone(), p.sku.clone())).collect::<Vec<_>>(),
            &links.iter().map(|l| (l.product_id.to_string(), l.retailer_id.clone())).collect(),
            &sku_claims.iter().map(|l| (l.retailer_id.clone(), l.product_id.to_string())).collect(),
        );
        let settings = settings(&connection, name);
        let mapped: Vec<MetaItem> = products
            .iter()
            .map(|p| to_meta_item(p, &settings, retailer_ids.get(&p.id).map(String::as_str)))
            .collect();
        let skipped: Vec<Value> = mapped
            .iter()
            .filter_map(|m| match m {
                MetaItem::Skipped { product_id, reason } => Some(json!({ "productId": product_id, "reason": reason })),
                MetaItem::Mapped { .. } => None,
            })
            .collect();
        let settled: HashSet<String> = handles.iter().flat_map(batch_retailer_ids).collect();
        let accepted: Vec<(&String, &String, &Value)> = mapped
            .iter()
            .filter_map(|m| match m {
                MetaItem::Mapped { product_id, retailer_id, data } if !settled.contains(retailer_id) => {
                    Some((product_id, retailer_id, data))
                }
                _ => None,
            })
            .collect();
        let linked: HashSet<&str> = links.iter().map(|l| l.retailer_id.as_str()).collect();
        let mut item_errors: Vec<Value> = run
            .item_errors
            .0
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["reason"] != INTERRUPTED_ITEM_REASON)
            .cloned()
            .collect();
        let batches: Vec<&[(&String, &String, &Value)]> = accepted.chunks(CATALOG_BATCH_SIZE).collect();
        let checkpoint = Checkpoint {
            db,
            run_id: run.id,
            lease: lease.clone(),
            total_count: run.total_count.max(products.len() as i64),
            skipped,
        };
        let save = async |handles: &[Value], item_errors: &[Value], next: usize| -> Result<(), Stop> {
            let pending = batches[next.min(batches.len())..]
                .iter()
                .flat_map(|b| b.iter().map(|(_, r, _)| item_error(r, INTERRUPTED_ITEM_REASON)));
            let all: Vec<Value> = item_errors.iter().cloned().chain(pending).collect();
            if checkpoint.save(handles, item_errors.len(), all).await? {
                Ok(())
            } else {
                Err(Stop::LeaseLost)
            }
        };
        save(&handles, &item_errors, 0).await?;
        let auth = resolve_auth(db, process_encryption(), connection.id).await.map_err(McError::from)?;
        for (index, batch) in batches.iter().enumerate() {
            let requests: Vec<Value> = batch
                .iter()
                .map(|(_, retailer_id, data)| {
                    let method = if recovery || linked.contains(retailer_id.as_str()) { "UPDATE" } else { "CREATE" };
                    let mut data = (*data).clone();
                    data["id"] = json!(retailer_id);
                    json!({ "method": method, "data": data })
                })
                .collect();
            match submit_items_batch(&auth, &catalog_id, &requests).await {
                Err(error) if error.is_invalid_token() || !error.is_definite_rejection() => {
                    return Err(Stop::Failed { error, indeterminate: true });
                }
                Err(error) => {
                    item_errors.extend(batch.iter().map(|(_, r, _)| item_error(r, error.message())));
                    save(&handles, &item_errors, index + 1).await?;
                }
                Ok((new_handles, usage)) => {
                    let items: Vec<Value> = batch
                        .iter()
                        .map(|(p, r, _)| json!({ "productId": p, "retailerId": r }))
                        .collect();
                    handles.extend(new_handles.into_iter().map(|h| json!({ "handle": h, "items": items })));
                    save(&handles, &item_errors, index + 1).await?;
                    if concurrency_for_usage(parse_buc_header(usage.as_deref()).as_ref()) == 0 {
                        item_errors.extend(
                            batches[index + 1..]
                                .iter()
                                .flat_map(|b| b.iter().map(|(_, r, _)| item_error(r, RATE_LIMIT_REASON))),
                        );
                        break;
                    }
                }
            }
        }
        save(&handles, &item_errors, batches.len()).await?;
        let finish_lease = lease.clone();
        let finished = db
            .write(move |tx| {
                Ok(tx
                    .query_row(
                        r#"UPDATE "MetaCatalogSyncRun" SET "submissionLeaseId" = NULL, "updatedAt" = ?3
                           WHERE "id" = ?1 AND "status" = 'running' AND "submissionLeaseId" = ?2 RETURNING "id""#,
                        params![run_id, finish_lease, now_ms()],
                        |r| r.get::<_, i64>(0),
                    )
                    .optional()?
                    .is_some())
            })
            .await
            .map_err(McError::from)?;
        if !finished {
            return Ok(());
        }
        if handles.is_empty() {
            // ponytail: the `catalog_synced` audit needs a user; a worker job
            // has none, so the original drops it too.
            complete(db, run.id, scope, Vec::new(), Vec::new()).await?;
            return Ok(());
        }
        enqueue_check(deps, workspace_id, run.id, 0, 5000).await?;
        Ok(())
    }
    .await;
    let (error, indeterminate) = match result {
        Ok(()) | Err(Stop::LeaseLost) => return Ok(()),
        Err(Stop::Failed {
            error,
            indeterminate,
        }) => (error, indeterminate),
    };
    log_failure(
        &error,
        "Meta Catalog submission failed",
        json!({ "runId": run_id }),
    );
    if error.is_invalid_token() {
        mark_invalid(db, workspace_id).await.map_err(job_err)?;
    }
    if !handles.is_empty() || indeterminate {
        return Err(job_err(error));
    }
    fail_run(db, run_id, &error).await.map_err(job_err)?;
    // ponytail: `logProviderError` is not ported.
    Ok(())
}

// ---- check.ts -------------------------------------------------------------------

/// `pollDelay`.
fn poll_delay(attempt: i64) -> u64 {
    (5000u64 << attempt.clamp(0, 10)).min(60_000)
}

/// `checkMetaCatalogSync`.
pub async fn check(deps: &Deps, data: &Value) -> Result<(), JobError> {
    let db = &deps.db;
    let workspace_id =
        id_of(&data["workspaceId"]).ok_or_else(|| crate::fail("workspaceId is required"))?;
    let run_id = id_of(&data["runId"]).ok_or_else(|| crate::fail("runId is required"))?;
    let attempt = data["attempt"].as_i64().unwrap_or(0);
    let run = db
        .read(move |conn| find_run(conn, run_id, workspace_id).map_err(to_db))
        .await
        .map_err(db_fail)?
        .ok_or_else(|| crate::fail("Meta Catalog sync run not found"))?;
    if run.status != "running" {
        return Ok(());
    }
    // `Err(Ok(e))` = a dispatch failure (rethrown as is).
    let result: Result<(), Result<McError, McError>> = async {
        let connection = tx::read(db, move |conn| find_or_fail(conn, workspace_id)).await.map_err(|e| Err(e.into()))?;
        if run.integration_meta_catalog_id != connection.id {
            return Err(Err(McError::internal("Meta Catalog sync run does not match its connection")));
        }
        let Some(catalog_id) = run.catalog_id.clone().or(connection.catalog_id.clone()) else {
            return Err(Err(McError::internal("Meta Catalog is not selected")));
        };
        let auth = resolve_auth(db, process_encryption(), connection.id).await.map_err(|e| Err(e.into()))?;
        let batches: Vec<Value> = run.handles.0.as_array().cloned().unwrap_or_default();
        let mut checks = Vec::new();
        for batch in &batches {
            let handle = batch["handle"].as_str().unwrap_or("");
            checks.push(check_items_batch(&auth, &catalog_id, handle, &batch_retailer_ids(batch)).await.map_err(Err)?);
        }
        if checks.iter().any(Option::is_none) {
            if attempt >= MAX_POLL_ATTEMPTS {
                return Err(Err(McError::internal("Meta Catalog batch status timed out")));
            }
            let next = attempt + 1;
            db.write(move |tx| {
                tx.execute(
                    r#"UPDATE "MetaCatalogSyncRun" SET "pollAttempt" = "pollAttempt" + 1, "updatedAt" = ?2 WHERE "id" = ?1 AND "status" = 'running'"#,
                    params![run_id, now_ms()],
                )?;
                Ok(())
            })
            .await
            .map_err(|e| Err(e.into()))?;
            return enqueue_check(deps, workspace_id, run_id, next, poll_delay(next)).await.map_err(Ok);
        }
        let results: Vec<BatchItemResult> = checks.into_iter().flatten().flatten().collect();
        let scope = CatalogScope { integration_meta_catalog_id: connection.id, catalog_id: catalog_id.clone() };
        let mut product_of: HashMap<String, String> = batches
            .iter()
            .flat_map(|b| b["items"].as_array().cloned().unwrap_or_default())
            .filter_map(|i| Some((i["retailerId"].as_str()?.to_owned(), id_of(&i["productId"])?.to_string())))
            .collect();
        let legacy: Vec<String> = batches
            .iter()
            .filter(|b| b.get("items").is_none())
            .flat_map(batch_retailer_ids)
            .collect();
        if !legacy.is_empty() {
            let (lookup, ids) = (scope.clone(), legacy.clone());
            let links = tx::read(db, move |conn| {
                let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
                Ok(meta_catalog_item::find_by_retailer_ids(conn, &lookup, &refs)?)
            })
            .await
            .map_err(|e| Err(e.into()))?;
            for retailer_id in &legacy {
                let product = links
                    .iter()
                    .find(|l| &l.retailer_id == retailer_id)
                    .map_or_else(|| retailer_id.clone(), |l| l.product_id.to_string());
                product_of.insert(retailer_id.clone(), product);
            }
        }
        let mut requested: Vec<String> = Vec::new();
        for id in batches.iter().flat_map(batch_retailer_ids) {
            if !requested.contains(&id) {
                requested.push(id);
            }
        }
        let by_retailer: HashMap<&str, &BatchItemResult> =
            results.iter().filter(|r| !r.retailer_id.is_empty()).map(|r| (r.retailer_id.as_str(), r)).collect();
        let succeeded_ids: Vec<&String> =
            requested.iter().filter(|id| by_retailer.get(id.as_str()).is_some_and(|r| r.error.is_none())).collect();
        let mut errors: Vec<Value> = requested
            .iter()
            .filter(|id| !succeeded_ids.contains(id))
            .map(|id| {
                let reason = by_retailer
                    .get(id.as_str())
                    .and_then(|r| r.error.clone())
                    .unwrap_or_else(|| "Meta did not return a result for this catalog item".into());
                item_error(id, &reason)
            })
            .collect();
        let product_id = |r: &String| product_of.get(r).cloned().unwrap_or_else(|| r.clone());
        let retailer_by_product: HashMap<String, String> =
            succeeded_ids.iter().map(|r| (product_id(r), (*r).clone())).collect();
        let ids: Vec<i64> = retailer_by_product.keys().filter_map(|id| id.parse().ok()).collect();
        let (products, name) = tx::read(db, move |conn| {
            Ok((
                list_for_catalog_sync(conn, workspace_id, None, Some(&ids))?,
                workspace_name(conn, workspace_id).map_err(|e| chatbotx_business::BusinessError::Repo(e.into()))?,
            ))
        })
        .await
        .map_err(|e| Err(e.into()))?;
        let Some(name) = name else {
            return Err(Err(McError::internal("Workspace not found")));
        };
        let products: Vec<CatalogProduct> = products.iter().map(catalog_product).collect();
        for retailer_id in &succeeded_ids {
            if !products.iter().any(|p| p.id == product_id(retailer_id)) {
                errors.push(item_error(
                    retailer_id,
                    "The local product for this legacy Meta sync could not be resolved; sync this item again",
                ));
            }
        }
        let settings = settings(&connection, name);
        let succeeded: Vec<CatalogLinkItem> = products
            .iter()
            .filter_map(|p| match to_meta_item(p, &settings, retailer_by_product.get(&p.id).map(String::as_str)) {
                MetaItem::Mapped { product_id, retailer_id, data } => Some(CatalogLinkItem {
                    product_id: product_id.parse().ok()?,
                    retailer_id,
                    fingerprint: Some(fingerprint_meta_item(&data)),
                }),
                MetaItem::Skipped { .. } => None,
            })
            .collect();
        // ponytail: the `catalog_synced` audit needs a user (dropped, as in the original).
        complete(db, run_id, scope, succeeded, errors).await.map_err(Err)?;
        Ok(())
    }
    .await;
    match result {
        Ok(()) => Ok(()),
        Err(Ok(dispatch)) => Err(job_err(dispatch)),
        Err(Err(error)) => {
            log_failure(
                &error,
                "Meta Catalog status check failed",
                json!({ "runId": run_id }),
            );
            if error.is_invalid_token() {
                mark_invalid(db, workspace_id).await.map_err(job_err)?;
            }
            fail_run(db, run_id, &error).await.map_err(job_err)?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn marks_recognized_invalid_requests_as_failed() {
        let statuses = [
            json!({ "status": "finished", "errors": [{ "message": "Bad price" }], "ids_of_invalid_requests": ["b"] }),
        ];
        let results = parse_batch_results(&statuses, &ids(&["a", "b"]));
        assert_eq!(results[0].error, None);
        assert_eq!(results[1].error.as_deref(), Some("Bad price"));
    }

    #[test]
    fn fails_every_item_when_errors_cannot_be_matched() {
        let statuses = [
            json!({ "status": "finished", "errors_total_count": 2, "ids_of_invalid_requests": ["zzz"] }),
        ];
        let results = parse_batch_results(&statuses, &ids(&["a"]));
        assert_eq!(
            results[0].error.as_deref(),
            Some("Meta reported catalog item errors without recognizable retailer IDs")
        );
    }

    #[test]
    fn poll_delay_doubles_up_to_a_minute() {
        assert_eq!(poll_delay(1), 10_000);
        assert_eq!(poll_delay(3), 40_000);
        assert_eq!(poll_delay(4), 60_000);
    }
}
