//! Port of `packages/variables` (without `javascript-interpolation.ts`: the
//! Execute JavaScript step is dropped, DECISIONS 2026-11-11).
//!
//! | TypeScript file                       | Rust                                   |
//! |---------------------------------------|----------------------------------------|
//! | `schema.ts`                           | [`models`], [`VariableContext`]        |
//! | `utils.ts`                            | [`text`], [`Variables::get_system_field_value`] |
//! | `contact-variable.ts`                 | [`Variables::replace_all`], [`Variables::get_all`] |
//! | `deep-replace-contact-variables.ts`   | [`Variables::resolve_contact_variables_deep`] |
//! | `pipeline-variable.ts`                | [`pipeline`]                           |
//! | `bot-field-variable(-resolver).ts`    | [`bot_field`]                          |
//! | `coupon-variable.ts`                  | [`coupon`]                             |
//! | `helpers/*.ts`                        | [`helpers`], [`integration_fields`]    |
//!
//! Every database / business / provider call goes through
//! [`VariableSources`]; `withCache` is the in-process [`Cache`] of `kv`;
//! token signing is [`Encryption`]; "now" is the [`Clock`].

pub mod bot_field;
pub mod contact_variable;
pub mod coupon;
pub mod deep_replace;
pub mod helpers;
pub mod integration_fields;
pub mod models;
pub mod pipeline;
pub mod sources;
pub mod system_field;
pub mod system_fields;
pub mod text;

use std::fmt;

use chatbotx_kv::Clock;
use chatbotx_kv::cache::Cache;
use tokio::sync::OnceCell;

use crate::encryption::{Encryption, EncryptionError};
pub use models::*;
pub use sources::{
    AttachmentMediaRequest, AvatarResolution, MeSystemFieldPayload, SourceError, SourceResult,
    VariableSources,
};
pub use system_field::SystemFieldType;

/// `ReplaceVariableProps` (`ContactVariableContext` plus the field maps and
/// the pipeline loader).
#[derive(Debug, Default)]
pub struct VariableContext {
    pub contact: ContactModel,
    /// `None` when the contact has no inbox: inbox-based variables resolve empty.
    pub contact_inbox: Option<ContactInboxModel>,
    pub conversation: Option<ConversationModel>,
    pub appointment_id: Option<String>,
    /// The inbox user who sent the running flow (`{{admin_name}}`).
    pub sent_by_user_id: Option<String>,
    pub workspace: Option<WorkspaceModel>,
    pub custom_fields_map: CustomFieldsMap,
    /// `None` (as an empty map): no `bot_field:` token resolves.
    pub bot_fields_map: Option<BotFieldsMap>,
    pub pipeline_items: Option<PipelineItemsLoader>,
}

/// `PipelineItemsLoader`: the contact's pipeline items, loaded on first
/// use and shared by every later read of the same context (a failed load
/// is shared too, like the memoized rejected promise).
#[derive(Debug, Default)]
pub struct PipelineItemsLoader {
    /// `(workspace_id, contact_id)`; `None` for [`PipelineItemsLoader::preloaded`].
    scope: Option<(String, String)>,
    items: OnceCell<Result<Vec<PipelineVariableTarget>, SourceError>>,
    last_item_id: OnceCell<Result<Option<String>, SourceError>>,
}

impl PipelineItemsLoader {
    /// `createContactPipelineItemsLoader(contact)`.
    pub fn for_contact(contact: &ContactModel) -> PipelineItemsLoader {
        PipelineItemsLoader {
            scope: Some((contact.workspace_id.clone(), contact.id.clone())),
            ..PipelineItemsLoader::default()
        }
    }

    /// A loader whose items are already known (`{ get: () => Promise.resolve(items) }`).
    pub fn preloaded(items: Vec<PipelineVariableTarget>) -> PipelineItemsLoader {
        PipelineItemsLoader {
            scope: None,
            items: OnceCell::new_with(Some(Ok(items))),
            last_item_id: OnceCell::new(),
        }
    }

