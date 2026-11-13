//! `productMessageService.listSendable` with the Meta catalog source
//! (`catalog-sources.ts`): only WhatsApp sends product messages.

use chatbotx_core::flow_config::channel_rules::get_channel_flow_policy;
use chatbotx_db::Db;
use chatbotx_db::repo::meta_catalog_item::{self as catalog, CatalogScope, SendableListInput};
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
