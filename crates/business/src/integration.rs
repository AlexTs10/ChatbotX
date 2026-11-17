//! `packages/business/src/integration/service.ts` (`integrationService`):
//! `listByWorkspaceId`, `findByIdForWorkspace`,
//! `findTokenRefreshErrorsByWorkspaceId`.

use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, Row, params};

use crate::errors::{BusinessError, Result};
use crate::tx;

/// An `Integration` row (the public columns).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationRow {
    pub id: i64,
    pub integration_type: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `TokenRefreshErrorIntegration` (`id` = the channel satellite's id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRefreshError {
    pub id: i64,
    pub channel: String,
    pub name: String,
    pub error: String,
}

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn integration(r: &Row<'_>) -> chatbotx_db::rusqlite::Result<IntegrationRow> {
    Ok(IntegrationRow {
        id: r.get(0)?,
        integration_type: r.get(1)?,
        created_at: r.get(2)?,
        updated_at: r.get(3)?,
    })
}

/// `listByWorkspaceId`: a `metaCatalog` row is listed only with a live
/// (not soft-deleted) catalog.
pub async fn list_by_workspace(db: &Db, workspace_id: i64) -> Result<Vec<IntegrationRow>> {
    tx::read(db, move |conn| {
        let mut statement = conn
            .prepare(
                r#"SELECT i."id", i."integrationType", i."createdAt", i."updatedAt"
                   FROM "Integration" i
                   WHERE i."workspaceId" = ?1 AND (i."integrationType" <> 'metaCatalog'
                       OR EXISTS (SELECT 1 FROM "IntegrationMetaCatalog" c
                                  WHERE c."integrationId" = i."id" AND c."deletedAt" IS NULL))
                   ORDER BY i."id""#,
            )
            .map_err(repo_err)?;
        let rows = statement
            .query_map([workspace_id], integration)
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok(rows)
    })
    .await
}

/// `findByIdForWorkspace` (any type, a hidden `metaCatalog` row too).
pub async fn find_for_workspace(
    db: &Db,
    id: i64,
    workspace_id: i64,
) -> Result<Option<IntegrationRow>> {
    tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "id", "integrationType", "createdAt", "updatedAt"
               FROM "Integration" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
            integration,
        )
        .optional()
        .map_err(repo_err)
    })
    .await
}

/// `findTokenRefreshErrorsByWorkspaceId`: the channel satellites whose
/// `tokenRefreshError` is set, in the original's order (Zalo, TikTok,
/// Instagram, Messenger, WhatsApp, Threads); an Instagram row of
/// `type = 'facebook'` is `instagramFacebook`.
pub async fn token_refresh_errors(db: &Db, workspace_id: i64) -> Result<Vec<TokenRefreshError>> {
    const TABLES: &[(&str, &str)] = &[
        ("IntegrationZalo", "'zalo'"),
        ("IntegrationTiktok", "'tiktok'"),
        (
            "IntegrationInstagram",
            r#"CASE WHEN "type" = 'facebook' THEN 'instagramFacebook' ELSE 'instagram' END"#,
        ),
        ("IntegrationMessenger", "'messenger'"),
        ("IntegrationWhatsapp", "'whatsapp'"),
        ("IntegrationThreads", "'threads'"),
    ];
    tx::read(db, move |conn| {
        let mut out = Vec::new();
        for (table, channel) in TABLES {
            let mut statement = conn
                .prepare(&format!(
                    r#"SELECT "id", {channel}, "name", "tokenRefreshError" FROM "{table}"
                       WHERE "workspaceId" = ?1 AND "tokenRefreshError" IS NOT NULL ORDER BY "id""#
                ))
                .map_err(repo_err)?;
            let rows = statement
                .query_map([workspace_id], |r| {
                    Ok(TokenRefreshError {
                        id: r.get(0)?,
                        channel: r.get(1)?,
                        name: r.get(2)?,
                        error: r.get(3)?,
                    })
                })
                .map_err(repo_err)?;
            for row in rows {
                out.push(row.map_err(repo_err)?);
            }
        }
        Ok(out)
    })
    .await
}
