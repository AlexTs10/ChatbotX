//! The Meta Catalog routes of `features/products/api/public.ts`.

use chatbotx_business::meta_catalog::{
    self as service, Deps, ENGLISH_REASONS, HttpGraph, PushScope, RunWithNames,
};
use chatbotx_db::repo::models::IntegrationMetaCatalog;
use serde_json::{Value, json};

use super::input::{Input, Seg, Str, key};
use super::{
    ApiState, Call, ErrorMap, FINDING, LISTING, Reply, business_error, date, id, json, no_content,
    opt_date, opt_id,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "products.getMetaCatalog",
    "products.searchMetaCatalogItems",
    // `whatsappTemplates.searchCatalogProducts`: the same schemas and service.
    "whatsappTemplates.searchCatalogProducts",
    "products.disconnectMetaCatalog",
    "products.listMetaCatalogBusinesses",
    "products.createMetaCatalog",
    "products.selectMetaCatalog",
    "products.syncMetaCatalog",
];

/// `possibleErrorsOnStartingMetaCatalogRun`.
const STARTING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("metaCatalogSyncAlreadyRunning", 409),
    ("metaCatalogReconnectRequired", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
/// `possibleErrorsOnCreatingMetaCatalog`.
const CREATING_CATALOG: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("metaCatalogReconnectRequired", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
/// `possibleErrorsOnDisconnectingMetaCatalog`.
const DISCONNECTING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("metaCatalogSyncAlreadyRunning", 409),
];

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

const DIGITS: Str = Str {
    trim: true,
    min: None,
    max: None,
    pattern: Some(digits),
};

/// `metaCatalogConnectionPublicResource` (no credential, integration,
/// workspace or deletion columns).
fn connection_json(row: &IntegrationMetaCatalog) -> Value {
    json!({
        "id": id(row.id),
        "createdAt": date(row.created_at),
        "updatedAt": date(row.updated_at),
        "catalogId": row.catalog_id,
        "catalogName": row.catalog_name,
        "businessId": row.business_id,
        "authMode": row.auth_mode,
        "tokenExpiresAt": opt_date(row.token_expires_at),
        "status": row.status,
        "importStatus": row.import_status,
        "importTotalCount": row.import_total_count,
        "importedCount": row.imported_count,
        "importFailedCount": row.import_failed_count,
        "importError": row.import_error,
        "lastImportedAt": opt_date(row.last_imported_at),
        "currency": row.currency,
        "storeUrl": row.store_url,
    })
}

/// `metaCatalogSyncRunPublicResource` (no workspace, handles or lease).
fn run_json(row: &RunWithNames) -> Value {
    let run = &row.run;
    json!({
        "id": id(run.id),
        "createdAt": date(run.created_at),
        "updatedAt": date(run.updated_at),
        "integrationMetaCatalogId": id(run.integration_meta_catalog_id),
        "status": run.status,
        "direction": run.direction,
        "catalogId": run.catalog_id,
        "scope": run.scope,
        "categoryId": opt_id(run.category_id),
        "selectedProductIds": run.selected_product_ids.0,
        "totalCount": run.total_count,
        "succeededCount": run.succeeded_count,
        "failedCount": run.failed_count,
        "skippedCount": run.skipped_count,
        "itemErrors": run.item_errors.0,
        "skippedItems": row.skipped_items,
        "pollAttempt": run.poll_attempt,
        "error": run.error,
        "startedAt": opt_date(run.started_at),
        "finishedAt": opt_date(run.finished_at),
    })
}

/// `syncMetaCatalogPublicRequest` (+ the `isMetaCatalogSyncScopeComplete`
/// refine on `scope`).
fn sync_input(input: &mut Input) -> Option<(Option<String>, PushScope)> {
    let raw_scope = input.get("scope").cloned();
    let scope = input.enum_at(&key("scope"), raw_scope.as_ref(), service::SCOPES, true);
    let catalog_id = input.opt_string("catalogId", DIGITS);
    let category_id = input.nullish_id("categoryId").flatten();
    let mut selected: Option<Vec<i64>> = None;
    match input.get("selectedProductIds").cloned() {
        None => {}
        Some(Value::Array(items)) => {
            let path = key("selectedProductIds");
            if items.len() > 1000 {
                input.issue(
                    &path,
                    "too_big",
                    "Too big: expected array to have <=1000 items",
                );
            }
            let mut ids = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let mut at = path.clone();
                at.push(Seg::Index(index));
                ids.push(input.id_at(&at, Some(item), true));
            }
            selected = ids.into_iter().collect();
        }
        Some(other) => {
            let path = key("selectedProductIds");
            input.string_at(&path, Some(&other), Str::default(), true);
        }
    }
    let scope = scope?;
    let complete = match scope.as_str() {
        "category" => category_id.is_some(),
        "selected" => selected.as_ref().is_some_and(|ids| !ids.is_empty()),
        _ => true,
    };
    if !input.failed() && !complete {
        input.issue(
            &key("scope"),
            "custom",
            "The scope needs its categoryId or selectedProductIds",
        );
    }
    Some((
        catalog_id,
        PushScope {
            scope,
            category_id,
            selected_product_ids: selected.unwrap_or_default(),
        },
    ))
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    let graph = HttpGraph::default();
    let deps = Deps {
        db,
        graph: &graph,
        encryption: service::process_encryption(),
        jobs: state.jobs.as_deref(),
    };
    match route_id {
        "products.getMetaCatalog" => {
            let (connection, history) = service::get_state(db, ws)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(
                200,
                json!({
                    "connection": connection.as_ref().map(connection_json),
                    "history": history.iter().map(run_json).collect::<Vec<_>>(),
                }),
            ))
        }
        "products.searchMetaCatalogItems" | "whatsappTemplates.searchCatalogProducts" => {
            let mut input = Input::from_query(call.params, call.query);
            let keyword = input.opt_string("keyword", Str::trimmed(0, 200));
            let retailer_id = input.opt_string("retailerId", Str::trimmed(0, 200));
            input.finish()?;
            let items = service::search_products_for_send(db, ws, keyword, retailer_id)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            Ok(json(
                200,
                json!({
                    "connected": items.is_some(),
                    "items": items.unwrap_or_default().iter().map(|item| json!({
                        "retailerId": item.retailer_id,
                        "name": item.name,
                        "imageUrl": item.image_url,
                    })).collect::<Vec<_>>(),
                }),
            ))
        }
        "products.disconnectMetaCatalog" => service::disconnect(db, ws)
            .await
            .map(|()| no_content())
            .map_err(|e| business_error(e, DISCONNECTING)),
        "products.listMetaCatalogBusinesses" => {
            let businesses = service::list_businesses(&deps, ws)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(
                200,
                Value::Array(
                    businesses
                        .iter()
                        // `name: undefined` is no key in the original.
                        .map(|b| match &b.name {
                            Some(name) => json!({ "id": b.id, "name": name }),
                            None => json!({ "id": b.id }),
                        })
                        .collect(),
                ),
            ))
        }
        "products.createMetaCatalog" => {
            let mut input = Input::from_body(call.params, call.body);
            let business_id = input.string("businessId", DIGITS);
            let name = input.string("name", Str::trimmed(1, 100));
            input.finish()?;
            service::create_and_bind(
                &deps,
                ws,
                &business_id.unwrap_or_default(),
                &name.unwrap_or_default(),
            )
            .await
            .map(|row| json(201, connection_json(&row)))
            .map_err(|e| business_error(e, CREATING_CATALOG))
        }
        "products.selectMetaCatalog" => {
            let mut input = Input::from_body(call.params, call.body);
            let catalog_id = input.string("catalogId", DIGITS);
            input.finish()?;
            service::select(&deps, ws, &catalog_id.unwrap_or_default(), &ENGLISH_REASONS)
                .await
                .map(|row| json(202, connection_json(&row)))
                .map_err(|e| business_error(e, STARTING))
        }
        "products.syncMetaCatalog" => {
            let mut input = Input::from_body(call.params, call.body);
            let parsed = sync_input(&mut input);
            input.finish()?;
            let Some((catalog_id, scope)) = parsed else {
                return Err(super::internal_error("sync input"));
            };
            let run = service::sync_products(&deps, ws, catalog_id, scope, &ENGLISH_REASONS)
                .await
                .map_err(|e| business_error(e, STARTING))?;
            Ok(json(
                202,
                run_json(&RunWithNames {
                    skipped_items: run.skipped_items.0.as_array().cloned().unwrap_or_default(),
                    run,
                }),
            ))
        }
        _ => Err(super::internal_error(route_id)),
    }
}
