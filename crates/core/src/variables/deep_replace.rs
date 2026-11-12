//! `deep-replace-contact-variables.ts`: `{{variables}}` (and, opted in,
//! spintax) in every string leaf of a JSON structure.

use serde_json::Value;

use super::contact_variable::{ContactInboxArg, GetAllInput};
use super::sources::VariableSources;
use super::{ConversationModel, VariableContext, VariableError, Variables, WorkspaceModel};
use crate::utils::spintax::{apply_spintax, apply_spintax_with, contains_spintax};

/// Sync tree walk: `true` if any string leaf satisfies `predicate`.
fn some_string(value: &Value, predicate: &dyn Fn(&str) -> bool) -> bool {
    match value {
        Value::String(text) => predicate(text),
        Value::Array(items) => items.iter().any(|item| some_string(item, predicate)),
        Value::Object(map) => map.values().any(|item| some_string(item, predicate)),
        _ => false,
    }
}

/// `valueContainsVariablePlaceholder`: any string leaf contains `{{`.
pub fn value_contains_variable_placeholder(value: &Value) -> bool {
    some_string(value, &|text| text.contains("{{"))
}

/// The `source` of `resolveContactVariablesDeep`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeepSource {
    pub contact_inbox: ContactInboxArg,
    pub conversation: Option<ConversationModel>,
    pub workspace: Option<WorkspaceModel>,
    pub appointment_id: Option<String>,
    pub sent_by_user_id: Option<String>,
}

/// `ResolveContactVariablesDeepOptions`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeepOptions {
    /// Also resolve `{a|b|c}` spintax in every string leaf, before the
    /// variable pass (only for structures known to be prose).
    pub spintax: bool,
    /// The branch picker (`Math.random` in the original); `None` = random.
    pub pick: Option<fn(usize) -> usize>,
}

/// Every string leaf, in walk order (array order, then object key order).
fn collect_strings<'v>(value: &'v Value, out: &mut Vec<&'v str>) {
    match value {
        Value::String(text) => out.push(text),
        Value::Array(items) => items.iter().for_each(|item| collect_strings(item, out)),
        Value::Object(map) => map.values().for_each(|item| collect_strings(item, out)),
        _ => {}
    }
}

/// A copy of `value` with its string leaves taken from `replacements`, in
/// the order of [`collect_strings`].
fn rebuild(value: &Value, replacements: &mut std::vec::IntoIter<String>) -> Value {
    match value {
        Value::String(_) => Value::String(replacements.next().unwrap_or_default()),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| rebuild(item, replacements))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), rebuild(item, replacements)))
                .collect(),
        ),
        other => other.clone(),
    }
}

impl<S: VariableSources> Variables<'_, S> {
    /// `resolveContactVariablesDeep`: the structure with every `{{var}}`
    /// replaced. Contact data loads once, only when a placeholder exists;
    /// without placeholders (and spintax) the input comes back unchanged.
    /// Spintax runs first, so substituted contact data is never spun.
    pub async fn resolve_contact_variables_deep(
        &self,
        contact_id: &str,
        value: &Value,
        source: DeepSource,
        options: DeepOptions,
    ) -> Result<Value, VariableError> {
        let has_placeholder = value_contains_variable_placeholder(value);
        if !(has_placeholder || (options.spintax && some_string(value, &contains_spintax))) {
            return Ok(value.clone());
        }
        let variables: Option<VariableContext> = if has_placeholder {
            Some(
                self.get_all(GetAllInput {
                    contact_id: contact_id.to_owned(),
                    contact_inbox: source.contact_inbox,
                    conversation: source.conversation,
                    appointment_id: source.appointment_id,
                    sent_by_user_id: source.sent_by_user_id,
                    workspace: source.workspace,
                })
                .await?,
            )
        } else {
            None
        };
        let mut leaves = Vec::new();
        collect_strings(value, &mut leaves);
        let spun: Vec<String> = leaves
            .into_iter()
            .map(|text| match (options.spintax, options.pick) {
                (false, _) => text.to_owned(),
                (true, Some(pick)) => apply_spintax_with(text, &mut |n| pick(n)),
                (true, None) => apply_spintax(text),
            })
            .collect();
        let mut resolved = Vec::with_capacity(spun.len());
        for text in spun {
            match &variables {
                Some(variables) if text.contains("{{") => {
                    resolved.push(self.replace_all(&text, variables, None).await?);
                }
                _ => resolved.push(text),
            }
        }
        Ok(rebuild(value, &mut resolved.into_iter()))
    }
}
