//! A scripted [`ChannelHttp`] of the Q.P6-U3 tests: one responder over the
//! request (the calls of a batch run concurrently, so a reply is chosen by
//! what the request carries, not by its order).

#![allow(dead_code)]

use std::sync::Mutex;

use bytes::Bytes;
use chatbotx_business::channel_api::{ChannelHttp, Download};
use chatbotx_business::connection::credentials::{UpstreamReply, UpstreamRequest};
use futures::future::BoxFuture;
use serde_json::Value;

type Responder = Box<dyn Fn(&UpstreamRequest) -> UpstreamReply + Send + Sync>;

pub struct FakeHttp {
    respond: Responder,
    downloads: Mutex<Vec<(String, Option<Download>)>>,
    seen: Mutex<Vec<UpstreamRequest>>,
    fetched: Mutex<Vec<(String, Option<String>)>>,
}

impl FakeHttp {
    pub fn new(
        respond: impl Fn(&UpstreamRequest) -> UpstreamReply + Send + Sync + 'static,
    ) -> FakeHttp {
        FakeHttp {
            respond: Box::new(respond),
            downloads: Mutex::new(Vec::new()),
            seen: Mutex::new(Vec::new()),
            fetched: Mutex::new(Vec::new()),
        }
    }

    /// No provider answer at all.
    pub fn offline() -> FakeHttp {
        FakeHttp::new(|_| Err("offline".into()))
    }

    /// `download(url)` answers these bytes (any other URL: `None`).
    pub fn with_download(self, url: &str, bytes: &'static [u8], mime: &str) -> FakeHttp {
        self.downloads.lock().unwrap().push((
            url.to_owned(),
            Some((Bytes::from_static(bytes), Some(mime.to_owned()))),
        ));
        self
    }

    pub fn seen(&self) -> Vec<UpstreamRequest> {
        self.seen.lock().unwrap().clone()
    }

    pub fn urls(&self) -> Vec<String> {
        self.seen().into_iter().map(|r| r.url).collect()
    }

    pub fn fetched(&self) -> Vec<(String, Option<String>)> {
        self.fetched.lock().unwrap().clone()
    }
}

/// The URL and body of a request, for matching.
pub fn text(request: &UpstreamRequest) -> String {
    format!("{} {}", request.url, request.body.as_deref().unwrap_or(""))
}

pub fn ok(body: Value) -> UpstreamReply {
    Ok((200, body.to_string()))
}

pub fn status(code: u16, body: Value) -> UpstreamReply {
    Ok((code, body.to_string()))
}

/// A Graph `{ error }` answer.
pub fn graph_error(
    code: u16,
    message: &str,
    error_code: i64,
    sub_code: Option<i64>,
) -> UpstreamReply {
    let mut error = serde_json::json!({ "message": message, "code": error_code });
    if let Some(sub) = sub_code {
        error["error_subcode"] = sub.into();
    }
    status(code, serde_json::json!({ "error": error }))
}

impl ChannelHttp for FakeHttp {
    fn send(&self, request: UpstreamRequest) -> BoxFuture<'_, UpstreamReply> {
        let reply = (self.respond)(&request);
        self.seen.lock().unwrap().push(request);
        Box::pin(async move { reply })
    }

    fn download(
        &self,
        url: String,
        bearer: Option<String>,
    ) -> BoxFuture<'_, Result<Option<Download>, String>> {
        let found = self
            .downloads
            .lock()
            .unwrap()
            .iter()
            .find(|(u, _)| *u == url)
            .and_then(|(_, d)| d.clone());
        self.fetched.lock().unwrap().push((url, bearer));
        Box::pin(async move { Ok(found) })
    }
}

// ---- channel rows ------------------------------------------------------------

use chatbotx_db::rusqlite::{Connection, params};
use chatbotx_db::{Db, DbError};

