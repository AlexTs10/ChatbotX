//! `contact-variable.ts`: `contactVariableService.getAll` (load a contact's
//! variable context) and `replaceAll` (render `{{variables}}` in a text).

use std::collections::HashMap;

use super::bot_field::{FieldReference, parse_field_reference};
use super::coupon::is_coupon_variable;
use super::pipeline::is_pipeline_variable;
use super::sources::VariableSources;
use super::system_field::SystemFieldType;
use super::text::{
    RAW_CUSTOM_FIELD_VARIABLE_PREFIX, extract_variables, get_contact_timezone, interpolate,
    render_custom_field_value, to_raw_custom_field_name,
};
use super::{
    ContactCustomFieldValue, ContactInboxModel, ContactModel, ConversationModel,
    PipelineItemsLoader, VariableContext, VariableError, Variables, WorkspaceModel,
};

/// The `contactInbox` of `getAll`: a loaded row, an id to load, or none.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum ContactInboxArg {
    Model(ContactInboxModel),
    Id(String),
    #[default]
    None,
}

/// `GetAllProps`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GetAllInput {
    pub contact_id: String,
    pub contact_inbox: ContactInboxArg,
    pub conversation: Option<ConversationModel>,
    pub appointment_id: Option<String>,
    pub sent_by_user_id: Option<String>,
    /// Skips the workspace query when given.
    pub workspace: Option<WorkspaceModel>,
}

/// The resolvers of `replaceAll`, in match order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolver {
    SystemField(SystemFieldType),
    RawCustomField,
    BotField,
    CustomField,
    Pipeline,
    Coupon,
}

fn find_resolver(variable: &str, context: &VariableContext) -> Option<Resolver> {
    if let Some(key) = SystemFieldType::parse(variable) {
        return Some(Resolver::SystemField(key));
    }
    if variable.starts_with(RAW_CUSTOM_FIELD_VARIABLE_PREFIX)
        && context
            .custom_fields_map
            .contains_key(to_raw_custom_field_name(variable))
    {
        return Some(Resolver::RawCustomField);
    }
    if let FieldReference::BotField { id } = parse_field_reference(variable) {
        if context
            .bot_fields_map
            .as_ref()
            .is_some_and(|map| map.contains_key(&id))
        {
            return Some(Resolver::BotField);
        }
    }
    if context.custom_fields_map.contains_key(variable) {
        return Some(Resolver::CustomField);
    }
    if is_pipeline_variable(variable) {
        return Some(Resolver::Pipeline);
    }
    if is_coupon_variable(variable) {
        return Some(Resolver::Coupon);
    }
    None
}

