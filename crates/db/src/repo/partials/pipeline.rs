//! Port of the settings, reference-name, activity-group and operand helpers of
//! `partials/pipeline.ts`. `isPipelineFilterDay` and the field condition live in
//! `repo::pipeline_field_condition` (P3.2b slice D2) and are re-exported here.

use serde::{Deserialize, Serialize};

pub use crate::repo::pipeline_field_condition::{
    CustomFieldType, PipelineFieldCondition, PipelineFieldOperator, is_pipeline_filter_day,
    is_pipeline_filter_number,
};

use super::shared::{Issues, ZodError, deserialize, js_length};
use crate::repo::utils::normalize_for_match;

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
#[serde(default)]
pub struct PipelineStatusLabels {
    pub open: String,
    pub won: String,
    pub lost: String,
}

/// `pipelineSettingsSchema.visibleFeatures`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PipelineVisibleFeatures {
    pub priority: bool,
    pub value: bool,
    pub due_date: bool,
}

/// `pipelineSettingsSchema.cardElements`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
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
#[serde(rename_all = "camelCase", default)]
pub struct PipelineSettings {
    pub version: i64,
    pub assign_contact_on_assign: bool,
    pub max_open_items_per_contact: Option<i64>,
    pub visible_features: PipelineVisibleFeatures,
    pub card_elements: PipelineCardElements,
    pub status_labels: PipelineStatusLabels,
}

impl Default for PipelineStatusLabels {
    fn default() -> PipelineStatusLabels {
        PipelineStatusLabels {
            open: "Open".to_owned(),
            won: "Won".to_owned(),
            lost: "Lost".to_owned(),
        }
    }
}

impl Default for PipelineVisibleFeatures {
    fn default() -> PipelineVisibleFeatures {
        PipelineVisibleFeatures {
            priority: true,
            value: true,
            due_date: true,
        }
    }
}

impl Default for PipelineCardElements {
    fn default() -> PipelineCardElements {
        PipelineCardElements {
            contact_phone: false,
            contact_email: false,
            created_at: true,
            updated_at: false,
            assignees: true,
            comments: true,
            card_id: true,
            priority: true,
            value: true,
            due_date: true,
        }
    }
}

impl Default for PipelineSettings {
    fn default() -> PipelineSettings {
        PipelineSettings {
            version: 1,
            assign_contact_on_assign: false,
            max_open_items_per_contact: None,
            visible_features: PipelineVisibleFeatures::default(),
            card_elements: PipelineCardElements::default(),
            status_labels: PipelineStatusLabels::default(),
        }
    }
}

/// The longest status label (`trimmedLabel`: `z.string().trim().min(1).max(40)`).
const STATUS_LABEL_MAX: usize = 40;

/// `pipelineSettingsSchema.parse` (missing keys get their defaults).
pub fn parse_pipeline_settings(value: &serde_json::Value) -> Result<PipelineSettings, ZodError> {
    let mut settings: PipelineSettings = deserialize(value)?;
    let mut issues = Issues::default();
    if settings.version != 1 {
        issues.add("version", "Invalid input: expected 1");
    }
    if let Some(max) = settings.max_open_items_per_contact
        && !(1..=100).contains(&max)
    {
        issues.add(
            "maxOpenItemsPerContact",
            "Invalid input: expected an integer from 1 to 100",
        );
    }
    let labels = &mut settings.status_labels;
    for (path, label) in [
        ("statusLabels.open", &mut labels.open),
        ("statusLabels.won", &mut labels.won),
        ("statusLabels.lost", &mut labels.lost),
    ] {
        *label = label.trim().to_owned();
        let length = js_length(label);
        if length < 1 {
            issues.add(path, "Too small: expected string to have >=1 characters");
        } else if length > STATUS_LABEL_MAX {
            issues.add(path, "Too big: expected string to have <=40 characters");
        }
    }
    issues.finish(settings)
}

/// `PIPELINE_REFERENCE_NAME_MAX`.
pub const PIPELINE_REFERENCE_NAME_MAX: usize = 64;

/// `pipelineReferenceNameSchema.safeParse` (`^[a-z][a-z0-9_]{0,63}$`).
pub fn parse_pipeline_reference_name(value: &str) -> Result<String, ZodError> {
    let mut chars = value.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && value.len() <= PIPELINE_REFERENCE_NAME_MAX
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if valid {
        Ok(value.to_owned())
    } else {
        Err(ZodError::one(
            "",
            "Invalid string: must match pattern /^[a-z][a-z0-9_]{0,63}$/",
        ))
    }
}

