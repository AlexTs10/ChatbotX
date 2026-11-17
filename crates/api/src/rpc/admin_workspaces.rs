//! The admin console workspace actions (Q.D11 slice 11) under the `/rpc` key
//! `adminWorkspaceActions`:
//!
//! - `createAdminWorkspace` (`enterprise/features/admin-workspaces`:
//!   `superAdminActionClient` + `assertEnterpriseFeatures` +
//!   `adminWorkspaceService.createWithMembers`);
//! - `listAdminWorkspaces` (`features/admin-workspaces/queries`: a super
//!   admin only, else 404; `workspaceSupportAccessService.listWorkspaces`).

use std::collections::HashSet;

use chatbotx_business::BusinessError;
use chatbotx_business::audit::AuditRecordInput;
use chatbotx_business::workspace::{self as ws, CreateWorkspace, ExtraMember};
use chatbotx_db::repo::models::Workspace;
use serde_json::{Value, json};

use super::actions::{ActionContext, ActionError, ActionResult, Gate};
use super::platform_admins::{EMAIL, find_or_provision_root_user};
use super::platform_credentials::{is_super_admin, unauthorized};
use crate::handlers::input::{Input, Num, Str, child, key};
use crate::handlers::{ApiState, date, id};

/// `MAX_ADMIN_WORKSPACE_MEMBERS`.
pub const MAX_ADMIN_WORKSPACE_MEMBERS: usize = 20;
/// `ROOT_TENANT_ID`.
const ROOT_TENANT_ID: i64 = 1;

/// The actions of the `adminWorkspaceActions` key.
pub fn action_gate(name: &str) -> Option<Gate> {
    matches!(name, "createAdminWorkspace" | "listAdminWorkspaces").then_some(Gate::Auth)
}

/// One resolved account (`AdminWorkspaceProvisionedMember`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provisioned {
    pub email: String,
    pub user_id: i64,
    pub created: bool,
}

/// `createWithMembers` result.
#[derive(Debug, Clone)]
pub struct Created {
    pub workspace: Workspace,
    pub owner: Provisioned,
    pub members: Vec<Provisioned>,
}

async fn read<R: Send + 'static>(
    api: &ApiState,
    f: impl FnOnce(&chatbotx_db::rusqlite::Connection) -> chatbotx_db::repo::Result<R> + Send + 'static,
) -> Result<R, ActionError> {
    Ok(chatbotx_business::tx::read(&api.db, move |c| Ok(f(c)?)).await?)
}

