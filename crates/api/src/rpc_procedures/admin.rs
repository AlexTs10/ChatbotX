//! The `/admin` (platform operator) and `/manage` (cloud reseller) console
//! page reads, as `POST /rpc/actions/<name>` without a workspace. Each one
//! re-checks the caller like the original page or layout.
//!
//! `/admin` (`assertPlatformSuperAdmin`: a `PlatformAdmin` row, else
//! `notFound()`; the `(enterprise)` pages also `hasEnterpriseFeatures`):
//!
//! - `listPlatformAdmins(page)`: `platformAdminService.list({ page, perPage: 50 })`
//! - `listRootHelpItems()`: `tenantHelpItemService.listByTenant(ROOT_TENANT_ID)`
//! - `getRootDefaultTemplateSettings()`: `app/admin/default-template/page.tsx`
//! - `getRootTenant()`: `tenantService.findById(ROOT_TENANT_ID)` (branding,
//!   email templates and platform channels pages)
//! - `getPlatformSmtpSettings()` (enterprise, not cloud): the SMTP public config
//! - `listPlatformAuditLogs(search)`, `listPlatformAuditWorkspaces(selectedId)`
//!   (enterprise): `enterprise/features/platform-audit-logs/queries`
//! - `listDeletedWorkspaces(search)`: `workspaceDeletionRecordService.listForAdmin`
//!
//! `/manage` (`app/manage/layout.tsx`: cloud edition and
//! `hasOwnCredentialAccess`; most pages also `requireActiveReseller`):
//!
//! - `getManageAccess()`: the layout's access flags (404 when refused)
//! - `getOwnTenant()`, `getOwnPlatformChannels()`, `listOwnHelpItems()`,
//!   `getOwnDefaultTemplateSettings()` (active reseller)
//!
//! Both: `getPlatformCredentialCards(scope)` (`ManagePlatformCredentials`):
//! `"platform"` = a super admin, the global credentials; `"user"` on cloud =
//! the caller's own (inherited cards show no values).

use chatbotx_business::BusinessError;
use chatbotx_business::audit::log_query::{self, ListAuditLogsInput};
use chatbotx_business::platform_credential as credential;
use chatbotx_business::template::tenant_default::{self, Scope};
use chatbotx_business::tenant;
use chatbotx_db::repo::PaginationInput;
use chatbotx_db::repo::models::Tenant;
use chatbotx_db::repo::platform_admin::{self, ROOT_TENANT_ID};
use serde_json::{Value, json};

use super::{Ctx, In, Outcome, arg, fail, page};
use crate::handlers::{date, id};
use crate::orpc::OrpcError;

const ADMIN_READS: &[&str] = &[
    "listPlatformAdmins",
    "listRootHelpItems",
    "getRootDefaultTemplateSettings",
    "getRootTenant",
    "getPlatformSmtpSettings",
    "listPlatformAuditLogs",
    "listPlatformAuditWorkspaces",
    "listDeletedWorkspaces",
];
/// The admin reads outside the `(enterprise)` route group.
const NOT_ENTERPRISE: &[&str] = &[
    "getRootDefaultTemplateSettings",
    "getRootTenant",
    "listDeletedWorkspaces",
];
const MANAGE_READS: &[&str] = &[
    "getManageAccess",
    "getOwnTenant",
    "getOwnPlatformChannels",
    "listOwnHelpItems",
    "getOwnDefaultTemplateSettings",
];
/// `PLATFORM_AUDIT_WORKSPACE_OPTION_LIMIT`.
const AUDIT_WORKSPACE_OPTIONS: i64 = 200;
/// The cards of `ManagePlatformCredentials`, in its order of reads.
const CARD_TYPES: &[&str] = &[
    "whatsapp",
    "messenger",
    "instagram",
    "instagramFacebook",
    "threads",
    "google",
    "googleAds",
    "zalo",
    "giphy",
    "tiktok",
    "make",
    "stripe",
];

pub(super) async fn action(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Outcome {
    if ADMIN_READS.contains(&name) {
        return Some(admin(ctx, name, args).await);
    }
    if MANAGE_READS.contains(&name) {
        return Some(manage(ctx, name).await);
    }
    if name == "getPlatformCredentialCards" {
        return Some(credential_cards(ctx, arg(args, 0)).await);
    }
    None
}

fn not_found() -> OrpcError {
    fail(BusinessError::not_found("Not found"))
}

async fn is_super_admin(ctx: &Ctx<'_>) -> Result<bool, OrpcError> {
    let user_id = ctx.user_id;
    chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        Ok(platform_admin::find_for_auth(conn, user_id)?.is_some())
    })
    .await
    .map_err(fail)
}

