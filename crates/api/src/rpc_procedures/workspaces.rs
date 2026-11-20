//! The first-channel actions that also create the workspace
//! (`/channels/create`): `createWebchatAction`
//! (`features/integration-webchat/actions/create-webchat.action.ts`) and
//! `createApiAction` (`features/integration-api/actions/create-api.action.ts`).
//! Both are `authActionClient` actions: `workspaceId` in the input is
//! optional; none = a new workspace named after the channel.

use chatbotx_business::BusinessError;
use chatbotx_business::audit::AuditRecordInput;
use chatbotx_business::integration_api::{self, CreateApiChannel};
use chatbotx_business::net::ssrf_guard::HttpDoh;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, action_fail, arg, invalid, public};
use crate::orpc::OrpcError;

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if name == "acceptInvitationAction" {
        // `features/invitations/actions/accept-invitation.ts`
        // (`authActionClient`, input `{ code }`).
        return Some(
            async {
                let code = In(arg(args, 0)).opt_str("code")?.ok_or_else(invalid)?;
                chatbotx_business::invitation::accept(
                    &ctx.api.db,
                    ctx.api.edition,
                    code,
                    ctx.user_id,
                )
                .await
                .map_err(action_fail)?;
                Ok(Value::Null)
            }
            .await,
        );
    }
    if name == "assertCurrentUserCanAccessChatbot" {
        // `lib/auth/utils.ts` (a plain "use server" function, `args: [workspaceId]`);
        // support sessions are not ported, so access = a real membership.
        return Some(
            async {
                let workspace_id = arg(args, 0)
                    .as_str()
                    .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
                    .and_then(|id| id.parse::<i64>().ok());
                match workspace_id {
                    Some(id) if is_member(ctx, id).await? => Ok(Value::Null),
                    _ => Err(action_fail(BusinessError::exception(
                        "User is not associated with this workspace",
                        "systemError",
                        400,
                    ))),
                }
            }
            .await,
        );
    }
    if !matches!(name, "createWebchatAction" | "createApiAction") {
        return None;
    }
    Some(
        async {
            let input = In(arg(args, 0));
            let channel_name = input.opt_str("name")?.ok_or_else(invalid)?;
            let (workspace_id, created) =
                resolve_workspace(ctx, input.opt_id("workspaceId")?, &channel_name).await?;
            let member = member_ctx(ctx, workspace_id).await?;
            let ctx = Ctx {
                workspace_id,
                role: &member.0,
                permissions: &member.1,
                ..*ctx
            };
            if created {
                ctx.api
                    .audit
                    .record(AuditRecordInput {
                        user_id: Some(ctx.user_id.to_string()),
                        workspace_id: Some(workspace_id.to_string()),
                        ..AuditRecordInput::new(
                            "create",
                            format!("created the workspace (#{workspace_id})"),
                        )
                    })
                    .await;
            }
            if name == "createWebchatAction" {
                let mut body = arg(args, 0).clone();
                if let Some(object) = body.as_object_mut() {
                    object.remove("workspaceId");
                    // `authorizedDomains: [{ value }]` -> `[value]`.
                    if let Some(Value::Array(domains)) = object.get_mut("authorizedDomains") {
                        for domain in domains.iter_mut() {
                            if let Some(value) = domain.get("value").cloned() {
                                *domain = value;
                            }
                        }
                    }
                }
                public(&ctx, "webchats.create", &[], None, Some(&body)).await?;
                return Ok(json!({ "workspaceId": workspace_id.to_string() }));
            }
            let callback_url = input.opt_str("callbackUrl")?.filter(|url| !url.is_empty());
            let (_, token) = integration_api::create_with_token(
                &ctx.api.db,
                CreateApiChannel {
                    workspace_id,
                    actor_user_id: Some(ctx.user_id),
                    name: channel_name,
                    callback_url,
                },
                &HttpDoh,
                ctx.now_ms,
            )
            .await
            .map_err(action_fail)?;
            Ok(json!({ "workspaceId": workspace_id.to_string(), "token": token }))
        }
        .await,
    )
}

/// The given workspace (the caller must be a member: 404 "Workspace not
/// found") or a new one owned by the caller: `(id, created)`.
async fn resolve_workspace(
    ctx: &Ctx<'_>,
    workspace_id: Option<i64>,
    name: &str,
) -> Result<(i64, bool), OrpcError> {
    let user_id = ctx.user_id;
    let name = name.to_string();
    // `workspaceService.create` takes the install intent first, so a creation
    // refused below still ends the choice.
    let intent = match (workspace_id, ctx.api.kv.as_deref()) {
        (None, Some(kv)) => {
            chatbotx_business::template::install_intent::take(&kv.durable, user_id).await
        }
        _ => None,
    };
    let resolved = ctx
        .api
        .db
        .write(move |tx| {
            if let Some(workspace_id) = workspace_id {
                let member: Option<i64> = tx
                    .query_row(
                        r#"SELECT "id" FROM "WorkspaceMember"
                           WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                        params![workspace_id, user_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                return Ok(member.map(|_| (workspace_id, false)));
            }
            let id = crate::spa::insert_workspace(tx, user_id, &name, intent.as_deref())?;
            Ok(Some((id, true)))
        })
        .await
        .map_err(|e| crate::handlers::internal_error(&e.to_string()))?
        .ok_or_else(|| {
            action_fail(BusinessError::exception(
                "Workspace not found",
                "notFound",
                404,
            ))
        })?;
    if resolved.1 {
        enqueue_staged_installs(ctx, resolved.0).await;
    }
    Ok(resolved)
}

/// Enqueues the template install staged with a new workspace (best effort:
/// the template sweep re-dispatches a miss).
async fn enqueue_staged_installs(ctx: &Ctx<'_>, workspace_id: i64) {
    if let Some(jobs) = ctx.api.jobs.as_deref() {
        chatbotx_business::template::install_intent::dispatch_staged_installs(
            &ctx.api.db,
            jobs,
            workspace_id,
        )
        .await;
    }
}

/// The caller's `(role, permissions)` in the workspace.
async fn member_ctx(ctx: &Ctx<'_>, workspace_id: i64) -> Result<(String, Value), OrpcError> {
    let user_id = ctx.user_id;
    let (role, permissions): (String, String) = ctx
        .api
        .db
        .read(move |conn| {
            Ok(conn.query_row(
                r#"SELECT "role", "permissions" FROM "WorkspaceMember"
                   WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                params![workspace_id, user_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        })
        .await
        .map_err(|e| crate::handlers::internal_error(&e.to_string()))?;
    Ok((
        role,
        serde_json::from_str(&permissions).unwrap_or(Value::Null),
    ))
}

/// Whether the caller is a member of the workspace (`hasWorkspaceAccess`).
pub(super) async fn is_member(ctx: &Ctx<'_>, workspace_id: i64) -> Result<bool, OrpcError> {
    let user_id = ctx.user_id;
    ctx.api
        .db
        .read(move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "id" FROM "WorkspaceMember" WHERE "workspaceId" = ?1 AND "userId" = ?2"#,
                    params![workspace_id, user_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .is_some())
        })
        .await
        .map_err(|e| crate::handlers::internal_error(&e.to_string()))
}