    /// `get()`: the latest open (else latest) item per pipeline.
    pub async fn get<S: VariableSources>(
        &self,
        sources: &S,
    ) -> Result<&[PipelineVariableTarget], SourceError> {
        let result = self
            .items
            .get_or_init(|| async {
                match &self.scope {
                    Some((workspace_id, contact_id)) => {
                        sources
                            .load_pipeline_variable_targets(workspace_id, contact_id)
                            .await
                    }
                    None => Ok(Vec::new()),
                }
            })
            .await;
        result.as_deref().map_err(Clone::clone)
    }

    /// `lastItemId()`: the contact's most recently created or updated item.
    pub async fn last_item_id<S: VariableSources>(
        &self,
        sources: &S,
    ) -> Result<Option<String>, SourceError> {
        self.last_item_id
            .get_or_init(|| async {
                match &self.scope {
                    Some((workspace_id, contact_id)) => {
                        sources
                            .load_last_pipeline_item_id(workspace_id, contact_id)
                            .await
                    }
                    None => Err(SourceError::new(
                        "pipelineItems.lastItemId is not a function",
                    )),
                }
            })
            .await
            .clone()
    }
}

/// A failed variable render or load.
#[derive(Debug)]
pub enum VariableError {
    /// A source call failed.
    Source(SourceError),
    /// A token could not be signed.
    Encryption(EncryptionError),
    /// `getAll`: `Contact not found`.
    ContactNotFound,
    /// `replaceAll`: `Unable to render custom fields to message`.
    Render(Box<VariableError>),
}

impl fmt::Display for VariableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VariableError::Source(err) => write!(f, "{err}"),
            VariableError::Encryption(err) => write!(f, "{err}"),
            VariableError::ContactNotFound => f.write_str("Contact not found"),
            VariableError::Render(_) => f.write_str("Unable to render custom fields to message"),
        }
    }
}

impl std::error::Error for VariableError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            VariableError::Source(err) => Some(err),
            VariableError::Render(err) => Some(err.as_ref()),
            _ => None,
        }
    }
}

impl From<SourceError> for VariableError {
    fn from(err: SourceError) -> VariableError {
        VariableError::Source(err)
    }
}

impl From<EncryptionError> for VariableError {
    fn from(err: EncryptionError) -> VariableError {
        VariableError::Encryption(err)
    }
}

/// The variable resolvers with their dependencies (the module-level
/// imports of the original).
pub struct Variables<'a, S> {
    pub sources: &'a S,
    /// `withCache` (`@chatbotx.io/redis`).
    pub cache: &'a Cache,
    /// Token signing (`ENCRYPTION_KEY`).
    pub encryption: &'a Encryption,
    /// `new Date()`.
    pub clock: &'a dyn Clock,
}

impl<'a, S: VariableSources> Variables<'a, S> {
    pub fn new(
        sources: &'a S,
        cache: &'a Cache,
        encryption: &'a Encryption,
        clock: &'a dyn Clock,
    ) -> Variables<'a, S> {
        Variables {
            sources,
            cache,
            encryption,
            clock,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The server runs renders on a multi-threaded runtime: the futures
    /// must be `Send` for any `Send` sources.
    #[allow(dead_code)]
    fn renders_are_send<S: VariableSources>(v: &Variables<'_, S>, ctx: &VariableContext) {
        fn is_send<T: Send>(_: T) {}
        is_send(v.replace_all("{{first_name}}", ctx, None));
        is_send(v.get_all(contact_variable::GetAllInput::default()));
        is_send(v.resolve_contact_variables_deep(
            "c",
            &serde_json::Value::Null,
            deep_replace::DeepSource::default(),
            deep_replace::DeepOptions::default(),
        ));
        is_send(v.get_system_field_value(ctx, SystemFieldType::Me));
    }
}
