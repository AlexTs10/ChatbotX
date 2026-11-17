//! `productMessageService` (`listSendable`, `prepareOutgoing`,
//! `resolveInboundProducts`) with the Meta catalog source
//! (`catalog-sources.ts`): only WhatsApp sends product messages.

use std::collections::HashMap;

use chatbotx_core::flow_config::{
    channel_aware_step_validators, get_channel_flow_policy, resolve_step_validator,
    send_products_step_default_fn,
};
use chatbotx_db::Db;
use chatbotx_db::repo::meta_catalog_item::{
    self as catalog, CatalogScope, SendableListInput, SendableProduct,
};
use chatbotx_db::rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// `catalogSourceByChannel`: only WhatsApp has a catalog source.
fn has_catalog_source(channel: &str) -> bool {
    channel == "whatsapp"
}

/// `resolveMetaCatalogScope`: the live connection bound to a catalog.
fn meta_catalog_scope(conn: &Connection, workspace_id: i64) -> Result<Option<CatalogScope>> {
    let row: Option<(i64, Option<String>)> = conn
        .query_row(
            r#"SELECT "id", "catalogId" FROM "IntegrationMetaCatalog" WHERE "workspaceId" = ?1 AND "deletedAt" IS NULL ORDER BY "id" LIMIT 1"#,
            [workspace_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| BusinessError::Repo(e.into()))?;
    Ok(row.and_then(|(id, catalog_id)| {
        catalog_id
            .filter(|c| !c.is_empty())
            .map(|catalog_id| CatalogScope {
                integration_meta_catalog_id: id,
                catalog_id,
            })
    }))
}

fn not_ready(reason: &'static str) -> Sendable {
    Sendable {
        ready: false,
        reason: Some(reason),
        data: Vec::new(),
        page_count: 0,
    }
}

/// `listSendable` input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListSendable {
    pub workspace_id: i64,
    pub channel: String,
    pub keyword: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

/// `SendableProductsResult`: `data` items are `{id, name, imageUrl, price, currency}`.
#[derive(Debug, Clone, PartialEq)]
pub struct Sendable {
    pub ready: bool,
    /// `channelUnsupported` or `catalogNotConnected` when not ready.
    pub reason: Option<&'static str>,
    pub data: Vec<Value>,
    pub page_count: i64,
}

/// `listSendable`.
pub async fn list_sendable(db: &Db, input: ListSendable) -> Result<Sendable> {
    let supported = get_channel_flow_policy(Some(&input.channel))
        .is_some_and(|p| p.product_messages.is_some())
        && has_catalog_source(&input.channel);
    if !supported {
        return Ok(not_ready("channelUnsupported"));
    }
    tx::read(db, move |conn| {
        let Some(scope) = meta_catalog_scope(conn, input.workspace_id)? else {
            return Ok(not_ready("catalogNotConnected"));
        };
        let page = catalog::list_sendable_products(
            conn,
            &SendableListInput {
                scope,
                workspace_id: input.workspace_id,
                keyword: input.keyword,
                product_ids: None,
                page: input.page,
                per_page: input.per_page,
            },
        )?;
        let data = page
            .data
            .into_iter()
            .map(|row| {
                json!({
                    "id": row.product_id.to_string(),
                    "name": row.name,
                    "imageUrl": row.images.0.get(0).and_then(|i| i.get("url")).cloned().unwrap_or(Value::Null),
                    "price": row.price,
                    "currency": row.currency,
                })
            })
            .collect();
        Ok(Sendable {
            ready: true,
            reason: None,
            data,
            page_count: page.page_count,
        })
    })
    .await
}

/// `ProductMessageUnresolvableError(reason, productIds?)`.
pub fn unresolvable(reason: &str, product_ids: Option<&[String]>) -> BusinessError {
    let mut data = json!({ "reason": reason });
    let mut message = format!("Product message cannot be sent: {reason}");
    if let Some(ids) = product_ids {
        data["productIds"] = json!(ids.join(","));
        message.push_str(&format!(" ({})", ids.join(", ")));
    }
    BusinessError::Exception {
        message,
        code: "productMessageUnresolvable".into(),
        status: 422,
        field: None,
        data: Some(data),
    }
}

/// `toSendableProductRecord` (`ProductMessageItem`).
fn sendable_record(row: &SendableProduct) -> Value {
    json!({
        "productId": row.product_id.to_string(),
        "name": row.name,
        "imageUrl": row.images.0.get(0).and_then(|i| i.get("url")).cloned().unwrap_or(Value::Null),
        "price": row.price,
        "currency": row.currency,
        "catalogReference": {
            "source": "metaCatalog",
            "catalogId": row.catalog_id,
            "retailerId": row.retailer_id,
        },
    })
}

/// `PreparedProductMessage`.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedProductMessage {
    /// `ResolvedProductMessage`.
    pub message: Value,
    /// Product ids no longer sendable (`[{ productId }]`).
    pub missing: Vec<String>,
    pub text: Option<String>,
    /// `{ type: "product_message", payload: message }`.
    pub content_attributes: Value,
}