fn tenant_json(tenant: Option<Tenant>) -> Value {
    tenant.map_or(Value::Null, |t| t.to_json())
}

fn hidden_channels(tenant: Option<&Tenant>) -> Value {
    tenant
        .and_then(|t| t.hidden_channels.as_ref())
        .map_or_else(|| json!([]), |j| j.0.clone())
}

async fn admin(ctx: &Ctx<'_>, name: &str, args: &[Value]) -> Result<Value, OrpcError> {
    let db = &ctx.api.db;
    // `notFound()`; the `(enterprise)` layout 404s without the features too.
    if !is_super_admin(ctx).await? {
        return Err(not_found());
    }
    if !NOT_ENTERPRISE.contains(&name) && ctx.api.assert_enterprise_features().await.is_err() {
        return Err(not_found());
    }
    // `(enterprise)/(non-cloud)/layout.tsx`.
    if name == "getPlatformSmtpSettings" && ctx.api.edition.is_cloud() {
        return Err(not_found());
    }
    match name {
        "getRootDefaultTemplateSettings" => {
            Ok(
                tenant_default::get_settings(db, encryption(ctx)?, Scope::Root)
                    .await
                    .map_err(fail)?
                    .unwrap_or(Value::Null),
            )
        }
        "getRootTenant" => Ok(tenant_json(
            tenant::find_by_id(db, ROOT_TENANT_ID).await.map_err(fail)?,
        )),
        "getPlatformSmtpSettings" => {
            let encryption = std::sync::Arc::clone(
                ctx.api
                    .encryption
                    .as_ref()
                    .ok_or_else(|| fail(no_encryption()))?,
            );
            chatbotx_business::tx::read(db, move |conn| {
                Ok(credential::find_platform_public(conn, &encryption, "smtp")?)
            })
            .await
            .map(|config| config.unwrap_or(Value::Null))
            .map_err(fail)
        }
        "listPlatformAuditLogs" => {
            let input = In(arg(args, 0));
            let workspace_id = match input.opt_str("workspaceId")? {
                Some(w) if !w.is_empty() => Some(w.parse::<i64>().unwrap_or(-1)),
                _ => None,
            };
            let listed = log_query::list_platform_logs(
                db,
                workspace_id,
                ListAuditLogsInput {
                    workspace_id: 0,
                    page: input.opt_positive("page")?,
                    per_page: input.opt_positive("perPage")?,
                    sort: input.sort(),
                    keyword: input.opt_str("keyword")?,
                    user_id: None,
                    from: input.opt_str("from")?.unwrap_or_default(),
                    to: input.opt_str("to")?.unwrap_or_default(),
                },
            )
            .await
            .map_err(fail)?;
            let data = listed.data.into_iter().map(|row| {
                let mut out = row.log.to_json();
                out["user"] = row.user.map_or(Value::Null, |u| u.to_json());
                out["workspace"] = row.workspace.map_or(
                    Value::Null,
                    |(w, name)| json!({ "id": id(w), "name": name }),
                );
                out
            });
            Ok(page(data, listed.page_count))
        }
        "listPlatformAuditWorkspaces" => {
            let selected = arg(args, 0).as_str().filter(|s| !s.is_empty());
            let listed = crate::rpc::admin_workspaces::list_workspaces(
                ctx.api,
                1,
                AUDIT_WORKSPACE_OPTIONS,
                None,
            )
            .await
            .map_err(|_| {
                fail(BusinessError::exception(
                    "Internal error",
                    "systemError",
                    500,
                ))
            })?;
            let mut options: Vec<Value> = listed["data"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|w| json!({ "id": w["id"], "label": w["name"] }))
                .collect();
            // A deep-linked `?workspaceId=` outside the first page.
            if let Some(selected) = selected
                && !options.iter().any(|o| o["id"] == selected)
                && let Ok(workspace_id) = selected.parse::<i64>()
            {
                let found = chatbotx_business::tx::read(db, move |conn| {
                    Ok(chatbotx_business::workspace::find(conn, workspace_id)?)
                })
                .await
                .map_err(fail)?;
                if let Some(w) = found {
                    options.insert(0, json!({ "id": id(w.id), "label": w.name }));
                }
            }
            Ok(Value::Array(options))
        }
        "listDeletedWorkspaces" => {
            let input = In(arg(args, 0));
            let page_number = input.opt_positive("page")?.unwrap_or(1);
            let per_page = input.opt_positive("perPage")?.unwrap_or(10);
            let search = input.opt_str("search")?.filter(|s| !s.is_empty());
            let (rows, total) = chatbotx_business::tx::read(db, move |conn| {
                Ok(
                    chatbotx_db::repo::workspace_deletion_record::list_for_admin(
                        conn,
                        per_page,
                        (page_number - 1) * per_page,
                        search.as_deref(),
                        None,
                    )?,
                )
            })
            .await
            .map_err(fail)?;
            Ok(json!({
                "data": rows.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
                "pageCount": (total + per_page - 1) / per_page,
            }))
        }
        "listPlatformAdmins" => {
            let page_number = arg(args, 0).as_i64().filter(|p| *p >= 1);
            let listed = chatbotx_business::tx::read(db, move |conn| {
                Ok(platform_admin::list_with_user(
                    conn,
                    PaginationInput {
                        page: page_number,
                        per_page: Some(50),
                    },
                )?)
            })
            .await
            .map_err(fail)?;
            Ok(page(
                listed.data.iter().map(|a| {
                    json!({
                        "id": id(a.id),
                        "createdAt": date(a.created_at),
                        "updatedAt": date(a.updated_at),
                        "userId": id(a.user_id),
                        "role": a.role,
                        "userEmail": a.user_email,
                        "userName": a.user_name,
                    })
                }),
                listed.page_count,
            ))
        }
        _ => Ok(
            chatbotx_business::tenant_help_item::list_by_tenant(db, ROOT_TENANT_ID)
                .await
                .map_err(fail)?
                .iter()
                .map(|item| item.to_json())
                .collect(),
        ),
    }
}