impl<S: VariableSources> Variables<'_, S> {
    async fn resolve_variable(
        &self,
        resolver: Resolver,
        variable: &str,
        context: &VariableContext,
        timezone: &str,
    ) -> Result<String, VariableError> {
        Ok(match resolver {
            Resolver::SystemField(key) => self
                .get_system_field_value(context, key)
                .await?
                .unwrap_or_default(),
            Resolver::RawCustomField => context
                .custom_fields_map
                .get(to_raw_custom_field_name(variable))
                .and_then(|field| field.value.clone())
                .unwrap_or_default(),
            Resolver::BotField => match parse_field_reference(variable) {
                FieldReference::BotField { id } => context
                    .bot_fields_map
                    .as_ref()
                    .and_then(|map| map.get(&id))
                    .map(|field| {
                        render_custom_field_value(
                            &field.kind,
                            Some(field.value.as_deref().unwrap_or("")),
                            Some(timezone),
                        )
                    })
                    .unwrap_or_default(),
                FieldReference::CustomField { .. } => String::new(),
            },
            Resolver::CustomField => context
                .custom_fields_map
                .get(variable)
                .map(|field| {
                    render_custom_field_value(&field.kind, field.value.as_deref(), Some(timezone))
                })
                .unwrap_or_default(),
            Resolver::Pipeline => {
                self.resolve_pipeline_variable(context, variable, Some(timezone))
                    .await?
            }
            Resolver::Coupon => self.resolve_coupon_variable(context, variable).await?,
        })
    }

    /// `contactVariableService.replaceAll`: renders every known
    /// `{{variable}}` of `text` (unknown ones stay literal). Temporal custom
    /// fields render in the contact's zone (then the workspace's, then
    /// UTC); `{{gender}}` is cased by position. `escape_value` escapes each
    /// resolved value. Any failure is logged and returned as
    /// [`VariableError::Render`].
    pub async fn replace_all(
        &self,
        text: &str,
        variables: &VariableContext,
        escape_value: Option<&(dyn Fn(&str) -> String + Sync)>,
    ) -> Result<String, VariableError> {
        let render_timezone = get_contact_timezone(variables);
        let mut mapping: HashMap<String, String> = HashMap::new();
        for variable in extract_variables(text) {
            let Some(resolver) = find_resolver(&variable, variables) else {
                continue;
            };
            match self
                .resolve_variable(resolver, &variable, variables, &render_timezone)
                .await
            {
                Ok(value) => {
                    let value = match escape_value {
                        Some(escape) => escape(&value),
                        None => value,
                    };
                    mapping.insert(variable, value);
                }
                Err(err) => {
                    tracing::error!(err = %err, "Unable to render custom fields to message");
                    return Err(VariableError::Render(Box::new(err)));
                }
            }
        }
        // Prose: "Anh vui lòng…" opening a sentence, "Xin chào anh" inside one.
        Ok(interpolate(text, &mapping, true))
    }

    async fn load_contact(&self, contact_id: &str) -> Result<ContactModel, VariableError> {
        match self.sources.find_contact(contact_id).await? {
            Some(contact) => Ok(contact),
            None => {
                tracing::error!("Contact {contact_id} not found");
                Err(VariableError::ContactNotFound)
            }
        }
    }

    async fn load_inbox(
        &self,
        arg: &ContactInboxArg,
    ) -> Result<Option<ContactInboxModel>, VariableError> {
        Ok(match arg {
            ContactInboxArg::Model(model) => Some(model.clone()),
            ContactInboxArg::Id(id) => self.sources.find_contact_inbox(id).await?,
            ContactInboxArg::None => None,
        })
    }

    async fn load_fields(
        &self,
        contact_id: &str,
    ) -> Result<HashMap<String, ContactCustomFieldValue>, VariableError> {
        let rows = self.sources.list_contact_custom_fields(contact_id).await?;
        Ok(rows.into_iter().map(|row| (row.key.clone(), row)).collect())
    }

    /// `contactVariableService.getAll`: the contact, its inbox, custom and
    /// bot fields, the workspace and a lazy pipeline loader.
    pub async fn get_all(&self, input: GetAllInput) -> Result<VariableContext, VariableError> {
        let (contact, contact_inbox, custom_fields_map) = tokio::join!(
            self.load_contact(&input.contact_id),
            self.load_inbox(&input.contact_inbox),
            self.load_fields(&input.contact_id),
        );
        let contact = contact?;
        let (contact_inbox, custom_fields_map) = (contact_inbox?, custom_fields_map?);
        let load_workspace = async {
            match input.workspace {
                Some(workspace) => Ok::<_, VariableError>(Some(workspace)),
                None => Ok(self.sources.find_workspace(&contact.workspace_id).await?),
            }
        };
        let (workspace, bot_fields_map) =
            tokio::join!(load_workspace, self.load_bot_fields(&contact.workspace_id));
        let pipeline_items = PipelineItemsLoader::for_contact(&contact);
        Ok(VariableContext {
            contact_inbox,
            conversation: input.conversation,
            appointment_id: input.appointment_id,
            sent_by_user_id: input.sent_by_user_id,
            custom_fields_map,
            bot_fields_map: Some(bot_fields_map?),
            workspace: workspace?,
            pipeline_items: Some(pipeline_items),
            contact,
        })
    }
}
