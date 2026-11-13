//! `pipeline/item-query.service.ts` (`listItems`, `listStagePage`,
//! `listStageTotals`, `listForContact`) and `pipeline/field-filter.ts`.
//! Money (`value` filters, stage totals) is the stored x 10,000 integer.

use chatbotx_db::Db;
use chatbotx_db::repo::models::PipelineField;
use chatbotx_db::repo::pipeline_field_condition::PipelineFieldCondition;
use chatbotx_db::repo::pipeline_item_filter::PipelineItemFilter;
use chatbotx_db::repo::pipeline_item_cursor::PipelineItemSort;

use super::access::PipelineAccessScope;
use super::currency::PipelineView;
use super::item::ItemView;
use crate::errors::Result;

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

/// `assertFieldCondition`: 422 when `condition` cannot run on `field`.
pub fn assert_field_condition(
    _field: &PipelineField,
    _condition: &PipelineFieldCondition,
) -> Result<()> {
    todo!("P3.6-D1p")
}

/// `listItems` (`stage_id` `None`) / `listStagePage` (a board column).
pub async fn list_items(
    _db: &Db,
    _workspace_id: i64,
    _pipeline_id: i64,
    _stage_id: Option<i64>,
    _scope: PipelineAccessScope,
    _query: ItemListQuery,
) -> Result<ItemViewPage> {
    todo!("P3.6-D1p")
}

/// `listStageTotals`: count and total value per stage, in board order.
pub async fn list_stage_totals(
    _db: &Db,
    _workspace_id: i64,
    _pipeline_id: i64,
    _scope: PipelineAccessScope,
    _filter: PipelineItemFilter,
) -> Result<Vec<StageTotal>> {
    todo!("P3.6-D1p")
}

/// `listForContact`: the contact's items across the viewable pipelines.
pub async fn list_for_contact(
    _db: &Db,
    _workspace_id: i64,
    _contact_id: i64,
    _scope: PipelineAccessScope,
) -> Result<Vec<ContactItemView>> {
    todo!("P3.6-D1p")
}