/// `referenceNameFromName`: the folded name as a reference name; a name that does not
/// start with a letter gets the `fallback` prefix.
pub fn reference_name_from_name(name: &str, fallback: &str) -> String {
    let folded = normalize_for_match(name);
    let mut base = String::with_capacity(folded.len());
    let mut in_run = false;
    for c in folded.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            base.push(c);
            in_run = false;
        } else if !in_run {
            base.push('_');
            in_run = true;
        }
    }
    let base = base.trim_matches('_');
    let prefixed = if base.starts_with(|c: char| c.is_ascii_lowercase()) {
        base.to_owned()
    } else {
        format!("{fallback}_{base}")
    };
    // `.slice(0, 64)` counts UTF-16 units; the folded base is ASCII.
    let trimmed = prefixed.trim_matches('_');
    let mut out = String::new();
    let mut units = 0;
    for c in trimmed.chars() {
        units += c.len_utf16();
        if units > PIPELINE_REFERENCE_NAME_MAX {
            break;
        }
        out.push(c);
    }
    out
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
    match pipeline_type {
        PipelineType::Opportunity => PipelineTypeDefaults {
            first_stage_name: "Lead",
            status_labels: ("Open", "Won", "Lost"),
            noun_key: "opportunity",
        },
        PipelineType::Ticket => PipelineTypeDefaults {
            first_stage_name: "New Issue",
            status_labels: ("Open", "Resolved", "Not Resolved"),
            noun_key: "ticket",
        },
        PipelineType::Custom => PipelineTypeDefaults {
            first_stage_name: "Intake",
            status_labels: ("Open", "Resolved", "Not Resolved"),
            noun_key: "card",
        },
    }
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
    let groups = groups.filter(|groups| !groups.is_empty())?;
    let mut kinds: Vec<&'static str> = Vec::new();
    for group in groups {
        let Some((_, group_kinds)) = PIPELINE_ACTIVITY_KINDS_BY_GROUP
            .iter()
            .find(|(name, _)| name == group)
        else {
            continue;
        };
        for kind in *group_kinds {
            if !kinds.contains(kind) {
                kinds.push(kind);
            }
        }
    }
    Some(kinds)
}

/// `pipelineFieldConditionError`: why the condition is not valid for the field type.
pub fn pipeline_field_condition_error(
    field_type: CustomFieldType,
    condition: &PipelineFieldCondition,
) -> Option<String> {
    use PipelineFieldOperator::*;
    let operator = condition.operator;
    let allowed: &[PipelineFieldOperator] = match field_type {
        CustomFieldType::ShortText
        | CustomFieldType::LongText
        | CustomFieldType::Email
        | CustomFieldType::PhoneNumber => &[
            Is,
            IsNot,
            HasValue,
            NoValue,
            Contains,
            NotContains,
            StartsWith,
            EndsWith,
        ],
        CustomFieldType::Number | CustomFieldType::Date | CustomFieldType::Datetime => {
            &[Is, IsNot, HasValue, NoValue, Gt, Lt, Between]
        }
        CustomFieldType::Boolean => &[Is, IsNot, HasValue, NoValue],
    };
    let operator_name = field_operator_name(operator);
    if !allowed.contains(&operator) {
        return Some(format!(
            "Operator {operator_name} is not available for {} fields",
            field_type_name(field_type)
        ));
    }
    if matches!(operator, HasValue | NoValue) {
        return None;
    }
    let (valid, expected): (fn(&str) -> bool, &str) = match field_type {
        CustomFieldType::Number => (|t| is_pipeline_filter_number(Some(t)), "a number"),
        CustomFieldType::Date | CustomFieldType::Datetime => {
            (|t| is_pipeline_filter_day(Some(t)), "a date as YYYY-MM-DD")
        }
        CustomFieldType::Boolean => (|t| t == "true" || t == "false", "\"true\" or \"false\""),
        _ => (|t| !t.is_empty(), "a non-empty text"),
    };
    let operand_error = |name: &str, text: Option<&str>| match text {
        Some(text) if !text.is_empty() && valid(text) => None,
        _ => Some(format!(
            "Operator {operator_name} needs {name} as {expected}"
        )),
    };
    if operator != Between {
        return operand_error("value", condition.value.as_deref());
    }
    if condition.value.is_none() && condition.value_to.is_none() {
        return Some("Operator between needs value, valueTo or both".to_owned());
    }
    condition
        .value
        .as_deref()
        .and_then(|value| operand_error("value", Some(value)))
        .or_else(|| {
            condition
                .value_to
                .as_deref()
                .and_then(|value_to| operand_error("valueTo", Some(value_to)))
        })
}

/// The TS text of a `PipelineFieldOperator`.
fn field_operator_name(operator: PipelineFieldOperator) -> &'static str {
    use PipelineFieldOperator::*;
    match operator {
        Is => "is",
        IsNot => "isNot",
        HasValue => "hasValue",
        NoValue => "noValue",
        Contains => "contains",
        NotContains => "notContains",
        StartsWith => "startsWith",
        EndsWith => "endsWith",
        Gt => "gt",
        Lt => "lt",
        Between => "between",
    }
}

/// The TS text of a `CustomFieldType`.
fn field_type_name(field_type: CustomFieldType) -> &'static str {
    match field_type {
        CustomFieldType::ShortText => "shortText",
        CustomFieldType::LongText => "longText",
        CustomFieldType::Email => "email",
        CustomFieldType::PhoneNumber => "phoneNumber",
        CustomFieldType::Number => "number",
        CustomFieldType::Date => "date",
        CustomFieldType::Datetime => "datetime",
        CustomFieldType::Boolean => "boolean",
    }
}
