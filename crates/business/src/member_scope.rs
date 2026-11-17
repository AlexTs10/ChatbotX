//! Member scoping (Q.A1 §3): `conversation/member-scope.ts`
//! (`conversationMemberScope`), `contact/permission-scope.ts`
//! (`buildContactPermissionScope`) and `flowService.assertEnabledInInbox`.

use chatbotx_db::Db;
use serde_json::Value;

use crate::errors::Result;
use crate::tx;
use crate::workspace_member::{has_contacts_access, has_permission};

/// `ConversationMemberScope.chatScope`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatScope {
    All,
    Assigned,
    None,
}

/// `ConversationMemberScope`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationMemberScope {
    pub chat_scope: ChatScope,
    pub team_ids: Vec<i64>,
    pub user_id: i64,
}

/// `conversationMemberScope.resolve`: `none` without contacts access, `all`
/// for a super admin or a member without `onlyAssignedContacts`, else
/// `assigned` with the member's inbox teams.
pub async fn resolve_conversation_scope(
    db: &Db,
    workspace_id: i64,
    user_id: i64,
    permissions: &Value,
) -> Result<ConversationMemberScope> {
    let scope = |chat_scope, team_ids| ConversationMemberScope {
        chat_scope,
        team_ids,
        user_id,
    };
    if !has_contacts_access(permissions) {
        return Ok(scope(ChatScope::None, Vec::new()));
    }
    if permissions.get("superAdmin") == Some(&Value::Bool(true))
        || permissions.get("onlyAssignedContacts") != Some(&Value::Bool(true))
    {
        return Ok(scope(ChatScope::All, Vec::new()));
    }
    let team_ids = tx::read(db, move |conn| {
        let mut stmt = conn.prepare_cached(
            r#"SELECT m."inboxTeamId" FROM "InboxTeamMember" m
               JOIN "InboxTeam" t ON t."id" = m."inboxTeamId"
               WHERE t."workspaceId" = ?1 AND m."userId" = ?2"#,
        )?;
        let ids = stmt
            .query_map([workspace_id, user_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<i64>, _>>()?;
        Ok(ids)
    })
    .await?;
    Ok(scope(ChatScope::Assigned, team_ids))
}

impl ConversationMemberScope {
    /// `conversationMemberScope.allows` for a conversation's assignment.
    pub fn allows(&self, assigned_user_id: Option<i64>, assigned_team_id: Option<i64>) -> bool {
        match self.chat_scope {
            ChatScope::All => true,
            ChatScope::None => false,
            ChatScope::Assigned => {
                assigned_user_id == Some(self.user_id)
                    || assigned_team_id.is_some_and(|team| self.team_ids.contains(&team))
            }
        }
    }
}

/// `ContactPermissionScope` (+ the `restrictToTeamIds` of `contactScopeForCaller`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactPermissionScope {
    pub can_view_email_and_phone: bool,
    pub restrict_to_assigned_user_id: Option<i64>,
    pub restrict_to_team_ids: Option<Vec<i64>>,
}

/// `buildContactPermissionScope` + `contactScopeForCaller`: `None` without
/// contacts access (the caller answers 404 "Contact not found").
pub fn contact_scope(
    permissions: &Value,
    user_id: i64,
    conversation: &ConversationMemberScope,
) -> Option<ContactPermissionScope> {
    if !has_contacts_access(permissions) {
        return None;
    }
    let restrict_to_assigned_user_id = (!has_permission(permissions, "superAdmin")
        && has_permission(permissions, "onlyAssignedContacts"))
    .then_some(user_id);
    Some(ContactPermissionScope {
        can_view_email_and_phone: has_permission(permissions, "emailAndPhone"),
        restrict_to_assigned_user_id,
        restrict_to_team_ids: (conversation.chat_scope == ChatScope::Assigned)
            .then(|| conversation.team_ids.clone()),
    })
}

/// `flowService.assertEnabledInInbox`: `false` = 404 "Flow not found".
pub async fn flow_enabled_in_inbox(db: &Db, workspace_id: i64, flow_id: i64) -> Result<bool> {
    tx::read(db, move |conn| {
        use chatbotx_db::rusqlite::OptionalExtension as _;
        let enabled: Option<bool> = conn
            .query_row(
                r#"SELECT "enableInInbox" FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                [flow_id, workspace_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(enabled == Some(true))
    })
    .await
}

/// `contactService.findByIdOrFail` with an `accessScope`: the contact is in
/// the workspace and, when restricted, has a conversation assigned to the
/// member or to one of the member's teams (`buildContactListWhere`).
pub async fn contact_visible(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    scope: &ContactPermissionScope,
) -> Result<bool> {
    let user = scope.restrict_to_assigned_user_id;
    let teams = scope.restrict_to_team_ids.clone().unwrap_or_default();
    tx::read(db, move |conn| {
        let teams_json = serde_json::to_string(&teams).unwrap_or_else(|_| "[]".into());
        let restricted = user.is_some() || !teams.is_empty();
        let found: bool = conn.query_row(
            r#"SELECT EXISTS (SELECT 1 FROM "Contact" c WHERE c."id" = ?1 AND c."workspaceId" = ?2
                 AND (?3 = 0 OR EXISTS (SELECT 1 FROM "Conversation" v WHERE v."contactId" = c."id"
                   AND (v."assignedUserId" = ?4
                        OR v."assignedInboxTeamId" IN (SELECT value FROM json_each(?5))))))"#,
            chatbotx_db::rusqlite::params![contact_id, workspace_id, restricted, user, teams_json],
            |row| row.get(0),
        )?;
        Ok(found)
    })
    .await
}
