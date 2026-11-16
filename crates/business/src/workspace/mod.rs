//! `packages/business/src/workspace` (Q.D11): `workspaceService` reads, the
//! settings write of the public API and the builder forms, the deletion
//! request (schedule / cancel, with the `ResourceDeletionLog` trail) and
//! workspace creation.
//!
//! Not here: caches (no workspace cache in Rust), template installs of a new
//! workspace (templates unit), the purge job (`lifecycle`).

pub mod codes;
pub mod deletion_schedule;

use chatbotx_db::repo::models::Workspace;
use chatbotx_db::repo::platform_admin::ROOT_TENANT_ID;
use chatbotx_db::repo::resource_deletion_log::{
    ResourceDeletionLogInsert, ResourceKey, cancel_pending_by_resource, insert as insert_log,
};
use chatbotx_db::repo::sql::Sql as Query;
use chatbotx_db::repo::workspace::{WorkspaceDeletionRequestState, find_by_id_for_update};
use chatbotx_db::repo::{self, Model, find_one, new_id, now_ms, patch};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::audit::{AuditActor, audit_actor};
use crate::keys::Edition;
use crate::pipeline::events::PipelineItemChanged;
use crate::user_quota::{self, QuotaMetric};
use crate::workspace_usage::{self, WorkspaceUsageMetric};
use crate::{BusinessError, Result, tx};

/// `FULL_WORKSPACE_MEMBER_PERMISSIONS` keys (every flag `true` for an owner).
pub const PERMISSION_KEYS: &[&str] = &[
    "superAdmin",
    "analytics",
    "flows",
    "contacts",
    "onlyAssignedContacts",
    "emailAndPhone",
    "broadcast",
    "ecommerce",
];

/// `COMMUNITY_MAX_WORKSPACES`.
pub const COMMUNITY_MAX_WORKSPACES: i64 = 1;

/// The owner's permissions (every flag on).
pub fn full_permissions() -> Value {
    Value::Object(
        PERMISSION_KEYS
            .iter()
            .map(|key| ((*key).to_string(), Value::Bool(true)))
            .collect(),
    )
}

/// `DEFAULT_MEMBER_NOTIFICATION_TYPES`.
pub fn default_notification_types() -> Value {
    json!({ "notifyAdmin": true, "newMessageToHuman": true, "newOrder": true })
}

/// `DEFAULT_MEMBER_NOTIFICATION_CHANNELS`.
pub fn default_notification_channels() -> Value {
    json!({ "messenger": true, "email": true, "telegram": true, "browser": true })
}

/// `workspaceLimitReachedException`.
pub fn workspace_limit_reached() -> BusinessError {
    BusinessError::exception(
        "Workspace limit reached for this plan",
        "workspaceLimitReached",
        400,
    )
}

/// `workspaceDeletionStartedException`.
pub fn workspace_deletion_started() -> BusinessError {
    BusinessError::exception(
        "Workspace deletion is already in progress",
        "workspaceDeletionStarted",
        409,
    )
}

/// `isWorkspaceScheduledForDeletion`.
pub fn is_scheduled_for_deletion(workspace: &Workspace) -> bool {
    workspace.scheduled_deletion_at.is_some()
}

/// One workspace row (`findForAuth`, uncached in Rust anyway).
pub fn find(conn: &Connection, id: i64) -> repo::Result<Option<Workspace>> {
    find_one::<Workspace>(conn, "\"id\" = ?1", &[&id])
}

/// `findById` / `findOrFail`: 404 "Workspace not found".
pub async fn find_by_id(db: &chatbotx_db::Db, id: i64) -> Result<Workspace> {
    tx::read(db, move |conn| {
        find(conn, id)?.ok_or_else(|| BusinessError::not_found("Workspace not found"))
    })
    .await
}

/// `isActiveNow`: inactive = `false`; no window = `true`; else the local
/// `HH:mm` of `now_ms` in the workspace timezone against the window
/// (an overnight window wraps midnight).
pub fn is_active_now(
    is_active: bool,
    start_time: Option<&str>,
    end_time: Option<&str>,
    timezone: &str,
    now_ms: i64,
) -> bool {
    if !is_active {
        return false;
    }
    let (Some(start), Some(end)) = (
        start_time.filter(|s| !s.is_empty()),
        end_time.filter(|s| !s.is_empty()),
    ) else {
        return true;
    };
    let Some(instant) = chrono::DateTime::from_timestamp_millis(now_ms) else {
        return true;
    };
    let current = match timezone.parse::<chrono_tz::Tz>() {
        Ok(tz) => instant.with_timezone(&tz).format("%H:%M").to_string(),
        Err(_) => instant.format("%H:%M").to_string(),
    };
    let current = current.as_str();
    if start <= end {
        current >= start && current <= end
    } else {
        current >= start || current <= end
    }
}

