//! `packages/business/src/external-webhook/service.ts`: Make / n8n hook
//! registrations (delivered by the `make` / `triggerN8n` flow steps).

use chatbotx_db::Db;
use chatbotx_db::repo::models::ExternalWebhook;
use chatbotx_db::repo::{self, new_id, now_ms};
use chatbotx_db::rusqlite::params;

use crate::errors::{BusinessError, Result};
use crate::tx;

const MAX_WEBHOOKS_PER_WORKSPACE: i64 = 50;

fn sql(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `register`: the existing row for the same `(event, url)`, else a new one.
pub async fn register(
    db: &Db,
    workspace_id: i64,
    provider: String,
    event: String,
    url: String,
) -> Result<ExternalWebhook> {
    crate::net::ssrf_guard::assert_public_url(&url, "Webhook URL")
        .await
        .map_err(|message| BusinessError::exception(message, "invalidRequestData", 422))?;
    tx::write(db, move |c| {
        let find = |c: &chatbotx_db::rusqlite::Connection| {
            repo::find_one::<ExternalWebhook>(
                c,
                r#""workspaceId" = ?1 AND "event" = ?2 AND "url" = ?3"#,
                &[&workspace_id, &event, &url],
            )
        };
        if let Some(existing) = find(c)? {
            return Ok(existing);
        }
        let count: i64 = c
            .query_row(
                r#"SELECT count(*) FROM "ExternalWebhook" WHERE "workspaceId" = ?1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .map_err(sql)?;
        if count >= MAX_WEBHOOKS_PER_WORKSPACE {
            return Err(BusinessError::exception(
                format!(
                    "Workspace has reached the maximum of {MAX_WEBHOOKS_PER_WORKSPACE} external webhooks"
                ),
                "externalWebhookLimitReached",
                400,
            ));
        }
        let now = now_ms();
        c.execute(
            r#"INSERT INTO "ExternalWebhook" ("id", "createdAt", "updatedAt", "provider", "event", "url", "workspaceId")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6) ON CONFLICT DO NOTHING"#,
            params![new_id(), now, provider, event, url, workspace_id],
        )
        .map_err(sql)?;
        find(c)?.ok_or_else(|| BusinessError::not_found("ExternalWebhook not found"))
    })
    .await
}

/// `unregister`.
pub async fn unregister(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    tx::write(db, move |c| {
        let n = c
            .execute(
                r#"DELETE FROM "ExternalWebhook" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![id, workspace_id],
            )
            .map_err(sql)?;
        if n == 0 {
            return Err(BusinessError::not_found("External webhook not found"));
        }
        Ok(())
    })
    .await
}

/// `listByWorkspaceId`.
pub async fn list(db: &Db, workspace_id: i64) -> Result<Vec<ExternalWebhook>> {
    tx::read(db, move |c| {
        Ok(repo::find_many(
            c,
            r#"WHERE "workspaceId" = ?1 ORDER BY "id""#,
            &[&workspace_id],
        )?)
    })
    .await
}

/// `listByWorkspaceAndEvents` (for the `make` / `triggerN8n` steps).
pub async fn list_by_events(
    db: &Db,
    workspace_id: i64,
    events: Vec<String>,
) -> Result<Vec<ExternalWebhook>> {
    if events.is_empty() {
        return Ok(Vec::new());
    }
    let events = serde_json::to_string(&events).unwrap_or_default();
    tx::read(db, move |c| {
        Ok(repo::find_many(
            c,
            r#"WHERE "workspaceId" = ?1 AND "event" IN (SELECT value FROM json_each(?2)) ORDER BY "id""#,
            &[&workspace_id, &events],
        )?)
    })
    .await
}
