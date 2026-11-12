//! `pipeline-variable.ts`: `{{pipelines.<reference_name>.<attribute>}}` of
//! the contact's latest open (else latest) item in that pipeline.

use std::sync::OnceLock;

use regex::Regex;

use super::sources::VariableSources;
use super::text::format_date;
use super::{PipelineVariableTarget, VariableContext, VariableError, Variables};
use crate::utils::money::{FormatMoneyOptions, SymbolPosition, format_money};

/// `PIPELINE_VARIABLE_PREFIX`.
pub const PIPELINE_VARIABLE_PREFIX: &str = "pipelines.";

const ASSIGNED_ADMINS_SEPARATOR: &str = ", ";

string_enum! {
    /// `PIPELINE_VARIABLE_ATTRIBUTES`.
    pub enum PipelineVariableAttribute {
        Name = "name",
        StageName = "stage_name",
        Status = "status",
        Priority = "priority",
        Value = "value",
        AssignedAdmins = "assigned_admins",
        CreatedDate = "created_date",
        DueDate = "due_date",
        UpdatedDate = "updated_date",
        Id = "id",
    }
}

/// `PIPELINE_REFERENCE_NAME_PATTERN`.
fn reference_name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[a-z][a-z0-9_]{0,63}$").expect("valid regex"))
}

/// A parsed `pipelines.<ref>.<attr>` token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineVariable {
    pub reference_name: String,
    pub attribute: PipelineVariableAttribute,
}

/// `parsePipelineVariable`: exactly `pipelines.<reference_name>.<attribute>`.
pub fn parse_pipeline_variable(variable: &str) -> Option<PipelineVariable> {
    let rest = variable.strip_prefix(PIPELINE_VARIABLE_PREFIX)?;
    let parts: Vec<&str> = rest.split('.').collect();
    let [reference_name, attribute] = parts.as_slice() else {
        return None;
    };
    if reference_name.is_empty() || !reference_name_re().is_match(reference_name) {
        return None;
    }
    Some(PipelineVariable {
        reference_name: (*reference_name).to_owned(),
        attribute: PipelineVariableAttribute::parse(attribute)?,
    })
}

/// `isPipelineVariable`: a well-formed token (the pipeline may not exist).
pub fn is_pipeline_variable(variable: &str) -> bool {
    parse_pipeline_variable(variable).is_some()
}

fn format_target_value(target: &PipelineVariableTarget) -> String {
    match target.value {
        None => String::new(),
        Some(value) => format_money(
            value,
            &FormatMoneyOptions {
                currency: &target.currency,
                symbol: None,
                position: SymbolPosition::LeftSpace,
            },
        ),
    }
}

/// `readAttribute`.
pub fn read_attribute(
    target: &PipelineVariableTarget,
    attribute: PipelineVariableAttribute,
    timezone: Option<&str>,
) -> String {
    use PipelineVariableAttribute as A;
    match attribute {
        A::Name => target.name.clone(),
        A::StageName => target.stage_name.clone(),
        A::Status => target.status_label.clone(),
        A::Priority => target.priority.clone(),
        A::Value => format_target_value(target),
        A::AssignedAdmins => target.assigned_admins.join(ASSIGNED_ADMINS_SEPARATOR),
        A::CreatedDate => format_date(Some(target.created_at), timezone).unwrap_or_default(),
        A::DueDate => format_date(target.due_date, timezone).unwrap_or_default(),
        A::UpdatedDate => format_date(Some(target.updated_at), timezone).unwrap_or_default(),
        A::Id => target.id.clone(),
    }
}

impl<S: VariableSources> Variables<'_, S> {
    /// `resolvePipelineVariable`: `""` when the pipeline does not exist or
    /// the contact has no item in it; a failed load fails the render.
    pub async fn resolve_pipeline_variable(
        &self,
        context: &VariableContext,
        variable: &str,
        timezone: Option<&str>,
    ) -> Result<String, VariableError> {
        let Some(parsed) = parse_pipeline_variable(variable) else {
            return Ok(String::new());
        };
        let Some(loader) = &context.pipeline_items else {
            return Ok(String::new());
        };
        let targets = loader.get(self.sources).await?;
        Ok(targets
            .iter()
            .find(|t| t.reference_name == parsed.reference_name)
            .map(|t| read_attribute(t, parsed.attribute, timezone))
            .unwrap_or_default())
    }
}
