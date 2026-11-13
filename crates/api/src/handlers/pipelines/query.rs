//! `listItemsPublicRequest` / `stageTotalsPublicRequest` + `toItemFilter` /
//! `toItemListQuery` (`lib/list-items-query.ts`) and the routes
//! `pipelines.listItems`, `stageTotals`, `listContactItems`.

use chatbotx_business::pipeline::query::ItemListQuery;
use chatbotx_db::repo::pipeline_item_filter::PipelineItemFilter;

use crate::orpc::OrpcError;

/// `listItemsPublicRequest.parse` + `toItemListQuery`.
pub fn item_list_query(
    _params: &[(&'static str, String)],
    _query: Option<&str>,
) -> Result<ItemListQuery, OrpcError> {
    todo!("P3.6-D1p")
}

/// `stageTotalsPublicRequest.parse` + `toItemFilter`.
pub fn item_filter_query(
    _params: &[(&'static str, String)],
    _query: Option<&str>,
) -> Result<PipelineItemFilter, OrpcError> {
    todo!("P3.6-D1p")
}
