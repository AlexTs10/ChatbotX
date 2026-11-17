//! The server-component queries of the settings data pages (Q.F2 web run 7),
//! as `POST /rpc/actions/<name>`; `workspaceId` made the `/rpc` membership
//! check run (`assertCurrentUserCanAccessChatbot` /
//! `getCurrentUserAndTargetWorkspace`):
//!
//! - `listWorkspaceTokens`: `workspaceApiTokenService.listTokens` +
//!   `toWorkspaceApiTokenDto` (`settings/integrations/workspace-token`)
//! - `listErrorLogs`: `@chatbotx.io/business/error-log` `listErrorLogs`
//!   (`/space/$id/error-logs`)
//! - `listAuditLogs`, `listAuditLogAdmins`: `enterprise/features/audit-logs`
//!   (`assertEnterpriseFeatures` + `assertWorkspaceSuperAdmin`)

use chatbotx_business::BusinessError;
use chatbotx_business::audit::log_query::{self, ListAuditLogsInput};
use chatbotx_business::error_log::{self, ListInput};
use chatbotx_business::workspace_api_token;
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, arg, fail, page};
use crate::handlers::{date, id};
use crate::orpc::OrpcError;

pub(super) const ACTIONS: &[&str] = &[
    "listWorkspaceTokens",
    "listErrorLogs",
    "listAuditLogs",
    "listAuditLogAdmins",
];

/// `errors.superAdminRequired` (en).
const SUPER_ADMIN_REQUIRED: &str = "You need to be a workspace super admin to do this.";

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if !ACTIONS.contains(&name) {
        return None;
    }
    Some(run(ctx, name, args).await)
}

async fn run(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let db = &ctx.api.db;
    let ws = ctx.workspace_id;
    let input = arg(args, 0);
    match name {
        "listWorkspaceTokens" => {
            let tokens = workspace_api_token::list_tokens(db, ws)
                .await
                .map_err(fail)?;
            // `toWorkspaceApiTokenDto`: never `tokenHash` / `encryptedToken`.
            Ok(tokens
                .iter()
                .map(|t| {
                    json!({
                        "id": id(t.id),
                        "name": t.name,
                        "permission": t.permission,
                        "tokenPrefix": t.token_prefix,
                        "isDefault": t.is_default,
                        "createdAt": date(t.created_at),
                        "scopes": t.scopes,
                    })
                })
                .collect())
        }
        "listErrorLogs" => {
            let input = In(input);
            let listed = error_log::list(
                db,
                ListInput {
                    workspace_id: ws,
                    page: input.opt_positive("page")?,
                    per_page: input.opt_positive("perPage")?,
                    sort: input.sort(),
                    keyword: input.opt_str("keyword")?,
                },
            )
            .await
            .map_err(fail)?;
            let data = listed.data.into_iter().map(|row| {
                let mut out = row.log.to_json();
                if let Some(object) = out.as_object_mut() {
                    // `withheldErrorLogColumns(true)`.
                    object.remove("sourceId");
                    object.remove("stackTrace");
                    object.insert(
                        "contact".into(),
                        row.contact.map_or(Value::Null, |contact| {
                            let mut contact = contact.to_json();
                            contact["conversation"] = row
                                .conversation_id
                                .map_or(Value::Null, |c| json!({ "id": id(c) }));
                            contact
                        }),
                    );
                }
                out
            });
            Ok(page(data, listed.page_count))
        }
        "listAuditLogs" => {
            assert_super_admin(ctx).await?;
            let input = In(input);
            let user_id = match input.opt_str("userId")? {
                Some(u) if !u.is_empty() => Some(u.parse::<i64>().unwrap_or(-1)),
                _ => None,
            };
            let listed = log_query::list_logs(
                db,
                ListAuditLogsInput {
                    workspace_id: ws,
                    page: input.opt_positive("page")?,
                    per_page: input.opt_positive("perPage")?,
                    sort: input.sort(),
                    keyword: input.opt_str("keyword")?,
                    user_id,
                    from: input.opt_str("from")?.unwrap_or_default(),
                    to: input.opt_str("to")?.unwrap_or_default(),
                },
            )
            .await
            .map_err(fail)?;
            let data = listed.data.into_iter().map(|row| {
                let mut out = row.log.to_json();
                out["user"] = row.user.map_or(Value::Null, |u| u.to_json());
                out
            });
            Ok(page(data, listed.page_count))
        }
        "listAuditLogAdmins" => {
            assert_super_admin(ctx).await?;
            let admins = chatbotx_business::tx::read(db, move |conn| {
                let mut stmt = conn
                    .prepare(
                        r#"SELECT u."id", u."name", u."email", m."permissions"
                           FROM "WorkspaceMember" m JOIN "User" u ON u."id" = m."userId"
                           WHERE m."workspaceId" = ?1 ORDER BY m."id""#,
                    )
                    .map_err(chatbotx_db::repo::Error::from)?;
                let rows = stmt
                    .query_map([ws], |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    })
                    .map_err(chatbotx_db::repo::Error::from)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(chatbotx_db::repo::Error::from)?;
                Ok(rows)
            })
            .await
            .map_err(fail)?;
            Ok(admins
                .into_iter()
                .filter(|(_, _, _, permissions)| {
                    permissions
                        .as_deref()
                        .and_then(|p| serde_json::from_str::<Value>(p).ok())
                        .and_then(|p| p.get("superAdmin").and_then(Value::as_bool))
                        == Some(true)
                })
                .map(|(user_id, name, email, _)| {
                    let label = [name, email]
                        .into_iter()
                        .flatten()
                        .find(|s| !s.is_empty())
                        .unwrap_or_else(|| user_id.to_string());
                    json!({ "id": id(user_id), "label": label })
                })
                .collect())
        }
        _ => unreachable!("checked against ACTIONS"),
    }
}

/// `assertEnterpriseFeatures()` + `assertWorkspaceSuperAdmin(workspaceId)`.
async fn assert_super_admin(ctx: &Ctx<'_>) -> Result<(), OrpcError> {
    ctx.api.assert_enterprise_features().await.map_err(fail)?;
    if ctx.permissions.get("superAdmin") != Some(&Value::Bool(true)) {
        return Err(fail(BusinessError::exception(
            SUPER_ADMIN_REQUIRED,
            "systemError",
            400,
        )));
    }
    Ok(())
}