/// The keys `updateSettings` writes (`WORKSPACE_SETTINGS_KEYS`); `None` =
/// left out (`undefined`), `Some(None)` = `null`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SettingsPatch {
    /// The Default Reply flow id.
    pub default_reply: Option<Option<i64>>,
    pub default_reply_frequency: Option<String>,
    pub smart_response_delay_seconds: Option<Option<i64>>,
    pub capi_limited_data_use: Option<bool>,
    pub logo: Option<Option<String>>,
    pub target_country: Option<String>,
    pub language: Option<String>,
    pub timezone: Option<String>,
    pub brand_color: Option<String>,
    pub development_mode: Option<bool>,
}

impl SettingsPatch {
    /// The `(column, value)` pairs of the given keys, in `WORKSPACE_SETTINGS_KEYS` order.
    pub fn columns(&self) -> patch::Patch {
        let mut out: patch::Patch = Vec::new();
        let text = |v: &Option<String>| v.clone().map_or(Sql::Null, Sql::Text);
        if let Some(value) = &self.default_reply {
            out.push((
                "defaultReply",
                value.map_or(Sql::Null, |id| Sql::Text(id.to_string())),
            ));
        }
        if let Some(value) = &self.default_reply_frequency {
            out.push(("defaultReplyFrequency", Sql::Text(value.clone())));
        }
        if let Some(value) = self.smart_response_delay_seconds {
            out.push((
                "smartResponseDelaySeconds",
                value.map_or(Sql::Null, Sql::Integer),
            ));
        }
        if let Some(value) = self.capi_limited_data_use {
            out.push(("capiLimitedDataUse", Sql::Integer(i64::from(value))));
        }
        if let Some(value) = &self.logo {
            out.push(("logo", text(value)));
        }
        if let Some(value) = &self.target_country {
            out.push(("targetCountry", Sql::Text(value.clone())));
        }
        if let Some(value) = &self.language {
            out.push(("language", Sql::Text(value.clone())));
        }
        if let Some(value) = &self.timezone {
            out.push(("timezone", Sql::Text(value.clone())));
        }
        if let Some(value) = &self.brand_color {
            out.push(("brandColor", Sql::Text(value.clone())));
        }
        if let Some(value) = self.development_mode {
            out.push(("developmentMode", Sql::Integer(i64::from(value))));
        }
        out
    }
}

/// What a write returns besides the row: the audit detail (`this.audit`,
/// action `update`) and the pipeline refresh events of a country write.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceWrite {
    pub workspace: Workspace,
    /// `Some(detail)` when the original audits the write (action `update`).
    pub audit: Option<String>,
    /// `notifyWorkspaceCurrencyChange` events (a `targetCountry` write).
    pub currency_changes: Vec<PipelineItemChanged>,
}

/// The deletion metadata columns (a write of only these is not audited).
const DELETION_METADATA: &[&str] = &[
    "deletionRequestedAt",
    "deletionRequestedById",
    "scheduledDeletionAt",
];

/// `update` inside a transaction: writes `patch` (+ `updatedAt`); a patch
/// with `scheduledDeletionAt` only while no purge started. No row ->
/// `workspaceDeletionStarted`. Returns the row and the audit detail the
/// original writes when it owns the transaction.
pub fn update_in(
    conn: &Connection,
    id: i64,
    patch: &patch::Patch,
) -> Result<(Workspace, Option<String>)> {
    let previous_name: Option<String> = if patch.iter().any(|(col, _)| *col == "name") {
        conn.query_row(
            r#"SELECT "name" FROM "Workspace" WHERE "id" = ?1"#,
            [id],
            |row| row.get(0),
        )
        .optional()
        .map_err(repo::Error::from)?
    } else {
        None
    };
    let mut full = patch.clone();
    full.push(("updatedAt", Sql::Integer(now_ms())));
    let set = patch::set_clause(Workspace::COLUMNS, &full)?;
    let guard = if patch.iter().any(|(col, _)| *col == "scheduledDeletionAt") {
        r#" AND "purgeStartedAt" IS NULL"#
    } else {
        ""
    };
    let mut query = Query::raw("UPDATE \"Workspace\" SET ");
    query.append(set);
    query.push(" WHERE \"id\" = ");
    query.push_bind(id);
    query.push(guard);
    let changed = query.execute(conn)?;
    if changed == 0 {
        return Err(workspace_deletion_started());
    }
    let workspace = find(conn, id)?.ok_or_else(workspace_deletion_started)?;
    let only_deletion_metadata =
        !patch.is_empty() && patch.iter().all(|(col, _)| DELETION_METADATA.contains(col));
    let audit = (!patch.is_empty() && !only_deletion_metadata).then(|| {
        let renamed = patch.iter().any(|(col, value)| {
            *col == "name" && Some(value) != previous_name.clone().map(Sql::Text).as_ref()
        });
        if renamed {
            "changed the workspace name".to_string()
        } else {
            "updated the workspace configuration".to_string()
        }
    });
    Ok((workspace, audit))
}