/// `adminWorkspaceService.createWithMembers`: the owner is resolved (or
/// provisioned) first; the existing-workspace and team-member checks run
/// before any member is resolved; members of a reseller tenant must exist.
pub async fn create_with_members(
    api: &ApiState,
    mailer: &dyn crate::mail::Mailer,
    actor_id: i64,
    name: &str,
    owner_email: &str,
    member_emails: &[String],
    permissions: &Value,
) -> Result<Created, ActionError> {
    let owner_email = owner_email.trim().to_lowercase();
    let mut seen = HashSet::new();
    let member_emails: Vec<String> = member_emails
        .iter()
        .map(|e| e.trim().to_lowercase())
        .filter(|e| seen.insert(e.clone()) && *e != owner_email)
        .collect();
    if member_emails.len() > MAX_ADMIN_WORKSPACE_MEMBERS {
        return Err(BusinessError::exception(
            format!("At most {MAX_ADMIN_WORKSPACE_MEMBERS} members can be added at once."),
            "tooManyMembers",
            400,
        )
        .into());
    }
    let (owner_id, owner_created) = find_or_provision_root_user(api, mailer, &owner_email).await?;
    let additional = 1 + member_emails.len() as i64;
    let (existing, tenant_id, has_room) = read(api, move |c| {
        Ok((
            chatbotx_db::repo::workspace::find_active_by_owner(c, owner_id)?.is_some(),
            ws::resolve_tenant_for_owner(c, owner_id)?,
            chatbotx_business::quota_enforcement::has_room_for_team_members(
                c, owner_id, additional,
            )?,
        ))
    })
    .await?;
    if existing {
        return Err(BusinessError::exception(
            "This owner already has a workspace.",
            "ownerAlreadyHasWorkspace",
            409,
        )
        .into());
    }
    if !has_room {
        return Err(BusinessError::exception(
            "Team member limit reached for this plan",
            "teamMembersLimit",
            400,
        )
        .into());
    }
    let mut members = Vec::new();
    for email in member_emails {
        let member = if tenant_id == ROOT_TENANT_ID {
            let (user_id, created) = find_or_provision_root_user(api, mailer, &email).await?;
            Provisioned {
                email,
                user_id,
                created,
            }
        } else {
            let lookup = email.clone();
            let found = read(api, move |c| {
                chatbotx_db::repo::user::find_id_by_email(c, &lookup, tenant_id)
            })
            .await?;
            let Some(user_id) = found else {
                return Err(BusinessError::exception(
                    format!("No user found for \"{email}\" in this tenant."),
                    "memberNotFound",
                    404,
                )
                .into());
            };
            Provisioned {
                email,
                user_id,
                created: false,
            }
        };
        members.push(member);
    }
    let workspace = ws::create(
        &api.db,
        api.edition,
        CreateWorkspace {
            name: name.to_owned(),
            owner_id: Some(owner_id),
            tenant_id: Some(tenant_id),
            created_by: owner_id,
            members: members
                .iter()
                .map(|m| ExtraMember {
                    user_id: m.user_id,
                    permissions: permissions.clone(),
                })
                .collect(),
        },
    )
    .await?;
    let new = |created: bool| if created { " (new account)" } else { "" };
    let mut detail = format!(
        "created the workspace (#{}) for {owner_email}{}",
        workspace.id,
        new(owner_created)
    );
    if !members.is_empty() {
        let list: Vec<String> = members
            .iter()
            .map(|m| format!("{}{}", m.email, new(m.created)))
            .collect();
        detail.push_str(&format!(" with members: {}", list.join(", ")));
    }
    let mut audit = AuditRecordInput::new("create", detail);
    audit.user_id = Some(actor_id.to_string());
    audit.workspace_id = Some(workspace.id.to_string());
    api.audit.record(audit).await;
    Ok(Created {
        workspace,
        owner: Provisioned {
            email: owner_email,
            user_id: owner_id,
            created: owner_created,
        },
        members,
    })
}

fn provisioned_json(p: &Provisioned) -> Value {
    json!({ "email": p.email, "userId": id(p.user_id), "created": p.created })
}

/// The workspace columns the admin console reads.
// ponytail: not the whole `WorkspaceModel`; add columns when a caller reads them.
fn workspace_json(w: &Workspace) -> Value {
    json!({
        "id": id(w.id),
        "createdAt": date(w.created_at),
        "updatedAt": date(w.updated_at),
        "name": w.name,
        "ownerId": id(w.owner_id),
        "tenantId": id(w.tenant_id),
    })
}