/// Inserts an `Inbox` (`id`) and its satellite row (same `id`) of `table`
/// with `identifier` as its source id.
pub fn channel_row(
    tx: &Connection,
    table: &str,
    id: i64,
    workspace_id: i64,
    identifier: &str,
    auth: &Value,
) -> Result<(), DbError> {
    let channel = match table {
        "IntegrationZalo" => "zalo",
        "IntegrationTiktok" => "tiktok",
        "IntegrationMessenger" => "messenger",
        "IntegrationInstagram" => "instagram",
        "IntegrationWhatsapp" => "whatsapp",
        "IntegrationThreads" => "threads",
        "IntegrationTelegram" => "telegram",
        other => panic!("unknown table {other}"),
    };
    tx.execute(
        r#"INSERT INTO "Inbox" ("id", "workspaceId", "name", "channel", "sourceId") VALUES (?1, ?2, ?3, ?4, ?5)"#,
        params![id, workspace_id, format!("Inbox {id}"), channel, format!("inbox-src-{id}")],
    )?;
    let sql = match table {
        "IntegrationZalo" => {
            r#"INSERT INTO "IntegrationZalo" ("id", "auth", "oaId", "name", "workspaceId", "inboxId")
               VALUES (?1, ?2, ?3, 'Zalo OA', ?4, ?1)"#
        }
        "IntegrationTiktok" => {
            r#"INSERT INTO "IntegrationTiktok" ("id", "auth", "openId", "name", "workspaceId", "inboxId")
               VALUES (?1, ?2, ?3, 'TikTok', ?4, ?1)"#
        }
        "IntegrationMessenger" => {
            r#"INSERT INTO "IntegrationMessenger" ("id", "auth", "pageId", "name", "conversationStarters",
                   "persistentMenus", "personas", "workspaceId", "inboxId")
               VALUES (?1, ?2, ?3, 'Page', '[]', '[]', '[]', ?4, ?1)"#
        }
        "IntegrationInstagram" => {
            r#"INSERT INTO "IntegrationInstagram" ("id", "auth", "igId", "pageId", "name", "username",
                   "conversationStarters", "persistentMenus", "workspaceId", "inboxId")
               VALUES (?1, ?2, ?3, 'page-' || ?3, 'IG', 'ig', '[]', '[]', ?4, ?1)"#
        }
        "IntegrationWhatsapp" => {
            r#"INSERT INTO "IntegrationWhatsapp" ("id", "auth", "phoneNumberId", "wabaId", "businessId",
                   "name", "workspaceId", "inboxId")
               VALUES (?1, ?2, ?3, 'waba-1', 'business-1', 'WA', ?4, ?1)"#
        }
        "IntegrationTelegram" => {
            r#"INSERT INTO "IntegrationTelegram" ("id", "auth", "botId", "name", "workspaceId", "inboxId")
               VALUES (?1, ?2, ?3, 'Bot', ?4, ?1)"#
        }
        _ => {
            r#"INSERT INTO "IntegrationThreads" ("id", "auth", "threadsUserId", "username", "name",
                   "workspaceId", "inboxId")
               VALUES (?1, ?2, ?3, 'threads', 'Threads', ?4, ?1)"#
        }
    };
    tx.execute(sql, params![id, auth.to_string(), identifier, workspace_id])?;
    Ok(())
}

/// Inserts a `connected` channel `Connection` (`id`) for an inbox.
pub fn connection_row(
    tx: &Connection,
    id: i64,
    workspace_id: i64,
    provider: &str,
    inbox_id: i64,
    source_id: &str,
) -> Result<(), DbError> {
    tx.execute(
        r#"INSERT INTO "Connection" ("id", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId", "displayName")
           VALUES (?1, ?2, ?3, 'channel', ?4, ?5, ?6, 'Channel')"#,
        params![id, workspace_id, provider, provider, inbox_id, source_id],
    )?;
    Ok(())
}

/// `(auth, tokenRefreshError)` of a satellite row.
pub async fn auth_row(db: &Db, table: &'static str, id: i64) -> (Value, Option<String>) {
    db.read(move |conn| {
        Ok(conn.query_row(
            &format!(r#"SELECT "auth", "tokenRefreshError" FROM "{table}" WHERE "id" = ?1"#),
            [id],
            |r| {
                let auth: String = r.get(0)?;
                Ok((
                    serde_json::from_str(&auth).unwrap_or(Value::Null),
                    r.get(1)?,
                ))
            },
        )?)
    })
    .await
    .expect("read satellite row")
}

/// `(status, statusReason)` of a `Connection`.
pub async fn connection_status(db: &Db, id: i64) -> (String, Option<String>) {
    db.read(move |conn| {
        Ok(conn.query_row(
            r#"SELECT "status", "statusReason" FROM "Connection" WHERE "id" = ?1"#,
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    })
    .await
    .expect("read connection")
}