/// `update` in its own transaction.
pub async fn update(db: &chatbotx_db::Db, id: i64, patch: patch::Patch) -> Result<WorkspaceWrite> {
    let (workspace, audit) = tx::write(db, move |tx| update_in(tx, id, &patch)).await?;
    Ok(WorkspaceWrite {
        workspace,
        audit,
        currency_changes: Vec::new(),
    })
}

/// `updateSettings`: a Default Reply flow must be a flow of the workspace
/// (else 404 "Flow not found", nothing written); an empty patch returns the
/// row; a `targetCountry` write refreshes every pipeline's currency.
pub async fn update_settings(
    db: &chatbotx_db::Db,
    id: i64,
    settings: SettingsPatch,
) -> Result<WorkspaceWrite> {
    if let Some(Some(flow_id)) = settings.default_reply {
        let found = tx::read(db, move |conn| {
            Ok(conn
                .query_row(
                    r#"SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                    params![flow_id, id],
                    |_| Ok(()),
                )
                .optional()
                .map_err(repo::Error::from)?
                .is_some())
        })
        .await?;
        if !found {
            return Err(BusinessError::not_found("Flow not found"));
        }
    }
    let columns = settings.columns();
    if columns.is_empty() {
        return Ok(WorkspaceWrite {
            workspace: find_by_id(db, id).await?,
            audit: None,
            currency_changes: Vec::new(),
        });
    }
    let mut out = update(db, id, columns).await?;
    if settings.target_country.is_some() {
        out.currency_changes =
            crate::pipeline::currency::notify_workspace_currency_change(db, id).await;
    }
    Ok(out)
}

/// `setLogoIfEmpty`: stores `logo` only while the workspace has none;
/// `true` when written (the original then audits "changed the workspace logo").
pub async fn set_logo_if_empty(db: &chatbotx_db::Db, id: i64, logo: String) -> Result<bool> {
    tx::write(db, move |tx| {
        Ok(tx
            .execute(
                r#"UPDATE "Workspace" SET "logo" = ?2, "updatedAt" = ?3 WHERE "id" = ?1 AND "logo" IS NULL"#,
                params![id, logo, now_ms()],
            )
            .map_err(repo::Error::from)?
            > 0)
    })
    .await
}

/// `MAX_IP_ADDRESS_LENGTH` / `MAX_USER_AGENT_LENGTH` of the deletion log.
const MAX_IP_ADDRESS_LENGTH: usize = 64;
const MAX_USER_AGENT_LENGTH: usize = 1024;

fn clip(value: Option<&String>, max: usize) -> Option<String> {
    value.map(|v| v.chars().take(max).collect())
}

fn workspace_key(id: i64) -> ResourceKey {
    ResourceKey {
        workspace_id: id,
        resource_type: "workspace".into(),
        resource_id: id.to_string(),
    }
}

/// `resourceDeletionLogService.recordRequest` / `recordHistoricalRequest`
/// of a workspace.
fn record_log(
    conn: &Connection,
    id: i64,
    name: &str,
    status: &str,
    requested_by: Option<i64>,
    actor: Option<&AuditActor>,
    requested_at: i64,
) -> repo::Result<()> {
    let now = now_ms();
    insert_log(
        conn,
        &ResourceDeletionLogInsert {
            workspace_id: id,
            resource_type: "workspace".into(),
            resource_id: id.to_string(),
            resource_subtype: None,
            resource_name: Some(name.to_string()),
            status: status.into(),
            requested_by_user_id: requested_by,
            request_source: actor.and_then(|a| a.source.clone()),
            ip_address: clip(
                actor.and_then(|a| a.ip_address.as_ref()),
                MAX_IP_ADDRESS_LENGTH,
            ),
            user_agent: clip(
                actor.and_then(|a| a.user_agent.as_ref()),
                MAX_USER_AGENT_LENGTH,
            ),
            requested_at,
            completed_at: (status == "completed").then_some(now),
            canceled_at: (status == "canceled").then_some(now),
        },
    )
}

