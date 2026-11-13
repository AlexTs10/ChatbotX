//! `apps/builder/src/features/contact-filter/lib/list-contact-filter-fields.ts`
//! (`listContactFilterFieldsForAPI`) with the parts of
//! `contact-filter/schema/definitions.ts` (`CONTACT_FILTER_FIELD_DEFINITIONS`:
//! `schemaKind`, `optionSource`, `hidden`, `surfaces`),
//! `custom-field-filter.ts` (`convertCustomFieldTypeToConditionType`) and
//! `pipeline-item-filter.ts` (`PIPELINE_ITEM_ATTRIBUTE_DESCRIPTORS`) it reads
//! (P3.6-D2 slice filter fields).

use chatbotx_db::Db;
use serde_json::Value;

use crate::contact_filter::Surface;
use crate::errors::Result;

/// One `CONTACT_FILTER_FIELD_DEFINITIONS` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticFieldDefinition {
    pub field: &'static str,
    pub schema_kind: &'static str,
    pub option_source: &'static str,
    /// Valid in saved filters, not offered for new conditions.
    pub hidden: bool,
    /// `surfaces: ["flowCondition"]`.
    pub flow_condition_only: bool,
}

/// `convertCustomFieldTypeToConditionType`.
pub fn convert_custom_field_type_to_condition_type(field_type: &str) -> &'static str {
    let _ = field_type;
    todo!("P3.6-D2 filter fields stage 2")
}

/// The static fields offered on `surface` (`tracked_fields_enabled` =
/// `isContactActivityFiltersEnabled()`), as `ContactFilterFieldPublicResource`s.
pub fn static_filter_fields(surface: Surface, tracked_fields_enabled: bool) -> Vec<Value> {
    let _ = (surface, tracked_fields_enabled);
    todo!("P3.6-D2 filter fields stage 2")
}

/// `PIPELINE_ITEM_ATTRIBUTE_DESCRIPTORS` as JSON.
pub fn pipeline_item_attribute_descriptors() -> Vec<Value> {
    todo!("P3.6-D2 filter fields stage 2")
}

/// `listContactFilterFieldsForAPI`: `{staticFields, customFields, botFields,
/// tags, pipelineItems}` of a workspace (a workspace token sees every pipeline).
pub async fn list_contact_filter_fields_for_api(
    db: &Db,
    workspace_id: i64,
    surface: Surface,
    tracked_fields_enabled: bool,
) -> Result<Value> {
    let _ = (db, workspace_id, surface, tracked_fields_enabled);
    todo!("P3.6-D2 filter fields stage 2")
}
