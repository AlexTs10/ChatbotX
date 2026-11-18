//! The installer side that needs no install engine: the installation list
//! (`templateService.listInstallationsPage`), the auto-update and publisher
//! grant switches and the sync history (`templateSyncRequestService`).

use chatbotx_db::Db;
use chatbotx_db::repo::template_uninstall;
use chatbotx_db::rusqlite::{
    Connection, OptionalExtension, params, params_from_iter, types::Value as Sql,
};
use serde_json::{Map, Value, json};

use super::permissions::{
    Actor, assert_can_use_categories, assert_can_write_installs, visible_install_categories,
};
use super::{ACTIVE_STATUSES_SQL, db_error, json_col};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `INSTALLATIONS_PAGE_SIZE`.
const PAGE_SIZE: i64 = 50;
/// `SYNC_HISTORY_LIMIT`.
const SYNC_HISTORY_LIMIT: i64 = 50;

/// `TEMPLATE_INSTALLER_WARNING_CODES`.
const INSTALLER_WARNING_CODES: &[&str] = &[
    "aiFileTooLarge",
    "aiFileUnavailable",
    "assigneeRequired",
    "backupTooLarge",
    "backupUnavailable",
    "botFieldRenamed",
    "categoryPermissionDenied",
    "couponTopicExpired",
    "credentialRequired",
    "customFieldRenamed",
    "enqueueFailed",
    "flowInvalidStructure",
    "flowNotPublished",
    "flowSkippedRequiredReference",
    "inboxBindingRequired",
    "localModifiedOverwritten",
    "name",
    "noEmbeddingProvider",
    "notUpdatable",
    "postSelectionRequired",
    "questionImage",
    "quota",
    "savedReplyImage",
    "workspaceUnavailable",
];

/// The keys of the public `uninstallCounts` object, in schema order.
const UNINSTALL_COUNT_KEYS: &[&str] = &[
    "deleted",
    "deletedModified",
    "deletedUnverified",
    "keptShared",
    "keptHasDependents",
    "blockedByReference",
    "keptPartlyRemoved",
    "keptLockedByOther",
    "absent",
    "failed",
    "keptModified",
    "keptByPolicy",
];

fn installation_not_found() -> BusinessError {
    BusinessError::not_found("Template installation not found")
}

fn not_syncable() -> BusinessError {
    BusinessError::exception(
        "This template installation can not be synced",
        "templateInstallationNotSyncable",
        409,
    )
}

/// `parseTemplateUninstallCounts` as the public object (`null` when unset or
/// not a counts object; a missing count is 0).
pub fn uninstall_counts(value: &Value) -> Value {
    let Value::Object(map) = value else {
        return Value::Null;
    };
    let mut out = Map::new();
    for key in UNINSTALL_COUNT_KEYS {
        match map.get(*key) {
            None => {
                out.insert((*key).to_string(), json!(0));
            }
            Some(Value::Number(n)) if n.as_u64().is_some() => {
                out.insert((*key).to_string(), Value::Number(n.clone()));
            }
            Some(_) => return Value::Null,
        }
    }
    Value::Object(out)
}

/// `holdsCategoryOutside(visible)` as an SQL condition on `i`.
fn holds_category_outside(visible: &[&str]) -> String {
    let not_in = |column: &str| {
        if visible.is_empty() {
            String::new()
        } else {
            let list = visible
                .iter()
                .map(|c| format!("'{c}'"))
                .collect::<Vec<_>>()
                .join(", ");
            format!(" AND {column} NOT IN ({list})")
        }
    };
    format!(
        r#"(EXISTS (SELECT 1 FROM "TemplateInstalledResource" r WHERE r."installationId" = i."id"{})
           OR (NOT EXISTS (SELECT 1 FROM "TemplateInstalledResource" r WHERE r."installationId" = i."id")
               AND (NOT EXISTS (SELECT 1 FROM "Template" s WHERE s."id" = i."sourceTemplateId")
                    OR EXISTS (SELECT 1 FROM "Template" s, json_each(s."categoryCounts") c
                               WHERE s."id" = i."sourceTemplateId" AND c.value > 0{}))))"#,
        not_in(r#"r."category""#),
        not_in("c.key")
    )
}