/// `closePreviousWorkspaceDeletionRequest`: cancels the pending row; a
/// request made before the log existed is recorded as canceled with an
/// unknown origin.
fn close_previous_request(
    conn: &Connection,
    current: &WorkspaceDeletionRequestState,
) -> repo::Result<()> {
    let canceled = cancel_pending_by_resource(conn, &workspace_key(current.id), now_ms())?;
    let Some(requested_at) = current.deletion_requested_at else {
        return Ok(());
    };
    if canceled > 0 {
        return Ok(());
    }
    record_log(
        conn,
        current.id,
        &current.name,
        "canceled",
        current.deletion_requested_by_id,
        None,
        requested_at,
    )
}

/// `scheduleDeletion`: in one transaction, sets the deletion request (`now`,
/// the actor, [`deletion_schedule::next_scheduled_deletion_at`]), closes a
/// previous request and records a `pending` log row for the actor (IP, user
/// agent and source from the ambient audit actor). A started purge ->
/// `workspaceDeletionStarted`, nothing written.
pub async fn schedule_deletion(
    db: &chatbotx_db::Db,
    actor_id: i64,
    id: i64,
    now: i64,
) -> Result<Workspace> {
    let actor = audit_actor();
    tx::write(db, move |tx| {
        let current = find_by_id_for_update(tx, id)?;
        let patch: patch::Patch = vec![
            ("deletionRequestedAt", Sql::Integer(now)),
            ("deletionRequestedById", Sql::Integer(actor_id)),
            (
                "scheduledDeletionAt",
                Sql::Integer(deletion_schedule::next_scheduled_deletion_at(now)),
            ),
        ];
        let (updated, _) = update_in(tx, id, &patch)?;
        if let Some(current) = current
            .as_ref()
            .filter(|c| c.scheduled_deletion_at.is_some())
        {
            close_previous_request(tx, current)?;
        }
        record_log(
            tx,
            id,
            &updated.name,
            "pending",
            Some(actor_id),
            actor.as_ref(),
            now_ms(),
        )?;
        Ok(updated)
    })
    .await
}

/// `cancelDeletion`: clears the request and closes the pending log row
/// (a never-scheduled workspace writes no log row).
pub async fn cancel_deletion(db: &chatbotx_db::Db, id: i64) -> Result<Workspace> {
    tx::write(db, move |tx| {
        let current = find_by_id_for_update(tx, id)?;
        let patch: patch::Patch = vec![
            ("deletionRequestedAt", Sql::Null),
            ("deletionRequestedById", Sql::Null),
            ("scheduledDeletionAt", Sql::Null),
        ];
        let (updated, _) = update_in(tx, id, &patch)?;
        if let Some(current) = current
            .as_ref()
            .filter(|c| c.scheduled_deletion_at.is_some())
        {
            close_previous_request(tx, current)?;
        }
        Ok(updated)
    })
    .await
}

