//! `enterprise/features/inbox-teams/api/public.ts` (`inboxTeamsPublicRouter`,
//! scope `inbox`; Q.D11 slice 10), plus the JSON and side effects the
//! `/rpc` procedure and actions share.
//!
//! ponytail: no `assertWorkspaceNotBlocked` call: the workspace-token gate
//! already refuses POST / PATCH for a blocked owner (`isCloud()` only).

use chatbotx_business::inbox_team::{self as service, Mutation, TeamMember, TeamWithMembers};
use chatbotx_db::repo::models::InboxTeam;
use chatbotx_rt::{RevokeArgs, RevokeReason};
use serde_json::{Value, json};

use super::input::{Input, Str};
use super::qr_codes::public_paging;
use super::tags::{array, digits_at};
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Reply, business_error, date, id, json,
    no_content,
};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "inboxTeams.list",
    "inboxTeams.create",
    "inboxTeams.get",
    "inboxTeams.update",
    "inboxTeams.delete",
    "inboxTeams.addMembers",
    "inboxTeams.removeMembers",
];

/// `inboxTeamResource`.
pub fn team_json(team: &InboxTeam) -> Value {
    json!({
        "id": id(team.id),
        "createdAt": date(team.created_at),
        "updatedAt": date(team.updated_at),
        "workspaceId": id(team.workspace_id),
        "name": team.name,
    })
}

fn member_json(member: &TeamMember) -> Value {
    let user = &member.user;
    json!({
        "id": id(member.id),
        "createdAt": date(member.created_at),
        "updatedAt": date(member.updated_at),
        "inboxTeamId": id(member.inbox_team_id),
        "userId": id(member.user_id),
        "user": {
            "id": id(user.id),
            "createdAt": date(user.created_at),
            "updatedAt": date(user.updated_at),
            "name": user.name,
            "email": user.email,
            "emailVerified": user.email_verified,
            "image": user.image,
            "isAnonymous": user.is_anonymous,
            "mustChangePassword": user.must_change_password,
            "tenantId": id(user.tenant_id),
        },
    })
}

/// `listInboxTeamsResponse` item.
pub fn team_with_members_json(row: &TeamWithMembers) -> Value {
    let mut out = team_json(&row.team);
    out["inboxTeamMembers"] = row.members.iter().map(member_json).collect();
    out
}

/// The realtime revocations (reason `reauth`) and the audit of a mutation.
pub async fn after_mutation(
    state: &ApiState,
    workspace_id: i64,
    revoke: &[i64],
    audit: Option<(&str, &str)>,
) {
    if let Some(realtime) = &state.realtime {
        for user_id in revoke {
            realtime
                .try_revoke_member(
                    RevokeArgs {
                        workspace_id: workspace_id.to_string(),
                        user_id: user_id.to_string(),
                        reason: RevokeReason::Reauth,
                        revoked_at_ms: None,
                    },
                    "Failed to revoke inbox team member realtime connections",
                )
                .await;
        }
    }
    if let Some((action, detail)) = audit {
        state.record_audit(action, detail).await;
    }
}

async fn finish(state: &ApiState, workspace_id: i64, out: Mutation, status: u16) -> Reply {
    let audit = out.audit.as_ref().map(|(a, d)| (*a, d.as_str()));
    after_mutation(state, workspace_id, &out.revoke_user_ids, audit).await;
    Ok(json(status, team_json(&out.team)))
}

/// `z.array(zodBigintAsString())` at `userIds`.
pub fn user_ids(input: &mut Input) -> Option<Vec<i64>> {
    array(input, "userIds", 0, usize::MAX, |input, path, value| {
        digits_at(input, path, Some(value), true)
    })
}

/// `z.string().trim().min(1).max(255)`.
pub const TEAM_NAME: Str = Str::trimmed(1, 255);

fn team_id(input: &mut Input) -> Option<i64> {
    let value = input.get("id").cloned();
    digits_at(input, &super::input::key("id"), value.as_ref(), true)
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "inboxTeams.list" => {
            let mut input = Input::from_query(call.params, call.query);
            let (page, per_page) = public_paging(&mut input);
            input.finish()?;
            let teams = service::list_by_workspace(db, ws)
                .await
                .map_err(|e| business_error(e, LISTING))?;
            // `paginateInMemory`.
            let start = ((page - 1) * per_page).max(0) as usize;
            let data: Vec<Value> = teams
                .iter()
                .skip(start)
                .take(per_page as usize)
                .map(team_with_members_json)
                .collect();
            let page_count = ((teams.len() as i64 + per_page - 1) / per_page).max(1);
            Ok(json(200, json!({ "data": data, "pageCount": page_count })))
        }
        "inboxTeams.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let team = team_id(&mut input);
            input.finish()?;
            service::find_by_id_or_fail(db, ws, team.unwrap_or_default())
                .await
                .map(|team| json(200, team_json(&team)))
                .map_err(|e| business_error(e, FINDING))
        }
        "inboxTeams.create" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.string("name", TEAM_NAME);
            let users = user_ids(&mut input);
            input.finish()?;
            let out = service::create(db, ws, name.unwrap_or_default(), users.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, CREATING))?;
            finish(state, ws, out, 201).await
        }
        "inboxTeams.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let name = input.opt_string("name", TEAM_NAME);
            let team = team_id(&mut input);
            input.finish()?;
            let out = service::update(db, ws, team.unwrap_or_default(), name)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            finish(state, ws, out, 200).await
        }
        "inboxTeams.delete" => {
            let mut input = Input::from_query(call.params, call.query);
            let team = team_id(&mut input);
            input.finish()?;
            let (revoke, detail) = service::delete(db, ws, vec![team.unwrap_or_default()])
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            after_mutation(state, ws, &revoke, Some(("delete", detail.as_str()))).await;
            Ok(no_content())
        }
        "inboxTeams.addMembers" | "inboxTeams.removeMembers" => {
            let mut input = Input::from_body(call.params, call.body);
            let users = user_ids(&mut input);
            let team = team_id(&mut input);
            input.finish()?;
            let (team, users) = (team.unwrap_or_default(), users.unwrap_or_default());
            let out = if route_id == "inboxTeams.addMembers" {
                service::add_members(db, ws, team, users).await
            } else {
                service::remove_members(db, ws, team, users).await
            }
            .map_err(|e| business_error(e, MUTATING))?;
            finish(state, ws, out, 200).await
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