/// `workspaceSupportAccessService.listWorkspaces({ page, perPage, keyword })`.
pub async fn list_workspaces(
    api: &ApiState,
    page: i64,
    per_page: i64,
    keyword: Option<String>,
) -> Result<Value, ActionError> {
    let (rows, count) = read(api, move |c| {
        // `ilike` on name / owner email / id (`likeContains`).
        let pattern = keyword.map(|k| {
            let escaped = k.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            format!("%{escaped}%")
        });
        let filter = r#"(?1 IS NULL OR w."name" LIKE ?1 ESCAPE '\' OR u."email" LIKE ?1 ESCAPE '\' OR CAST(w."id" AS TEXT) LIKE ?1 ESCAPE '\')"#;
        let sql = format!(
            r#"SELECT w."id", w."name", w."ownerId", u."name", u."email", w."tenantId", t."brandName", w."createdAt", w."supportAccessUntil"
               FROM "Workspace" w JOIN "User" u ON u."id" = w."ownerId" JOIN "Tenant" t ON t."id" = w."tenantId"
               WHERE {filter}
               ORDER BY CASE WHEN w."supportAccessUntil" > ?2 THEN 0 WHEN w."supportAccessUntil" IS NULL THEN 2 ELSE 1 END, w."createdAt" DESC, w."id" DESC
               LIMIT ?3 OFFSET ?4"#
        );
        let now = chatbotx_db::repo::now_ms();
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt
            .query_map(
                chatbotx_db::rusqlite::params![pattern, now, per_page, (page - 1) * per_page],
                |r| {
                    Ok(json!({
                        "id": id(r.get(0)?),
                        "name": r.get::<_, String>(1)?,
                        "ownerId": id(r.get(2)?),
                        "ownerName": r.get::<_, Option<String>>(3)?,
                        "ownerEmail": r.get::<_, String>(4)?,
                        "tenantId": id(r.get(5)?),
                        "tenantName": r.get::<_, Option<String>>(6)?,
                        "createdAt": date(r.get(7)?),
                        "supportAccessUntil": r.get::<_, Option<i64>>(8)?.map(date),
                    }))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let count: i64 = c.query_row(
            &format!(
                r#"SELECT COUNT(*) FROM "Workspace" w JOIN "User" u ON u."id" = w."ownerId" WHERE {filter}"#
            ),
            [&pattern],
            |r| r.get(0),
        )?;
        Ok((rows, count))
    })
    .await?;
    Ok(json!({ "data": rows, "pageCount": (count + per_page - 1) / per_page }))
}

/// Runs one action of the `adminWorkspaceActions` key.
pub async fn run_action(
    api: &ApiState,
    name: &str,
    context: &ActionContext,
    raw: &Value,
) -> ActionResult {
    let super_admin = is_super_admin(api, context.user_id).await?;
    let mut input = Input::from_object(raw.as_object().cloned().unwrap_or_default());
    match name {
        "listAdminWorkspaces" => {
            if !super_admin {
                // `notFound()`.
                return Err(BusinessError::not_found("Not found").into());
            }
            // nuqs `page` / `perPage` (defaults 1 / 10) and `keyword`.
            let page = input
                .opt_coerce_int("page", Num::int(1.0, f64::MAX))
                .unwrap_or(1);
            let per_page = input
                .opt_coerce_int("perPage", Num::int(1.0, 100.0))
                .unwrap_or(10);
            let keyword = input.nullish_string("keyword", Str::max(255)).flatten();
            input.finish()?;
            Ok(Some(
                list_workspaces(api, page, per_page, keyword.filter(|k| !k.is_empty())).await?,
            ))
        }
        "createAdminWorkspace" => {
            if !super_admin {
                return Err(unauthorized());
            }
            let workspace_name = input.string("name", Str::trimmed(1, 255));
            let owner = input.string("ownerEmail", EMAIL);
            let members = match input.get("memberEmails").cloned() {
                None => Some(Vec::new()),
                Some(Value::Array(items)) => {
                    if items.len() > MAX_ADMIN_WORKSPACE_MEMBERS {
                        input.issue(
                            &key("memberEmails"),
                            "too_big",
                            "Too big: expected array to have <=20 items",
                        );
                    }
                    let mut out = Vec::new();
                    for (i, item) in items.iter().enumerate() {
                        let path = child(&key("memberEmails"), &i.to_string());
                        if let Some(email) = input.string_at(&path, Some(item), EMAIL, true) {
                            out.push(email);
                        }
                    }
                    Some(out)
                }
                Some(_) => {
                    input.issue(
                        &key("memberEmails"),
                        "invalid_type",
                        "Invalid input: expected array",
                    );
                    None
                }
            };
            let permissions = super::workspace_members::permissions_request(&mut input);
            input.finish()?;
            api.assert_enterprise_features().await?;
            let created = create_with_members(
                api,
                context.mailer.as_ref(),
                context.user_id,
                &workspace_name.unwrap_or_default(),
                &owner.unwrap_or_default(),
                &members.unwrap_or_default(),
                &permissions.unwrap_or_default(),
            )
            .await?;
            Ok(Some(json!({
                "workspace": workspace_json(&created.workspace),
                "owner": provisioned_json(&created.owner),
                "members": created.members.iter().map(provisioned_json).collect::<Vec<_>>(),
            })))
        }
        _ => Err(ActionError::Internal(format!("unknown action {name}"))),
    }
}
