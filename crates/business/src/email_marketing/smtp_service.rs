//! `integrationSmtpService` (`packages/business/src/integration-smtp`) and
//! `prepareSmtpAuth` / `verifySmtpConnection`
//! (`features/integration-smtp/lib`).
//!
//! Unlike the original (plain JSON), the stored auth is sealed with the
//! process encryption key (DECISIONS 2026-11-17 Q.I3); rows written plain
//! are still read.

use chatbotx_db::Db;
use chatbotx_db::rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use super::smtp::{self, SmtpAuth};
use super::{open_auth, seal_auth};
use crate::errors::{BusinessError, Result};
use crate::{connection, tx};

fn repo_err(e: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(e.into())
}

/// One row (`IntegrationSmtpModel`; `auth` opened).
#[derive(Debug, Clone, PartialEq)]
pub struct SmtpRow {
    pub id: i64,
    pub name: String,
    pub from_address: String,
    pub inbox_id: i64,
    pub auth: Value,
}

/// The validated create / update input (`createSmtpRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmtpInput {
    pub provider: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub from_address: String,
}

fn find(
    conn: &chatbotx_db::rusqlite::Connection,
    id: i64,
    workspace_id: i64,
) -> Result<Option<SmtpRow>> {
    conn.query_row(
        r#"SELECT "id", "name", "fromAddress", "inboxId", "auth" FROM "IntegrationSmtp"
           WHERE "id" = ?1 AND "workspaceId" = ?2"#,
        params![id, workspace_id],
        |r| {
            let auth: String = r.get(4)?;
            Ok(SmtpRow {
                id: r.get(0)?,
                name: r.get(1)?,
                from_address: r.get(2)?,
                inbox_id: r.get(3)?,
                auth: open_auth(&auth).unwrap_or(Value::Null),
            })
        },
    )
    .optional()
    .map_err(repo_err)
}

/// `findByIdForWorkspace` (`404 SMTP integration not found`).
pub async fn find_by_id_for_workspace(db: &Db, id: i64, workspace_id: i64) -> Result<SmtpRow> {
    tx::read(db, move |conn| find(conn, id, workspace_id))
        .await?
        .ok_or_else(|| BusinessError::not_found("SMTP integration not found"))
}

/// `listByWorkspace`: `{ id, name, fromAddress }`, newest first.
pub async fn list_by_workspace(db: &Db, workspace_id: i64) -> Result<Vec<Value>> {
    tx::read(db, move |conn| {
        let mut stmt = conn
            .prepare(
                r#"SELECT "id", "name", "fromAddress" FROM "IntegrationSmtp"
                   WHERE "workspaceId" = ?1 ORDER BY "createdAt" DESC, "id" DESC"#,
            )
            .map_err(repo_err)?;
        let rows = stmt
            .query_map(params![workspace_id], |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?.to_string(),
                    "name": r.get::<_, String>(1)?,
                    "fromAddress": r.get::<_, String>(2)?,
                }))
            })
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok(rows)
    })
    .await
}

/// `resolveSmtpHostAndPort`: a known provider's pinned host / port.
pub fn resolve_host(input: &SmtpInput) -> (String, u16) {
    smtp::provider_host(&input.provider).map_or_else(
        || (input.host.trim().to_owned(), input.port),
        |(h, p)| (h.to_owned(), p),
    )
}

/// `prepareSmtpAuth`: verifies the credentials connect, then the effective
/// host / port. `smtp.errors.connectionFailed` (en) on failure.
pub async fn prepare_auth(input: &SmtpInput) -> Result<(String, u16)> {
    let (host, port) = resolve_host(input);
    let probe = SmtpAuth {
        host: host.clone(),
        port,
        username: input.username.clone(),
        password: input.password.clone(),
    };
    if let Err(err) = smtp::verify(&probe).await {
        tracing::warn!(err = %err, "SMTP verify failed");
        return Err(BusinessError::exception(
            "Could not connect to the SMTP server. Please check your settings.",
            "systemError",
            400,
        ));
    }
    Ok((host, port))
}

fn auth_json(input: &SmtpInput, host: &str, port: u16) -> Value {
    json!({
        "authType": "custom",
        "provider": input.provider,
        "host": host,
        "port": port,
        "username": input.username,
        "password": input.password,
    })
}

/// `connect`: the `smtp` inbox, the row (`id = inboxId = sourceId`) and the
/// `connected` connection. The new id.
pub async fn connect(
    db: &Db,
    workspace_id: i64,
    created_by: Option<i64>,
    input: &SmtpInput,
    host: &str,
    port: u16,
) -> Result<i64> {
    let auth = seal_auth(&auth_json(input, host, port))?;
    let (name, from) = (input.username.clone(), input.from_address.clone());
    let now = chatbotx_db::repo::now_ms();
    let id = chatbotx_db::repo::new_id();
    tx::write(db, move |tx| {
        connection::insert_inbox(tx, id, workspace_id, "smtp", &name, now)?;
        tx.execute(
            r#"INSERT INTO "IntegrationSmtp" ("id", "createdAt", "updatedAt", "auth", "name",
                   "fromAddress", "workspaceId", "inboxId")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?1)"#,
            params![id, now, auth, name, from, workspace_id],
        )
        .map_err(repo_err)?;
        connection::insert_connected_channel(tx, workspace_id, "smtp", id, &name, created_by, now)?;
        Ok(id)
    })
    .await
}

/// `update`: the merged auth (`host || current`, `port || current`), the
/// name (= username) and the from address. `true` = something changed.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    input: &SmtpInput,
    host: &str,
    port: u16,
) -> Result<bool> {
    let current = find_by_id_for_workspace(db, id, workspace_id).await?;
    let host = if host.is_empty() {
        current.auth["host"].as_str().unwrap_or_default().to_owned()
    } else {
        host.to_owned()
    };
    let port = if port == 0 {
        current.auth["port"]
            .as_u64()
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(0)
    } else {
        port
    };
    let auth = auth_json(input, &host, port);
    let changed = auth != current.auth
        || input.username != current.name
        || input.from_address != current.from_address;
    let sealed = seal_auth(&auth)?;
    let (name, from) = (input.username.clone(), input.from_address.clone());
    let now = chatbotx_db::repo::now_ms();
    let updated = tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "IntegrationSmtp" SET "auth" = ?3, "name" = ?4, "fromAddress" = ?5, "updatedAt" = ?6
               WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id, sealed, name, from, now],
        )
        .map_err(repo_err)
    })
    .await?;
    if updated == 0 {
        return Err(BusinessError::exception(
            "SMTP integration not found",
            "systemError",
            400,
        ));
    }
    Ok(changed)
}

/// `disconnect`: the row, then the connection / inbox disconnect.
pub async fn disconnect(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    let row = find_by_id_for_workspace(db, id, workspace_id).await?;
    let now = chatbotx_db::repo::now_ms();
    tx::write(db, move |tx| {
        tx.execute(
            r#"DELETE FROM "IntegrationSmtp" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
        )
        .map_err(repo_err)?;
        connection::disconnect_inbox(tx, row.inbox_id, workspace_id, now)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(provider: &str) -> SmtpInput {
        SmtpInput {
            provider: provider.into(),
            host: " mail.example.com ".into(),
            port: 2525,
            username: "u".into(),
            password: "p".into(),
            from_address: "a@b.co".into(),
        }
    }

    #[test]
    fn resolves_pinned_hosts() {
        assert_eq!(
            resolve_host(&input("google")),
            ("smtp.gmail.com".into(), 587)
        );
        assert_eq!(
            resolve_host(&input("other")),
            ("mail.example.com".into(), 2525)
        );
    }
}
