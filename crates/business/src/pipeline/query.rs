//! `pipeline/item-query.service.ts` (`listItems`, `listStagePage`,
//! `listStageTotals`, `listForContact`) and `pipeline/field-filter.ts`.
//! Money (`value` filters, stage totals) is the stored x 10,000 integer.

use std::collections::{BTreeMap, HashMap};

use chatbotx_db::Db;
use chatbotx_db::repo::models::PipelineField;
use chatbotx_db::repo::partials::pipeline::pipeline_field_condition_error;
use chatbotx_db::repo::pipeline as pipeline_repo;
use chatbotx_db::repo::pipeline_field_condition::{CustomFieldType, PipelineFieldCondition};
use chatbotx_db::repo::pipeline_item::{self as item_repo, ListStagePageInput};
use chatbotx_db::repo::pipeline_item_cursor::{PipelineItemSort, is_invalid_cursor};
use chatbotx_db::repo::pipeline_item_filter::PipelineItemFilter;
use chatbotx_db::repo::pipeline_stage as stage_repo;
use chatbotx_db::rusqlite::{Connection, OptionalExtension};

use super::access::{PipelineAccessScope, PipelineAction, can_view_contact_email_and_phone};
use super::currency::PipelineView;
use super::item::{ItemView, views_in};
use super::service::{list_with_permissions, require_access_in};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `DETAIL_LIST_LIMIT`.
const DETAIL_LIST_LIMIT: i64 = 100;

/// The public list query as the repository filter, sort and page
/// (`toItemListQuery`).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemListQuery {
    pub filter: PipelineItemFilter,
    pub sort: PipelineItemSort,
    pub limit: i64,
    pub cursor: Option<String>,
}

/// `PipelineItemViewPage`.
#[derive(Debug, Clone)]
pub struct ItemViewPage {
    pub items: Vec<ItemView>,
    pub next_cursor: Option<String>,
}

/// One row of `listStageTotals` (`total_value` x 10,000).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTotal {
    pub stage_id: i64,
    pub name: String,
    pub count: i64,
    pub total_value: i64,
}

/// One row of `listForContact`: the card and its pipeline.
#[derive(Debug, Clone)]
pub struct ContactItemView {
    pub view: ItemView,
    pub pipeline: PipelineView,
}

/// The stored field type (`CustomFieldType`); an unknown one compares as text.
pub fn field_type(text: &str) -> CustomFieldType {
    match text {
        "longText" => CustomFieldType::LongText,
        "email" => CustomFieldType::Email,
        "phoneNumber" => CustomFieldType::PhoneNumber,
        "number" => CustomFieldType::Number,
        "date" => CustomFieldType::Date,
        "datetime" => CustomFieldType::Datetime,
        "boolean" => CustomFieldType::Boolean,
        _ => CustomFieldType::ShortText,
    }
}