/// `prepareOutgoing`: `reject_missing` = `onMissing: "reject"`.
pub async fn prepare_outgoing(
    db: &Db,
    workspace_id: i64,
    channel: &str,
    content: &Value,
    reject_missing: bool,
) -> Result<PreparedProductMessage> {
    let capability = get_channel_flow_policy(Some(channel))
        .and_then(|p| p.product_messages.clone())
        .ok_or_else(|| unresolvable("channelUnsupported", None))?;
    if let Some(validator) = channel_aware_step_validators().get("sendProducts") {
        let parsed = resolve_step_validator(validator, channel)
            .safe_parse(&send_products_step_default_fn(content));
        if !parsed.success {
            let issues = parsed
                .issues()
                .iter()
                .map(|i| format!("{}: {}", i.path_string(), i.message))
                .collect::<Vec<_>>()
                .join("; ");
            let mut error = BusinessError::validation("products", "Invalid product message");
            if let BusinessError::Exception { data, .. } = &mut error {
                *data = Some(json!({ "issues": issues }));
            }
            return Err(error);
        }
    }
    let display = content.get("display").and_then(Value::as_str).unwrap_or("");
    if !has_catalog_source(channel)
        || capability["displays"]
            .get(display)
            .is_none_or(Value::is_null)
    {
        return Err(unresolvable("channelUnsupported", None));
    }
    let ids: Vec<i64> = content["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|s| s["productIds"].as_array().cloned().unwrap_or_default())
        .filter_map(|id| id.as_str().and_then(|id| id.parse().ok()))
        .collect();
    let rows = tx::read(db, move |conn| {
        let Some(scope) = meta_catalog_scope(conn, workspace_id)? else {
            return Ok(None);
        };
        Ok(Some(catalog::find_sendable_by_product_ids(
            conn,
            &scope,
            workspace_id,
            &ids,
        )?))
    })
    .await?
    .ok_or_else(|| unresolvable("catalogNotConnected", None))?;
    let sendable: HashMap<String, Value> = rows
        .iter()
        .map(|row| (row.product_id.to_string(), sendable_record(row)))
        .collect();
    let (message, missing) =
        match super::resolve::resolve_product_message(&capability, content, &sendable) {
            super::resolve::Resolution::Ok { message, missing } => (message, missing),
            super::resolve::Resolution::Failed("invalidMessage") => {
                return Err(BusinessError::validation(
                    "products",
                    "Invalid product message",
                ));
            }
            super::resolve::Resolution::Failed(reason) => return Err(unresolvable(reason, None)),
        };
    if reject_missing && !missing.is_empty() {
        return Err(unresolvable("productsNotSendable", Some(&missing)));
    }
    // `message.body ?? (display === "products" ? message.header : null)`.
    let text = message
        .get("body")
        .or_else(|| {
            (message["display"] == "products")
                .then(|| message.get("header"))
                .flatten()
        })
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(PreparedProductMessage {
        content_attributes: json!({ "type": "product_message", "payload": message }),
        message,
        missing,
        text,
    })
}

/// `unresolvedInboundReference("metaCatalog", ..)`.
fn unresolved_reference(catalog_id: &str, retailer_id: &str) -> Value {
    json!({
        "catalogReference": { "source": "metaCatalog", "catalogId": catalog_id, "retailerId": retailer_id },
        "product": null,
    })
}

/// `resolveInboundProducts`: one `ResolvedInboundProductReference` per
/// distinct retailer id, in first-seen order.
pub async fn resolve_inbound_products(
    db: &Db,
    workspace_id: i64,
    channel: &str,
    catalog_id: &str,
    retailer_ids: &[String],
) -> Vec<Value> {
    if !has_catalog_source(channel) {
        return Vec::new();
    }
    let mut ids: Vec<String> = Vec::new();
    for id in retailer_ids {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    if ids.is_empty() {
        return Vec::new();
    }
    let unresolved: Vec<Value> = ids
        .iter()
        .map(|id| unresolved_reference(catalog_id, id))
        .collect();
    let (wanted, lookup_ids) = (catalog_id.to_owned(), ids.clone());
    let rows = tx::read(db, move |conn| {
        let Some(scope) =
            meta_catalog_scope(conn, workspace_id)?.filter(|s| s.catalog_id == wanted)
        else {
            return Ok(None);
        };
        let refs: Vec<&str> = lookup_ids.iter().map(String::as_str).collect();
        Ok(Some(catalog::find_linked_products_by_retailer_ids(
            conn,
            &scope,
            workspace_id,
            &refs,
        )?))
    })
    .await;
    let rows = match rows {
        Ok(Some(rows)) => rows,
        Ok(None) => return unresolved,
        Err(error) => {
            tracing::warn!(%error, workspace_id, channel, "Inbound product lookup failed; reporting the references unresolved");
            return unresolved;
        }
    };
    unresolved
        .into_iter()
        .map(|reference| {
            let retailer_id = reference["catalogReference"]["retailerId"]
                .as_str()
                .unwrap_or("");
            match rows.iter().find(|row| row.retailer_id == retailer_id) {
                Some(row) => {
                    let mut product = sendable_record(row);
                    let catalog_reference = product
                        .as_object_mut()
                        .and_then(|p| p.remove("catalogReference"));
                    json!({ "catalogReference": catalog_reference, "product": product })
                }
                None => reference,
            }
        })
        .collect()
}
