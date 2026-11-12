//! `coupon-variable.ts`: `{{coupon:<topicId>}}` issues the contact a code.

use super::sources::VariableSources;
use super::{VariableContext, VariableError, Variables};
use crate::js;

/// `COUPON_VARIABLE_PREFIX`.
pub const COUPON_VARIABLE_PREFIX: &str = "coupon:";

/// `isCouponVariable`.
pub fn is_coupon_variable(variable: &str) -> bool {
    variable.starts_with(COUPON_VARIABLE_PREFIX)
}

/// `getCouponVariableTopicId`: the trimmed topic id, `None` when blank.
pub fn get_coupon_variable_topic_id(variable: &str) -> Option<String> {
    let topic = js::trim(variable.strip_prefix(COUPON_VARIABLE_PREFIX)?);
    (!topic.is_empty()).then(|| topic.to_owned())
}

impl<S: VariableSources> Variables<'_, S> {
    /// `resolveCouponVariable`: `""` without a topic, contact or workspace.
    pub async fn resolve_coupon_variable(
        &self,
        variables: &VariableContext,
        variable: &str,
    ) -> Result<String, VariableError> {
        let Some(topic_id) = get_coupon_variable_topic_id(variable) else {
            return Ok(String::new());
        };
        let contact_id = variables.contact.id.as_str();
        let workspace_id = variables.workspace.as_ref().map_or("", |w| w.id.as_str());
        if contact_id.is_empty() || workspace_id.is_empty() {
            return Ok(String::new());
        }
        Ok(self
            .sources
            .resolve_coupon_variable(workspace_id, contact_id, &topic_id)
            .await?)
    }
}