/// `listInstallationsPage` input.
#[derive(Debug, Clone, Default)]
pub struct ListInstallations {
    pub status: Option<String>,
    pub active_only: bool,
    pub before_id: Option<i64>,
    pub limit: Option<i64>,
}

const HAS_ACTIVE_SYNC: &str = r#"EXISTS (SELECT 1 FROM "TemplateInstallationSync" y
    WHERE y."installationId" = i."id" AND y."status" IN ('pending', 'running'))"#;

/// `listInstallationsPage` with the public row shape (`activeSync` dropped,
/// `uninstalledByKind` as `uninstalledBy`).
pub async fn list_page(
    db: &Db,
    workspace_id: i64,
    input: ListInstallations,
    actor: &Actor,
) -> Result<Value> {
    let limit = input.limit.unwrap_or(PAGE_SIZE);
    let codes = INSTALLER_WARNING_CODES
        .iter()
        .map(|c| format!("'{c}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!(
        r#"SELECT i."id", i."templateName", i."status", i."errorCode", i."resourceCount",
                  i."warningCount",
                  (SELECT count(*) FROM json_each(i."warnings") w
                     WHERE json_extract(w.value, '$.entityKind') IN ({codes})),
                  i."createdAt", i."completedAt", i."autoUpdate", i."installSource",
                  i."uninstalledByKind", i."uninstallState", i."uninstallCounts",
                  coalesce(i."sourceDeletedAt" IS NULL AND i."sourceVersion" < t."version", 0),
                  (i."sourceDeletedAt" IS NOT NULL OR i."templateId" IS NULL)
           FROM "TemplateInstallation" i LEFT JOIN "Template" t ON t."id" = i."templateId"
           WHERE i."workspaceId" = ?"#
    );
    let mut values = vec![Sql::Integer(workspace_id)];
    if let Some(status) = input.status {
        sql.push_str(r#" AND i."status" = ?"#);
        values.push(Sql::Text(status));
    }
    if input.active_only {
        sql.push_str(&format!(
            r#" AND (i."status" IN ('pending', 'installing') OR {HAS_ACTIVE_SYNC})"#
        ));
    }
    if let Some(before) = input.before_id {
        sql.push_str(r#" AND i."id" < ?"#);
        values.push(Sql::Integer(before));
    }
    if let Some(visible) = visible_install_categories(actor) {
        sql.push_str(&format!(" AND NOT {}", holds_category_outside(&visible)));
    }
    sql.push_str(r#" ORDER BY i."id" DESC LIMIT ?"#);
    values.push(Sql::Integer(limit + 1));
    tx::read(db, move |conn| {
        let mut stmt = conn.prepare(&sql).map_err(db_error)?;
        let mut rows = stmt
            .query_map(params_from_iter(values), |row| {
                let date = |index: usize| -> chatbotx_db::rusqlite::Result<Value> {
                    Ok(row
                        .get::<_, Option<i64>>(index)?
                        .map_or(Value::Null, |ms| Value::String(chatbotx_db::repo::iso(ms))))
                };
                Ok(json!({
                    "id": row.get::<_, i64>(0)?.to_string(),
                    "templateName": row.get::<_, String>(1)?,
                    "status": row.get::<_, String>(2)?,
                    "errorCode": row.get::<_, Option<String>>(3)?,
                    "resourceCount": row.get::<_, i64>(4)?,
                    "warningCount": row.get::<_, i64>(5)?,
                    "installerWarningCount": row.get::<_, i64>(6)?,
                    "createdAt": date(7)?,
                    "completedAt": date(8)?,
                    "autoUpdate": row.get::<_, bool>(9)?,
                    "installSource": row.get::<_, String>(10)?,
                    "uninstalledBy": row.get::<_, Option<String>>(11)?,
                    "uninstallState": row.get::<_, String>(12)?,
                    "uninstallCounts": uninstall_counts(&json_col(row, 13)?),
                    "updateAvailable": row.get::<_, bool>(14)?,
                    "sourceDeleted": row.get::<_, bool>(15)?,
                }))
            })
            .map_err(db_error)?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        let has_more = rows.len() as i64 > limit;
        rows.truncate(limit as usize);
        let next = if has_more {
            rows.last().map_or(Value::Null, |row| row["id"].clone())
        } else {
            Value::Null
        };
        Ok(json!({ "items": rows, "nextCursor": next }))
    })
    .await
}

/// What the settings switches read of an installation.
struct Installation {
    id: i64,
    status: String,
    template_id: Option<i64>,
    source_template_id: Option<i64>,
    source_deleted_at: Option<i64>,
}

fn find_installation(conn: &Connection, workspace_id: i64, id: i64) -> Result<Installation> {
    conn.query_row(
        r#"SELECT "id", "status", "templateId", "sourceTemplateId", "sourceDeletedAt"
           FROM "TemplateInstallation" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        params![id, workspace_id],
        |row| {
            Ok(Installation {
                id: row.get(0)?,
                status: row.get(1)?,
                template_id: row.get(2)?,
                source_template_id: row.get(3)?,
                source_deleted_at: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(db_error)?
    .ok_or_else(installation_not_found)
}

fn strings(conn: &Connection, sql: &str, values: Vec<Sql>) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(sql).map_err(db_error)?;
    stmt.query_map(params_from_iter(values), |row| row.get::<_, String>(0))
        .map_err(db_error)?
        .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
        .map_err(db_error)
}

/// `setAutoUpdate`.
pub async fn set_auto_update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    actor: &Actor,
    auto_update: bool,
) -> Result<()> {
    assert_can_write_installs(actor)?;
    let actor = actor.clone();
    tx::write(db, move |tx| {
        let installation = find_installation(tx, workspace_id, id)?;
        if installation.status == "failed" || installation.status == "uninstalled" {
            return Err(not_syncable());
        }
        if !matches!(installation.status.as_str(), "completed" | "partial") {
            return Err(BusinessError::exception(
                "Wait for the installation to finish before changing auto-update",
                "templateInstallationNotReady",
                409,
            ));
        }
        if auto_update && (installation.source_deleted_at.is_some() || installation.template_id.is_none()) {
            return Err(not_syncable());
        }
        // `assertCanManageInstallation`: every category the copy holds.
        let categories = strings(
            tx,
            r#"SELECT DISTINCT "category" FROM "TemplateInstalledResource" WHERE "installationId" = ?"#,
            vec![Sql::Integer(installation.id)],
        )?;
        assert_can_use_categories(&actor, categories.iter().map(String::as_str))?;
        tx.execute(
            r#"UPDATE "TemplateInstallation" SET "autoUpdate" = ?1,
                 "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER) WHERE "id" = ?2"#,
            params![auto_update, installation.id],
        )
        .map_err(db_error)?;
        Ok(())
    })
    .await
}

/// `setPublisherUninstallGrant` for every copy of the (workspace, template)
/// pair. Withdrawing it skips a publisher uninstall that still waits.
pub async fn set_publisher_uninstall_grant(
    db: &Db,
    workspace_id: i64,
    id: i64,
    actor: &Actor,
    granted: bool,
) -> Result<()> {
    assert_can_write_installs(actor)?;
    let actor = actor.clone();
    tx::write(db, move |tx| {
        let installation = find_installation(tx, workspace_id, id)?;
        let Some(source) = installation.source_template_id else {
            return Ok(());
        };
        let in_flight: bool = tx
            .query_row(
                r#"SELECT EXISTS (SELECT 1 FROM "TemplateInstallation" WHERE "workspaceId" = ?1
                     AND "sourceTemplateId" = ?2 AND "status" IN ('pending', 'installing'))"#,
                params![workspace_id, source],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if in_flight {
            return Err(BusinessError::exception(
                "An install of this template is in progress",
                "templateInstallationInProgress",
                409,
            ));
        }
        let categories = strings(
            tx,
            &format!(
                r#"SELECT DISTINCT r."category" FROM "TemplateInstalledResource" r
                   JOIN "TemplateInstallation" i ON i."id" = r."installationId"
                   WHERE i."workspaceId" = ? AND i."sourceTemplateId" = ?
                     AND i."status" IN {ACTIVE_STATUSES_SQL} AND r."detachedAt" IS NULL"#
            ),
            vec![Sql::Integer(workspace_id), Sql::Integer(source)],
        )?;
        assert_can_use_categories(&actor, categories.iter().map(String::as_str))?;
        if granted {
            // `assertGrantAllowedForCopy`: never across tenants.
            let tenants: Option<(i64, i64)> = tx
                .query_row(
                    r#"SELECT t."tenantId", w."tenantId" FROM "Template" t, "Workspace" w
                       WHERE t."id" = ?1 AND w."id" = ?2"#,
                    params![source, workspace_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(db_error)?;
            if tenants.is_some_and(|(template, target)| template != target) {
                return Err(BusinessError::exception(
                    "A template from another tenant can not be given access to this workspace",
                    "templatePublisherGrantCrossTenant",
                    409,
                ));
            }
        } else {
            // `cancelPublisherUninstalls`: a publisher uninstall nobody claimed
            // yet is skipped (the single-transaction uninstall commits nothing
            // before it runs; one a worker holds finishes).
            for (pending, operation) in
                template_uninstall::list_pending_publisher_uninstalls(tx, workspace_id, source)?
            {
                template_uninstall::skip_pending(tx, pending, "templateUninstallNotGranted")?;
                if let Some(operation) = operation {
                    template_uninstall::finalize_operation_if_complete(tx, operation)?;
                }
            }
        }
        tx.execute(
            r#"UPDATE "TemplateInstallation" SET "publisherUninstallGrant" = ?1,
                 "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)
               WHERE "workspaceId" = ?2 AND "sourceTemplateId" = ?3"#,
            params![i64::from(granted), workspace_id, source],
        )
        .map_err(db_error)?;
        Ok(())
    })
    .await
}

const SYNC_COLUMNS: &str = r#""id", "status", "trigger", "requestedVersion", "fromVersion",
    "appliedVersion", "errorCode", "counts", "createdAt", "finishedAt", "warnings", "installationId""#;

/// `toSyncSummary` of a `TemplateInstallationSync` row, plus its raw warnings
/// and installation id.
fn sync_of(
    row: &chatbotx_db::rusqlite::Row<'_>,
) -> chatbotx_db::rusqlite::Result<(Value, Value, i64)> {
    let opt_date =
        |ms: Option<i64>| ms.map_or(Value::Null, |ms| Value::String(chatbotx_db::repo::iso(ms)));
    let summary = json!({
        "id": row.get::<_, i64>(0)?.to_string(),
        "status": row.get::<_, String>(1)?,
        "trigger": row.get::<_, String>(2)?,
        "requestedVersion": row.get::<_, i64>(3)?,
        "fromVersion": row.get::<_, Option<i64>>(4)?,
        "appliedVersion": row.get::<_, Option<i64>>(5)?,
        "errorCode": row.get::<_, Option<String>>(6)?,
        "counts": sync_counts(&json_col(row, 7)?),
        "createdAt": opt_date(Some(row.get(8)?)),
        "finishedAt": opt_date(row.get(9)?),
    });
    Ok((summary, json_col(row, 10)?, row.get(11)?))
}

/// The public `counts` object (`syncCountsSchema` keys only).
fn sync_counts(value: &Value) -> Value {
    let Value::Object(map) = value else {
        return Value::Null;
    };
    let keys = [
        "created",
        "updated",
        "detached",
        "overwrittenModified",
        "notUpdatable",
    ];
    Value::Object(
        keys.iter()
            .map(|key| {
                (
                    (*key).to_string(),
                    map.get(*key).cloned().unwrap_or(Value::Null),
                )
            })
            .collect(),
    )
}

/// `templateInstallations.listSyncs`: the newest 50 syncs, as summaries.
pub async fn list_syncs(db: &Db, workspace_id: i64, installation_id: i64) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        find_installation(conn, workspace_id, installation_id)?;
        let mut stmt = conn
            .prepare(&format!(
                r#"SELECT {SYNC_COLUMNS} FROM "TemplateInstallationSync"
                   WHERE "installationId" = ?1 AND "workspaceId" = ?2 ORDER BY "id" DESC LIMIT ?3"#
            ))
            .map_err(db_error)?;
        stmt.query_map(
            params![installation_id, workspace_id, SYNC_HISTORY_LIMIT],
            |row| Ok(sync_of(row)?.0),
        )
        .map_err(db_error)?
        .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
        .map_err(db_error)
    })
    .await
}

/// `templateInstallations.getSync`: the summary with warnings and backups.
pub async fn get_sync(
    db: &Db,
    workspace_id: i64,
    installation_id: i64,
    sync_id: i64,
) -> Result<Value> {
    tx::read(db, move |conn| {
        let found = conn
            .query_row(
                &format!(r#"SELECT {SYNC_COLUMNS} FROM "TemplateInstallationSync" WHERE "id" = ?1 AND "workspaceId" = ?2"#),
                params![sync_id, workspace_id],
                sync_of,
            )
            .optional()
            .map_err(db_error)?;
        let Some((mut summary, warnings, _)) = found.filter(|f| f.2 == installation_id) else {
            return Err(BusinessError::not_found("Template sync not found"));
        };
        let warnings: Vec<Value> = warnings
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|w| {
                        json!({
                            "category": w.get("category").cloned().unwrap_or(Value::Null),
                            "code": w.get("entityKind").cloned().unwrap_or(Value::Null),
                            "path": w.get("path").cloned().unwrap_or(Value::Null),
                            "value": w.get("value").cloned().unwrap_or(Value::Null),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut stmt = conn
            .prepare(
                r#"SELECT "id", "resourceKind", "resourceId", "byteSize" FROM "TemplateResourceBackup"
                   WHERE "syncId" = ?1 ORDER BY "id""#,
            )
            .map_err(db_error)?;
        let backups = stmt
            .query_map(params![sync_id], |row| {
                Ok(json!({
                    "id": row.get::<_, i64>(0)?.to_string(),
                    "resourceKind": row.get::<_, String>(1)?,
                    "resourceId": row.get::<_, i64>(2)?.to_string(),
                    "byteSize": row.get::<_, i64>(3)?,
                }))
            })
            .map_err(db_error)?
            .collect::<chatbotx_db::rusqlite::Result<Vec<_>>>()
            .map_err(db_error)?;
        summary["warnings"] = Value::Array(warnings);
        summary["backups"] = Value::Array(backups);
        Ok(summary)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstall_counts_default_and_reject() {
        let counts = uninstall_counts(&json!({ "deleted": 2, "extra": 1 }));
        assert_eq!(counts["deleted"], 2);
        assert_eq!(counts["failed"], 0);
        assert!(counts.get("extra").is_none());
        assert_eq!(uninstall_counts(&json!({ "deleted": -1 })), Value::Null);
        assert_eq!(uninstall_counts(&Value::Null), Value::Null);
    }
}
