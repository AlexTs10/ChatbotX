//! Port of `partials/contact-activity.ts`: the activity-backed contact filter registry
//! (`CONTACT_ACTIVITY_FILTERS`) and the value parser (`parseContactActivityReferences`).
//! The zod schemas are not ported here.

use serde_json::Value as Json;

/// `UNSCOPED_PARENT_REFERENCE_ID`.
pub const UNSCOPED_PARENT_REFERENCE_ID: i64 = 0;
/// `NO_REFERENCE_ID`.
pub const NO_REFERENCE_ID: i64 = -1;
/// `NO_TOPIC_FIXED_VALUE`.
pub const NO_TOPIC_FIXED_VALUE: &str = "noTopic";
/// `CONTACT_ACTIVITY_MAX_FILTER_VALUES`.
pub const CONTACT_ACTIVITY_MAX_FILTER_VALUES: usize = 500;

/// `ContactActivityValueFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueFormat {
    /// `"id"`.
    Id,
    /// `"flowId:nodeId"`.
    FlowIdNodeId,
}

/// `ContactActivityFilterConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactActivityFilterConfig {
    pub activity_type: &'static str,
    pub reference_type: &'static str,
    /// Whether `parentReferenceId` carries a parent id (a flow node's flow).
    pub parent_scoped: bool,
    /// Public values that stand for "no reference" (mapped to [`NO_REFERENCE_ID`]).
    pub fixed_values: &'static [&'static str],
    pub value_format: ValueFormat,
    /// Public operation that supplies selectable values.
    pub value_source_operation: &'static str,
    /// API token scope that operation requires.
    pub value_source_scope: &'static str,
}

const fn config(
    activity_type: &'static str,
    reference_type: &'static str,
    parent_scoped: bool,
    fixed_values: &'static [&'static str],
    value_format: ValueFormat,
    value_source_operation: &'static str,
    value_source_scope: &'static str,
) -> ContactActivityFilterConfig {
    ContactActivityFilterConfig {
        activity_type,
        reference_type,
        parent_scoped,
        fixed_values,
        value_format,
        value_source_operation,
        value_source_scope,
    }
}

const NO_TOPIC: &[&str] = &[NO_TOPIC_FIXED_VALUE];

/// `CONTACT_ACTIVITY_FILTERS`, in the TS key order: `(field, config)`.
pub const CONTACT_ACTIVITY_FILTERS: &[(&str, ContactActivityFilterConfig)] = &[
    (
        "executedFlow",
        config(
            "flowExecuted",
            "flow",
            false,
            &[],
            ValueFormat::Id,
            "flows.list",
            "automation",
        ),
    ),
    (
        "executedStep",
        config(
            "flowNodeExecuted",
            "flowNode",
            true,
            &[],
            ValueFormat::FlowIdNodeId,
            "flows.listNodes",
            "automation",
        ),
    ),
    (
        "keywordsReceived",
        config(
            "automatedResponseMatched",
            "automatedResponse",
            false,
            &[],
            ValueFormat::Id,
            "keywords.list",
            "automation",
        ),
    ),
    (
        "completedWhatsAppFlows",
        config(
            "formCompleted",
            "whatsappFlow",
            false,
            &[],
            ValueFormat::Id,
            "whatsappFlows.list",
            "broadcasts",
        ),
    ),
    (
        "emailSent",
        config(
            "messageSent",
            "emailTopic",
            false,
            NO_TOPIC,
            ValueFormat::Id,
            "emailTopics.list",
            "broadcasts",
        ),
    ),
    (
        "emailDelivered",
        config(
            "messageDelivered",
            "emailTopic",
            false,
            NO_TOPIC,
            ValueFormat::Id,
            "emailTopics.list",
            "broadcasts",
        ),
    ),
    (
        "emailOpened",
        config(
            "messageOpened",
            "emailTopic",
            false,
            NO_TOPIC,
            ValueFormat::Id,
            "emailTopics.list",
            "broadcasts",
        ),
    ),
    (
        "emailClicked",
        config(
            "messageClicked",
            "emailTopic",
            false,
            NO_TOPIC,
            ValueFormat::Id,
            "emailTopics.list",
            "broadcasts",
        ),
    ),
    (
        "clickedAd",
        config(
            "adClicked",
            "ad",
            false,
            &[],
            ValueFormat::Id,
            "ads.listReferralAds",
            "ads",
        ),
    ),
];

/// `isContactActivityFilterField` + the lookup: the field's config.
pub fn contact_activity_filter(field: &str) -> Option<&'static ContactActivityFilterConfig> {
    CONTACT_ACTIVITY_FILTERS
        .iter()
        .find(|(name, _)| *name == field)
        .map(|(_, config)| config)
}

/// `isPositiveBigint`: `^[1-9]\d*$` and at most `i64::MAX`.
pub fn positive_bigint(raw: &str) -> Option<i64> {
    let mut chars = raw.chars();
    match chars.next() {
        Some('1'..='9') if chars.all(|c| c.is_ascii_digit()) => raw.parse::<i64>().ok(),
        _ => None,
    }
}

/// `ContactActivityReference` (ids as integers).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContactActivityReference {
    pub parent_reference_id: i64,
    pub reference_id: i64,
}

fn parse_reference(format: ValueFormat, raw: &str) -> Option<ContactActivityReference> {
    match format {
        ValueFormat::Id => positive_bigint(raw).map(|reference_id| ContactActivityReference {
            parent_reference_id: UNSCOPED_PARENT_REFERENCE_ID,
            reference_id,
        }),
        ValueFormat::FlowIdNodeId => {
            let parts: Vec<&str> = raw.split(':').collect();
            let [parent, reference] = parts.as_slice() else {
                return None;
            };
            Some(ContactActivityReference {
                parent_reference_id: positive_bigint(parent)?,
                reference_id: positive_bigint(reference)?,
            })
        }
    }
}

/// `parseContactActivityReferences`: a bare string or a list of 1..500 strings, each a
/// fixed value (-> the no-reference sentinel) or an id in the field's format; duplicates
/// collapse (first occurrence order). `None` when any value is unusable.
pub fn parse_contact_activity_references(
    config: &ContactActivityFilterConfig,
    value: Option<&Json>,
) -> Option<Vec<ContactActivityReference>> {
    let raws: Vec<&Json> = match value {
        Some(Json::Array(items)) => items.iter().collect(),
        Some(other) => vec![other],
        None => return None,
    };
    if raws.is_empty() || raws.len() > CONTACT_ACTIVITY_MAX_FILTER_VALUES {
        return None;
    }
    let mut parsed: Vec<ContactActivityReference> = Vec::new();
    for raw in raws {
        let raw = raw.as_str()?;
        let reference = if config.fixed_values.contains(&raw) {
            ContactActivityReference {
                parent_reference_id: UNSCOPED_PARENT_REFERENCE_ID,
                reference_id: NO_REFERENCE_ID,
            }
        } else {
            parse_reference(config.value_format, raw)?
        };
        if !parsed.contains(&reference) {
            parsed.push(reference);
        }
    }
    Some(parsed)
}