fn no_encryption() -> BusinessError {
    BusinessError::exception("encryption keys are not configured", "systemError", 500)
}

fn encryption<'a>(
    ctx: &'a Ctx<'_>,
) -> Result<&'a chatbotx_core::encryption::Encryption, OrpcError> {
    ctx.api
        .encryption
        .as_deref()
        .ok_or_else(|| fail(no_encryption()))
}

/// `getManageAccess` of `app/manage/reseller-guard.ts`.
struct ManageAccess {
    tenant: Option<Tenant>,
    can_manage_credentials: bool,
    can_manage_reseller: bool,
    saas_mode: bool,
    white_label: bool,
}

async fn manage_access(ctx: &Ctx<'_>) -> Result<ManageAccess, OrpcError> {
    let user_id = ctx.user_id;
    let tenant = tenant::find_by_owner(&ctx.api.db, user_id)
        .await
        .map_err(fail)?;
    let quota = chatbotx_business::tx::read(&ctx.api.db, move |conn| {
        Ok(chatbotx_business::user_quota::get_for_user(conn, user_id)?)
    })
    .await
    .map_err(fail)?;
    let status = tenant.as_ref().map(|t| t.status.as_str());
    Ok(ManageAccess {
        can_manage_credentials: credential::has_own_credential_access(
            status,
            quota.as_ref().is_some_and(|q| q.custom_app_credentials),
        ),
        can_manage_reseller: status == Some("active"),
        saas_mode: quota.as_ref().is_some_and(|q| q.saas_mode),
        white_label: quota.as_ref().is_some_and(|q| q.white_label),
        tenant,
    })
}

async fn manage(ctx: &Ctx<'_>, name: &str) -> Result<Value, OrpcError> {
    let db = &ctx.api.db;
    // The manage layout: cloud only, and own-credential access.
    if !ctx.api.edition.is_cloud() {
        return Err(not_found());
    }
    let access = manage_access(ctx).await?;
    if !access.can_manage_credentials {
        return Err(not_found());
    }
    if name == "getManageAccess" {
        // Pricing lives on a white-label reseller's custom domain.
        let pricing = match &access.tenant {
            Some(t) if access.can_manage_reseller && access.white_label => {
                let tenant_id = t.id;
                let domains = tenant::find_domains_by_tenant(db, tenant_id)
                    .await
                    .map_err(fail)?;
                let active = domains.into_iter().find(|d| d.status == "active");
                json!({ "domain": active.map(|d| d.domain) })
            }
            _ => Value::Null,
        };
        return Ok(json!({
            "canManageCredentials": access.can_manage_credentials,
            "canManageReseller": access.can_manage_reseller,
            "flags": { "saasMode": access.saas_mode, "whiteLabel": access.white_label },
            "pricing": pricing,
        }));
    }
    // `requireActiveReseller()`.
    if !access.can_manage_reseller {
        return Err(not_found());
    }
    let Some(tenant) = access.tenant else {
        return Err(not_found());
    };
    match name {
        "getOwnTenant" => Ok(tenant.to_json()),
        "getOwnPlatformChannels" => {
            let root = tenant::find_by_id(db, ROOT_TENANT_ID).await.map_err(fail)?;
            Ok(json!({
                "hiddenChannels": hidden_channels(Some(&tenant)),
                "platformHiddenChannels": hidden_channels(root.as_ref()),
            }))
        }
        "listOwnHelpItems" => Ok(chatbotx_business::tenant_help_item::list_by_tenant(
            db, tenant.id,
        )
        .await
        .map_err(fail)?
        .iter()
        .map(|item| item.to_json())
        .collect()),
        _ => Ok(
            tenant_default::get_settings(db, encryption(ctx)?, Scope::Owner(ctx.user_id))
                .await
                .map_err(fail)?
                .unwrap_or(Value::Null),
        ),
    }
}