/// `assertFieldCondition`: 422 when `condition` cannot run on `field`.
pub fn assert_field_condition(
    field: &PipelineField,
    condition: &PipelineFieldCondition,
) -> Result<()> {
    match pipeline_field_condition_error(field_type(&field.r#type), condition) {
        Some(error) => Err(BusinessError::validation(
            "fieldConditions",
            format!("Field {}: {error}", field.id),
        )),
        None => Ok(()),
    }
}

/// The filter with contact email / phone left out of `search` when the scope
/// may not see them (`scopeSearch`).
fn scope_search(mut filter: PipelineItemFilter, scope: &PipelineAccessScope) -> PipelineItemFilter {
    filter.search_contact_email_and_phone = Some(can_view_contact_email_and_phone(scope));
    filter
}

fn workspace_timezone(conn: &Connection, workspace_id: i64) -> Result<Option<String>> {
    let zone: Option<Option<String>> = conn
        .query_row(
            r#"SELECT "timezone" FROM "Workspace" WHERE "id" = ?1"#,
            [workspace_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(chatbotx_db::repo::Error::from)?;
    Ok(zone.flatten())
}

/// `resolveFieldFilter`: checks each custom-field condition against the
/// pipeline's fields, attaches the field types and (for a datetime field) the
/// request's or the workspace's timezone.
pub fn resolve_field_filter_in(
    conn: &Connection,
    workspace_id: i64,
    pipeline_id: i64,
    mut filter: PipelineItemFilter,
) -> Result<PipelineItemFilter> {
    if filter.field_conditions.is_empty() {
        return Ok(filter);
    }
    let fields = pipeline_repo::list_fields(conn, workspace_id, pipeline_id)?;
    let mut field_types = BTreeMap::new();
    for (id, condition) in &filter.field_conditions {
        let field = fields
            .iter()
            .find(|field| field.id.to_string() == *id)
            .ok_or_else(|| {
                BusinessError::validation(
                    "fieldConditions",
                    format!("Field {id} is not a custom field of this pipeline"),
                )
            })?;
        assert_field_condition(field, condition)?;
        field_types.insert(id.clone(), field_type(&field.r#type));
    }
    let needs_timezone = field_types
        .values()
        .any(|kind| *kind == CustomFieldType::Datetime);
    if needs_timezone && filter.timezone.is_none() {
        filter.timezone = workspace_timezone(conn, workspace_id)?;
    }
    filter.field_types = field_types;
    Ok(filter)
}

/// `withValidCursor`: a cursor of another sort or version is a 422.
fn valid_cursor<T>(result: chatbotx_db::repo::Result<T>) -> Result<T> {
    result.map_err(|error| {
        if is_invalid_cursor(&error) {
            BusinessError::validation("cursor", "Invalid cursor")
        } else {
            error.into()
        }
    })
}

/// `listItems` (`stage_id` `None`) / `listStagePage` (a board column).
pub async fn list_items(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    stage_id: Option<i64>,
    scope: PipelineAccessScope,
    query: ItemListQuery,
) -> Result<ItemViewPage> {
    tx::read(db, move |conn| {
        require_access_in(
            conn,
            workspace_id,
            pipeline_id,
            &scope,
            PipelineAction::View,
        )?;
        let filter = resolve_field_filter_in(
            conn,
            workspace_id,
            pipeline_id,
            scope_search(query.filter, &scope),
        )?;
        if let Some(stage_id) = stage_id
            && stage_repo::find_by_id(conn, workspace_id, Some(pipeline_id), stage_id)?.is_none()
        {
            return Err(BusinessError::not_found("Stage not found"));
        }
        let page = valid_cursor(item_repo::list_stage_page(
            conn,
            &ListStagePageInput {
                workspace_id,
                pipeline_id,
                stage_id,
                filter,
                sort: query.sort,
                limit: query.limit,
                cursor: query.cursor,
            },
        ))?;
        Ok(ItemViewPage {
            items: views_in(conn, workspace_id, page.items, &scope)?,
            next_cursor: page.next_cursor,
        })
    })
    .await
}

/// `listStageTotals`: count and total value per stage, in board order.
pub async fn list_stage_totals(
    db: &Db,
    workspace_id: i64,
    pipeline_id: i64,
    scope: PipelineAccessScope,
    filter: PipelineItemFilter,
) -> Result<Vec<StageTotal>> {
    tx::read(db, move |conn| {
        require_access_in(
            conn,
            workspace_id,
            pipeline_id,
            &scope,
            PipelineAction::View,
        )?;
        let filter = resolve_field_filter_in(
            conn,
            workspace_id,
            pipeline_id,
            scope_search(filter, &scope),
        )?;
        let stages = stage_repo::list_by_pipeline(conn, workspace_id, pipeline_id)?;
        let totals: HashMap<i64, (i64, i64)> =
            item_repo::list_stage_totals(conn, workspace_id, pipeline_id, &filter)?
                .into_iter()
                .map(|row| (row.stage_id, (row.count, row.total_value)))
                .collect();
        Ok(stages
            .into_iter()
            .map(|stage| {
                let (count, total_value) = totals.get(&stage.id).copied().unwrap_or((0, 0));
                StageTotal {
                    stage_id: stage.id,
                    name: stage.name,
                    count,
                    total_value,
                }
            })
            .collect())
    })
    .await
}

/// `listForContact`: the contact's items across the viewable pipelines,
/// newest first (at most 100).
pub async fn list_for_contact(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    scope: PipelineAccessScope,
) -> Result<Vec<ContactItemView>> {
    let pipelines: HashMap<i64, PipelineView> = list_with_permissions(db, workspace_id, scope)
        .await?
        .into_iter()
        .map(|entry| (entry.pipeline.pipeline.id, entry.pipeline))
        .collect();
    let ids: Vec<i64> = pipelines.keys().copied().collect();
    let views = tx::read(db, move |conn| {
        let items = item_repo::list_for_contact(
            conn,
            workspace_id,
            contact_id,
            Some(&ids),
            DETAIL_LIST_LIMIT,
        )?;
        views_in(conn, workspace_id, items, &scope)
    })
    .await?;
    Ok(views
        .into_iter()
        .filter_map(|view| {
            let pipeline = pipelines.get(&view.item.pipeline_id)?.clone();
            Some(ContactItemView { view, pipeline })
        })
        .collect())
}
