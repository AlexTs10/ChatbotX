//! `pipelineSettingsPatchSchema` / `mergePipelineSettings` of
//! `partials/pipeline.ts` (the parse half lives in `chatbotx_db`).

use chatbotx_db::repo::partials::pipeline::{PipelineSettings, PipelineType};

/// `visibleFeatures` patch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VisibleFeaturesPatch {
    pub priority: Option<bool>,
    pub value: Option<bool>,
    pub due_date: Option<bool>,
}

/// `cardElements` patch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CardElementsPatch {
    pub contact_phone: Option<bool>,
    pub contact_email: Option<bool>,
    pub created_at: Option<bool>,
    pub updated_at: Option<bool>,
    pub assignees: Option<bool>,
    pub comments: Option<bool>,
    pub card_id: Option<bool>,
    pub priority: Option<bool>,
    pub value: Option<bool>,
    pub due_date: Option<bool>,
}

/// `statusLabels` patch (labels already trimmed, 1..=40).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusLabelsPatch {
    pub open: Option<String>,
    pub won: Option<String>,
    pub lost: Option<String>,
}

/// `PipelineSettingsPatch`: `None` keeps the stored value;
/// `max_open_items_per_contact: Some(None)` clears the limit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipelineSettingsPatch {
    pub assign_contact_on_assign: Option<bool>,
    pub max_open_items_per_contact: Option<Option<i64>>,
    pub visible_features: VisibleFeaturesPatch,
    pub card_elements: CardElementsPatch,
    pub status_labels: StatusLabelsPatch,
}

/// `mergePipelineSettings`.
pub fn merge_pipeline_settings(
    current: &PipelineSettings,
    patch: &PipelineSettingsPatch,
) -> PipelineSettings {
    let _ = (current, patch);
    todo!("P3.6-D1p")
}

/// `defaultSettingsFor`: the defaults with the type's status labels.
pub fn default_settings_for(pipeline_type: PipelineType) -> PipelineSettings {
    let _ = pipeline_type;
    todo!("P3.6-D1p")
}
