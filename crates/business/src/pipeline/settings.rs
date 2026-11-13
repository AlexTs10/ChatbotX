//! `pipelineSettingsPatchSchema` / `mergePipelineSettings` of
//! `partials/pipeline.ts` (the parse half lives in `chatbotx_db`).

use chatbotx_db::repo::partials::pipeline::{
    PipelineSettings, PipelineStatusLabels, PipelineType, pipeline_type_defaults,
};

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
    fn set<T: Clone>(slot: &mut T, value: &Option<T>) {
        if let Some(value) = value {
            *slot = value.clone();
        }
    }
    let mut out = current.clone();
    set(
        &mut out.assign_contact_on_assign,
        &patch.assign_contact_on_assign,
    );
    set(
        &mut out.max_open_items_per_contact,
        &patch.max_open_items_per_contact,
    );
    let v = &patch.visible_features;
    set(&mut out.visible_features.priority, &v.priority);
    set(&mut out.visible_features.value, &v.value);
    set(&mut out.visible_features.due_date, &v.due_date);
    let c = &patch.card_elements;
    let card = &mut out.card_elements;
    set(&mut card.contact_phone, &c.contact_phone);
    set(&mut card.contact_email, &c.contact_email);
    set(&mut card.created_at, &c.created_at);
    set(&mut card.updated_at, &c.updated_at);
    set(&mut card.assignees, &c.assignees);
    set(&mut card.comments, &c.comments);
    set(&mut card.card_id, &c.card_id);
    set(&mut card.priority, &c.priority);
    set(&mut card.value, &c.value);
    set(&mut card.due_date, &c.due_date);
    let l = &patch.status_labels;
    set(&mut out.status_labels.open, &l.open);
    set(&mut out.status_labels.won, &l.won);
    set(&mut out.status_labels.lost, &l.lost);
    out
}

/// `defaultSettingsFor`: the defaults with the type's status labels.
pub fn default_settings_for(pipeline_type: PipelineType) -> PipelineSettings {
    let (open, won, lost) = pipeline_type_defaults(pipeline_type).status_labels;
    PipelineSettings {
        status_labels: PipelineStatusLabels {
            open: open.to_string(),
            won: won.to_string(),
            lost: lost.to_string(),
        },
        ..PipelineSettings::default()
    }
}