/// `PLACEHOLDER_DOMAIN_ORIGIN`.
const PLACEHOLDER_DOMAIN_ORIGIN: &str = "https://<your-domain.com>";

/// `ManagePlatformCredentials` (`resolveCard` per type, the origins and the
/// webhook URLs of `lib/provider-origin.ts`).
async fn credential_cards(ctx: &Ctx<'_>, scope: &Value) -> Result<Value, OrpcError> {
    let db = &ctx.api.db;
    let user_scope = scope.as_str() != Some("platform");
    let scoped_user = (user_scope && ctx.api.edition.is_cloud()).then_some(ctx.user_id);
    match scoped_user {
        // A reseller (manage layout): own-credential access.
        Some(_) => {
            if !manage_access(ctx).await?.can_manage_credentials {
                return Err(not_found());
            }
        }
        // The platform's credentials: a super admin.
        None => {
            if !is_super_admin(ctx).await? {
                return Err(not_found());
            }
        }
    }
    // `resolveCredentialOwnerOrigin`.
    let (custom_domain_origin, has_active_tenant) = match scoped_user {
        Some(user) => match tenant::find_by_owner(db, user).await.map_err(fail)? {
            Some(t) if t.status == "active" => (
                tenant::find_active_domain_by_tenant(db, t.id)
                    .await
                    .map_err(fail)?
                    .map(|d| format!("https://{}", d.domain)),
                true,
            ),
            _ => (None, false),
        },
        None => (None, false),
    };
    let broker = crate::handlers::messenger_templates::broker_origin(&ctx.api.app_url);
    let origin_for = |inherited: bool| {
        if inherited || !has_active_tenant {
            broker.clone()
        } else {
            custom_domain_origin
                .clone()
                .unwrap_or_else(|| PLACEHOLDER_DOMAIN_ORIGIN.to_string())
        }
    };
    let encryption = std::sync::Arc::clone(
        ctx.api
            .encryption
            .as_ref()
            .ok_or_else(|| fail(no_encryption()))?,
    );
    let (cards, has_stripe_webhook) = chatbotx_business::tx::read(db, move |conn| {
        let mut cards = Vec::new();
        for kind in CARD_TYPES {
            // `Promise.allSettled`: a failed read is the empty card.
            let card = match scoped_user {
                None => credential::find_platform_public(conn, &encryption, kind)
                    .ok()
                    .flatten()
                    .map(|config| (None, config, false)),
                Some(user) => credential::resolve_public_for_user(conn, &encryption, user, kind)
                    .ok()
                    .flatten()
                    .map(|c| {
                        if c.is_inherited {
                            (None, Value::Null, true)
                        } else {
                            (c.credential_id, c.public_config, false)
                        }
                    }),
            };
            cards.push((*kind, card.unwrap_or((None, Value::Null, false))));
        }
        let webhook = credential::find_decrypted(
            conn,
            &encryption,
            scoped_user,
            "stripe",
            credential::DecryptOptions::default(),
        )
        .ok()
        .flatten()
        .is_some_and(|c| {
            c.config
                .get("connectWebhookSecret")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        });
        Ok((cards, webhook))
    })
    .await
    .map_err(fail)?;
    let mut out = serde_json::Map::new();
    for (kind, (credential_id, public_config, is_inherited)) in cards {
        let origin = origin_for(is_inherited);
        // `buildCredentialWebhookUrl`.
        let path = match credential_id {
            Some(c) if !has_active_tenant => format!("/integrations/{kind}/webhook/c/{c}"),
            _ => format!("/integrations/{kind}/webhook"),
        };
        out.insert(
            kind.to_string(),
            json!({
                "credentialId": credential_id.map(id),
                "publicConfig": public_config,
                "isInherited": is_inherited,
                "callbackOrigin": origin,
                "webhookUrl": format!("{}{path}", origin.trim_end_matches('/')),
            }),
        );
    }
    out.insert("hasStripeWebhook".into(), json!(has_stripe_webhook));
    Ok(Value::Object(out))
}
