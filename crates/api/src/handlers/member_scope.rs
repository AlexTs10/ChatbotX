//! `lib/public-api/member-scope.ts` (Q.A1 §3): member scoping for the `/v1`
//! handlers of [`crate::auth::MOBILE_MEMBER_ROUTES`]. A workspace token or
//! MCP OAuth has no mobile member ([`super::mobile_member`] is `None`) and
//! every helper keeps the unscoped behaviour.

use chatbotx_business::BusinessError;
use chatbotx_business::member_scope::{self, ConversationMemberScope};
use chatbotx_business::workspace_member::has_permission;
use chatbotx_db::Db;
use serde_json::Value;

use super::{ErrorMap, business_error, mobile_member};
use crate::orpc::OrpcError;

/// `conversationScopeForCaller`.
pub fn conversation_scope() -> Option<ConversationMemberScope> {
    mobile_member().map(|m| m.conversation.clone())
}

/// `assertConversationAccessForCaller` after the handler's own
/// `findByOrFail`: a conversation outside the member's chat scope is the
/// same 404 as a missing one.
pub fn assert_conversation_allowed(
    assigned_user_id: Option<i64>,
    assigned_team_id: Option<i64>,
    errors: ErrorMap,
) -> Result<(), OrpcError> {
    match conversation_scope() {
        Some(scope) if !scope.allows(assigned_user_id, assigned_team_id) => Err(business_error(
            BusinessError::not_found("Conversation not found"),
            errors,
        )),
        _ => Ok(()),
    }
}

/// [`assert_conversation_allowed`] for a conversation id (loads its assignment).
pub async fn assert_conversation_access(
    db: &Db,
    workspace_id: i64,
    conversation_id: i64,
    errors: ErrorMap,
) -> Result<(), OrpcError> {
    if mobile_member().is_none() {
        return Ok(());
    }
    let conversation =
        chatbotx_business::conversation_list::find_or_fail(db, workspace_id, conversation_id)
            .await
            .map_err(|e| business_error(e, errors))?;
    assert_conversation_allowed(
        conversation.assigned_user_id,
        conversation.assigned_inbox_team_id,
        errors,
    )
}

/// `contactScopeForCaller`: `None` = unscoped (a token); a member without
/// contacts access is 404 "Contact not found".
pub fn contact_scope(
    errors: ErrorMap,
) -> Result<Option<member_scope::ContactPermissionScope>, OrpcError> {
    let Some(member) = mobile_member() else {
        return Ok(None);
    };
    member_scope::contact_scope(&member.permissions, member.user_id, &member.conversation)
        .map(Some)
        .ok_or_else(|| business_error(BusinessError::not_found("Contact not found"), errors))
}

/// The member half of `contactIdFor`: after the handler resolved the
/// identifier, proves the member may see the contact (else the same 404 a
/// missing contact gets, so existence is never exposed).
pub async fn assert_contact_access(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    errors: ErrorMap,
) -> Result<(), OrpcError> {
    let Some(scope) = contact_scope(errors)? else {
        return Ok(());
    };
    let visible = member_scope::contact_visible(db, workspace_id, contact_id, &scope)
        .await
        .map_err(|e| business_error(e, errors))?;
    if visible {
        Ok(())
    } else {
        Err(business_error(
            BusinessError::not_found("Contact not found"),
            errors,
        ))
    }
}

/// `includeEmailAndPhone` of a caller: a token, or a member with `emailAndPhone`.
pub fn can_view_email_and_phone() -> bool {
    mobile_member().is_none_or(|m| has_permission(&m.permissions, "emailAndPhone"))
}

/// `maskContactForCaller`: `email` / `phoneNumber` become `null` for a
/// member without `emailAndPhone`.
pub fn mask_contact(contact: &mut Value) {
    if can_view_email_and_phone() {
        return;
    }
    if let Value::Object(map) = contact {
        map.insert("email".into(), Value::Null);
        map.insert("phoneNumber".into(), Value::Null);
    }
}

/// `resolveSenderForCaller`: the mobile user for outgoing message attribution.
pub fn sender_for_caller() -> Option<i64> {
    mobile_member().map(|m| m.user_id)
}

/// `assertMemberMayStartFlow`: a member without `flows` may only start a
/// flow enabled in the inbox (else 404 "Flow not found").
pub async fn assert_member_may_start_flow(
    db: &Db,
    workspace_id: i64,
    flow_id: Option<i64>,
    errors: ErrorMap,
) -> Result<(), OrpcError> {
    let (Some(flow_id), Some(member)) = (flow_id, mobile_member()) else {
        return Ok(());
    };
    if has_permission(&member.permissions, "flows") {
        return Ok(());
    }
    let enabled = member_scope::flow_enabled_in_inbox(db, workspace_id, flow_id)
        .await
        .map_err(|e| business_error(e, errors))?;
    if enabled {
        Ok(())
    } else {
        Err(business_error(
            BusinessError::not_found("Flow not found"),
            errors,
        ))
    }
}
