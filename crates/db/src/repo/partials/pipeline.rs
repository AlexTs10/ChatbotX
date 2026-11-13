//! Port of the settings, reference-name, activity-group and operand helpers of
//! `partials/pipeline.ts`. `isPipelineFilterDay` and the field condition live in
//! `repo::pipeline_field_condition` (P3.2b slice D2) and are re-exported here.

use serde::{Deserialize, Serialize};

pub use crate::repo::pipeline_field_condition::{
    CustomFieldType, PipelineFieldCondition, PipelineFieldOperator, is_pipeline_filter_day,
};

use super::shared::ZodError;

/// `pipelineTypes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PipelineType {
    Opportunity,
    Ticket,
    Custom,
}

/// `pipelineTypes.options`.
pub const PIPELINE_TYPES: [PipelineType; 3] = [
    PipelineType::Opportunity,
    PipelineType::Ticket,
    PipelineType::Custom,
];

/// `PipelineSettings["statusLabels"]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PipelineStatusLabels {
    pub open: String,
    pub won: String,
    pub lost: String,
}

/// `pipelineSettingsSchema.visibleFeatures`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineVisibleFeatures {
    pub priority: bool,
    pub value: bool,
    pub due_date: bool,
}

/// `pipelineSettingsSchema.cardElements`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineCardElements {
    pub contact_phone: bool,
    pub contact_email: bool,
    pub created_at: bool,
    pub updated_at: bool,
    pub assignees: bool,
    pub comments: bool,
    pub card_id: bool,
    pub priority: bool,
    pub value: bool,
    pub due_date: bool,
}

/// `PipelineSettings` (every key backfilled with its default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSettings {
    pub version: i64,
    pub assign_contact_on_assign: bool,
    pub max_open_items_per_contact: Option<i64>,
    pub visible_features: PipelineVisibleFeatures,
    pub card_elements: PipelineCardElements,
    pub status_labels: PipelineStatusLabels,
}

/// `pipelineSettingsSchema.parse` (missing keys get their defaults).
pub fn parse_pipeline_settings(value: &serde_json::Value) -> Result<PipelineSettings, ZodError> {
    let _ = value;
    todo!("P3.2b")
}

/// `PIPELINE_REFERENCE_NAME_MAX`.
pub const PIPELINE_REFERENCE_NAME_MAX: usize = 64;

/// `pipelineReferenceNameSchema.safeParse` (`^[a-z][a-z0-9_]{0,63}$`).
pub fn parse_pipeline_reference_name(value: &str) -> Result<String, ZodError> {
    let _ = value;
    todo!("P3.2b")
}

/// `referenceNameFromName`: the folded name as a reference name; a name that does not
/// start with a letter gets the `fallback` prefix.
pub fn reference_name_from_name(name: &str, fallback: &str) -> String {
    let _ = (name, fallback);
    todo!("P3.2b")
}

/// `pipelineTypeDefaults[type]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineTypeDefaults {
    pub first_stage_name: &'static str,
    /// `open`, `won`, `lost`.
    pub status_labels: (&'static str, &'static str, &'static str),
    pub noun_key: &'static str,
}

/// `pipelineTypeDefaults`.
pub fn pipeline_type_defaults(pipeline_type: PipelineType) -> PipelineTypeDefaults {
    let _ = pipeline_type;
    todo!("P3.2b")
}

/// The numeric `PIPELINE_LIMITS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipelineLimits {
    pub name: usize,
    pub title: usize,
    pub description: usize,
    pub comment: usize,
    pub stage_description: usize,
    pub idempotency_key: usize,
    pub bulk_sync_items: usize,
    pub bulk_batch_size: usize,
    pub assignees_per_item: usize,
    pub item_value: i64,
    pub stages_per_pipeline: usize,
    pub fields_per_pipeline: usize,
    pub attachments_per_item: usize,
}

/// `PIPELINE_LIMITS` (the color / currency patterns are not ported here).
pub const PIPELINE_LIMITS: PipelineLimits = PipelineLimits {
    name: 120,
    title: 200,
    description: 5000,
    comment: 2000,
    stage_description: 300,
    idempotency_key: 128,
    bulk_sync_items: 2000,
    bulk_batch_size: 500,
    assignees_per_item: 20,
    item_value: 999_999_999_999_999,
    stages_per_pipeline: 200,
    fields_per_pipeline: 100,
    attachments_per_item: 50,
};

/// `PIPELINE_ACTIVITY_KINDS_BY_GROUP` (in `pipelineActivityGroups` order).
pub const PIPELINE_ACTIVITY_KINDS_BY_GROUP: &[(&str, &[&str])] = &[
    ("lifecycle", &["created", "transferred", "deleted"]),
    ("stage", &["stageChanged"]),
    ("status", &["statusChanged"]),
    ("value", &["valueChanged"]),
    ("priority", &["priorityChanged"]),
    ("dueDate", &["dueDateChanged"]),
    (
        "details",
        &["titleChanged", "descriptionChanged", "fieldValuesChanged"],
    ),
    ("assignees", &["assigneesChanged"]),
    ("comments", &["commentAdded", "commentDeleted"]),
    ("attachments", &["attachmentAdded", "attachmentRemoved"]),
];

/// `activityKindsOf`: `None` for no groups, else the kinds of the groups, each once.
pub fn activity_kinds_of(groups: Option<&[&str]>) -> Option<Vec<&'static str>> {
    let _ = groups;
    todo!("P3.2b")
}

/// `pipelineFieldConditionError`: why the condition is not valid for the field type.
pub fn pipeline_field_condition_error(
    field_type: CustomFieldType,
    condition: &PipelineFieldCondition,
) -> Option<String> {
    let _ = (field_type, condition);
    todo!("P3.2b")
}