/// `resolveTenantForOwner`: the creator's tenant when it is not the root
/// tenant; else the tenant the creator owns; else the root tenant.
pub fn resolve_tenant_for_owner(conn: &Connection, creator_id: i64) -> repo::Result<i64> {
    let tenant: Option<i64> = conn
        .query_row(
            r#"SELECT "tenantId" FROM "User" WHERE "id" = ?1"#,
            [creator_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(tenant) = tenant.filter(|t| *t != ROOT_TENANT_ID) {
        return Ok(tenant);
    }
    Ok(
        chatbotx_db::repo::tenant::find_by_owner(conn, creator_id)?
            .map_or(ROOT_TENANT_ID, |t| t.id),
    )
}

/// A member `create` adds next to the owner (`members`, role `agent`).
#[derive(Debug, Clone, PartialEq)]
pub struct ExtraMember {
    pub user_id: i64,
    pub permissions: Value,
}

/// `workspaceService.create` input.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateWorkspace {
    pub name: String,
    /// `data.ownerId` (defaults to `created_by`).
    pub owner_id: Option<i64>,
    /// `data.tenantId` (defaults to [`resolve_tenant_for_owner`]).
    pub tenant_id: Option<i64>,
    pub created_by: i64,
    pub members: Vec<ExtraMember>,
}

/// `workspaceMemberService.create` inside a transaction: the row and the
/// `teamMembers` workspace usage.
pub fn insert_member(
    conn: &Connection,
    workspace_id: i64,
    user_id: i64,
    role: &str,
    permissions: &Value,
) -> repo::Result<i64> {
    let id = new_id();
    let now = now_ms();
    conn.execute(
        r#"INSERT INTO "WorkspaceMember" ("id", "createdAt", "updatedAt", "workspaceId", "userId", "role", "permissions", "notificationTypes", "notificationChannels")
           VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
        params![
            id,
            now,
            workspace_id,
            user_id,
            role,
            permissions.to_string(),
            default_notification_types().to_string(),
            default_notification_channels().to_string(),
        ],
    )?;
    workspace_usage::adjust(conn, workspace_id, WorkspaceUsageMetric::TeamMembers, 1)?;
    Ok(id)
}

/// `create`: on community one workspace per owner (purge-pending ones count;
/// 400 `workspaceLimitReached`); one `workspaces` quota unit of the creator
/// (none left -> `workspaceLimitReached`); the row (tenant from the owner),
/// the owner member (role `owner`, every permission) and the extra `agent`
/// members, all in one transaction. Returns the row; the original then
/// audits `create` "created the workspace (#id)" for the creator.
pub async fn create(
    db: &chatbotx_db::Db,
    edition: Edition,
    input: CreateWorkspace,
) -> Result<Workspace> {
    tx::write(db, move |tx| {
        let owner_id = input.owner_id.unwrap_or(input.created_by);
        if edition.is_community() {
            let owned: i64 = tx
                .query_row(
                    r#"SELECT COUNT(*) FROM "Workspace" WHERE "ownerId" = ?1"#,
                    [owner_id],
                    |row| row.get(0),
                )
                .map_err(repo::Error::from)?;
            if owned >= COMMUNITY_MAX_WORKSPACES {
                return Err(workspace_limit_reached());
            }
        }
        if !user_quota::try_consume(tx, input.created_by, QuotaMetric::Workspaces)? {
            return Err(workspace_limit_reached());
        }
        let tenant_id = match input.tenant_id {
            Some(tenant) => tenant,
            None => resolve_tenant_for_owner(tx, input.created_by)?,
        };
        let id = new_id();
        let now = now_ms();
        tx.execute(
            r#"INSERT INTO "Workspace" ("id", "createdAt", "updatedAt", "name", "ownerId", "tenantId") VALUES (?1, ?2, ?2, ?3, ?4, ?5)"#,
            params![id, now, input.name, owner_id, tenant_id],
        )
        .map_err(repo::Error::from)?;
        insert_member(tx, id, input.created_by, "owner", &full_permissions())?;
        for member in &input.members {
            insert_member(tx, id, member.user_id, "agent", &member.permissions)?;
        }
        find(tx, id)?.ok_or_else(|| BusinessError::not_found("Workspace not found"))
    })
    .await
}

/// `ensureFirstForOwner`: the owner's live workspace (`findActiveByOwner`),
/// created (named `name`) when there is none.
pub async fn ensure_first_for_owner(
    db: &chatbotx_db::Db,
    edition: Edition,
    owner_id: i64,
    name: String,
) -> Result<Workspace> {
    let existing = tx::read(db, move |conn| {
        Ok(chatbotx_db::repo::workspace::find_active_by_owner(
            conn, owner_id,
        )?)
    })
    .await?;
    if let Some(workspace) = existing {
        return Ok(workspace);
    }
    create(
        db,
        edition,
        CreateWorkspace {
            name,
            owner_id: Some(owner_id),
            tenant_id: None,
            created_by: owner_id,
            members: Vec::new(),
        },
    )
    .await
}

/// `workspaceMemberService.listByUserId` + `member.workspace`: the
/// workspaces the user is a member of, membership order.
pub async fn list_for_member(db: &chatbotx_db::Db, user_id: i64) -> Result<Vec<Workspace>> {
    tx::read(db, move |conn| {
        let sql = format!(
            r#"{} WHERE "id" IN (SELECT "workspaceId" FROM "WorkspaceMember" WHERE "userId" = ?1)
               ORDER BY (SELECT MIN(m."id") FROM "WorkspaceMember" m WHERE m."workspaceId" = "Workspace"."id" AND m."userId" = ?1)"#,
            Workspace::select_sql()
        );
        Ok(repo::query_models::<Workspace>(conn, &sql, &[&user_id])?)
    })
    .await
}
