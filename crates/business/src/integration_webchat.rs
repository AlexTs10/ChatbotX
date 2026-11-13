//! `integration-webchat/service.ts` (create / find / list / update / delete)
//! and `branding.ts` (`brandWebchatMenus`, community edition).

use chatbotx_db::Db;
use chatbotx_db::rusqlite::types::Value as SqlValue;
use chatbotx_db::rusqlite::{Connection, OptionalExtension as _, Row, params};
use serde_json::{Value, json};

use crate::connection::{self, ConnectionRow};
use crate::{BusinessError, Result, tx};

const COLUMNS: &str = r#""id", "createdAt", "updatedAt", "name", "enable", "authorizedDomains",
    "conversationStarters", "persistentMenus", "brandColor", "hideHeader", "showLogo",
    "hideMessageInput", "customCss", "inboxId", "welcomeFlowId""#;

fn repo_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

fn json_array(text: &str) -> Vec<Value> {
    serde_json::from_str::<Vec<Value>>(text).unwrap_or_default()
}

fn row(r: &Row<'_>) -> chatbotx_db::rusqlite::Result<WebchatRow> {
    let domains: String = r.get(5)?;
    let starters: String = r.get(6)?;
    let menus: String = r.get(7)?;
    Ok(WebchatRow {
        id: r.get(0)?,
        created_at: r.get(1)?,
        updated_at: r.get(2)?,
        name: r.get(3)?,
        enable: r.get(4)?,
        authorized_domains: json_array(&domains)
            .into_iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        conversation_starters: json_array(&starters),
        persistent_menus: json_array(&menus),
        brand_color: r.get(8)?,
        hide_header: r.get(9)?,
        show_logo: r.get(10)?,
        hide_message_input: r.get(11)?,
        custom_css: r.get(12)?,
        inbox_id: r.get(13)?,
        welcome_flow_id: r.get(14)?,
    })
}

fn find(conn: &Connection, id: i64, workspace_id: i64) -> Result<Option<WebchatRow>> {
    conn.query_row(
        &format!(
            r#"SELECT {COLUMNS} FROM "IntegrationWebchat" WHERE "id" = ?1 AND "workspaceId" = ?2"#
        ),
        params![id, workspace_id],
        row,
    )
    .optional()
    .map_err(repo_err)
}

/// `resolveWelcomeFlowId`: an active flow of the workspace, or `404 Welcome
/// flow not found` (private `flowService.findActiveById` query).
fn resolve_welcome_flow_id(
    conn: &Connection,
    flow_id: Option<i64>,
    workspace_id: i64,
) -> Result<Option<i64>> {
    let Some(flow_id) = flow_id else {
        return Ok(None);
    };
    let found = conn
        .query_row(
            r#"SELECT 1 FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1"#,
            params![flow_id, workspace_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(repo_err)?;
    match found {
        Some(()) => Ok(Some(flow_id)),
        None => Err(BusinessError::not_found("Welcome flow not found")),
    }
}

fn to_json_text(values: &[Value]) -> String {
    Value::Array(values.to_vec()).to_string()
}

/// `BRANDING_TITLE`.
pub const BRANDING_TITLE: &str = "⚡ Built with chatbotx.io";

/// `CreateWebchatRequest` after the zod defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct WebchatData {
    pub name: String,
    pub welcome_flow_id: Option<i64>,
    pub authorized_domains: Vec<String>,
    /// `webchatConversationStarter` objects (already validated).
    pub conversation_starters: Vec<Value>,
    /// `webchatPersistentMenu` objects (already validated).
    pub persistent_menus: Vec<Value>,
    pub brand_color: String,
    pub hide_header: bool,
    pub show_logo: bool,
    pub hide_message_input: bool,
    pub custom_css: Option<String>,
    pub enable: bool,
}

/// `UpdateWebchatRequest`: `None` = key omitted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WebchatPatch {
    pub name: Option<String>,
    pub welcome_flow_id: Option<Option<i64>>,
    pub authorized_domains: Option<Vec<String>>,
    pub conversation_starters: Option<Vec<Value>>,
    pub persistent_menus: Option<Vec<Value>>,
    pub brand_color: Option<String>,
    pub hide_header: Option<bool>,
    pub show_logo: Option<bool>,
    pub hide_message_input: Option<bool>,
    pub custom_css: Option<Option<String>>,
    pub enable: Option<bool>,
}

/// An `IntegrationWebchat` row (without `workspaceId` and `auth`).
#[derive(Debug, Clone, PartialEq)]
pub struct WebchatRow {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub name: String,
    pub enable: bool,
    pub authorized_domains: Vec<String>,
    pub conversation_starters: Vec<Value>,
    pub persistent_menus: Vec<Value>,
    pub brand_color: String,
    pub hide_header: bool,
    pub show_logo: bool,
    pub hide_message_input: bool,
    pub custom_css: Option<String>,
    pub inbox_id: i64,
    pub welcome_flow_id: Option<i64>,
}

/// `buildBrandingUrl(appUrl, "webchat", true)`.
pub fn branding_url(app_url: &str) -> String {
    match url::Url::parse(app_url) {
        Ok(mut url) => {
            url.query_pairs_mut()
                .append_pair("ref", "selfhosted")
                .append_pair("channel", "webchat");
            url.to_string()
        }
        Err(_) => app_url.to_string(),
    }
}

/// `brandWebchatMenus` (community edition): the branding URL menu last,
/// exactly once.
pub fn brand_webchat_menus(menus: Vec<Value>, app_url: &str) -> Vec<Value> {
    let url = branding_url(app_url);
    let is_url = |menu: &Value| menu.get("type").and_then(Value::as_str) == Some("url");
    let label_of = |menu: &Value| {
        menu.get("label")
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let url_of = |menu: &Value| menu.get("url").and_then(Value::as_str).map(str::to_string);
    let branding_count = menus
        .iter()
        .filter(|menu| {
            is_url(menu)
                && (label_of(menu).as_deref() == Some(BRANDING_TITLE)
                    || url_of(menu).as_deref() == Some(url.as_str()))
        })
        .count();
    if branding_count == 1
        && menus.last().is_some_and(|last| {
            is_url(last)
                && label_of(last).as_deref() == Some(BRANDING_TITLE)
                && url_of(last).as_deref() == Some(url.as_str())
        })
    {
        return menus;
    }
    let mut kept: Vec<Value> = menus
        .into_iter()
        .filter(|menu| {
            !is_url(menu)
                || (label_of(menu).as_deref() != Some(BRANDING_TITLE)
                    && url_of(menu).as_deref() != Some(url.as_str()))
        })
        .collect();
    // `ensureBrandingMenuEntry`: the filter above removed every entry with
    // that URL, so the entry is always appended.
    kept.push(json!({ "label": BRANDING_TITLE, "type": "url", "url": url }));
    kept
}

/// `integrationWebchatService.create` (existing workspace): the welcome-flow
/// check (`404 Welcome flow not found`), the inbox, the webchat row and the
/// `connected` connection in one transaction.
pub async fn create(
    db: &Db,
    workspace_id: i64,
    created_by: Option<i64>,
    data: WebchatData,
    app_url: &str,
    now: i64,
) -> Result<(WebchatRow, ConnectionRow)> {
    let menus = brand_webchat_menus(data.persistent_menus.clone(), app_url);
    tx::write(db, move |tx| {
        let welcome_flow_id = resolve_welcome_flow_id(tx, data.welcome_flow_id, workspace_id)?;
        let webchat_id: i64 = chatbotx_core::utils::id::create_id()
            .parse()
            .unwrap_or_default();
        connection::insert_inbox(tx, webchat_id, workspace_id, "webchat", &data.name, now)?;
        let domains: Vec<Value> = data.authorized_domains.iter().map(|d| json!(d)).collect();
        tx.execute(
            r#"INSERT INTO "IntegrationWebchat" ("id", "createdAt", "updatedAt", "auth", "name",
                   "enable", "authorizedDomains", "conversationStarters", "persistentMenus",
                   "brandColor", "hideHeader", "showLogo", "hideMessageInput", "customCss",
                   "workspaceId", "inboxId", "welcomeFlowId")
               VALUES (?1, ?2, ?2, '{}', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?1, ?14)"#,
            params![
                webchat_id,
                now,
                data.name,
                data.enable,
                to_json_text(&domains),
                to_json_text(&data.conversation_starters),
                to_json_text(&menus),
                data.brand_color,
                data.hide_header,
                data.show_logo,
                data.hide_message_input,
                data.custom_css,
                workspace_id,
                welcome_flow_id,
            ],
        )
        .map_err(repo_err)?;
        let connection = connection::insert_connected_channel(
            tx,
            workspace_id,
            "webchat",
            webchat_id,
            &data.name,
            created_by,
            now,
        )?;
        let webchat = find(tx, webchat_id, workspace_id)?.ok_or_else(|| {
            BusinessError::Repo(chatbotx_db::repo::Error::msg(
                "IntegrationWebchat row missing",
            ))
        })?;
        Ok((webchat, connection))
    })
    .await
}

/// `findByIdForWorkspace` (`404 Integration Webchat not found`).
pub async fn find_by_id_for_workspace(db: &Db, id: i64, workspace_id: i64) -> Result<WebchatRow> {
    tx::read(db, move |conn| {
        find(conn, id, workspace_id)?
            .ok_or_else(|| BusinessError::not_found("Webchat integration not found"))
    })
    .await
}

/// `list`: rows ordered by id, one page; plus the page count.
pub async fn list(
    db: &Db,
    workspace_id: i64,
    page: i64,
    per_page: i64,
) -> Result<(Vec<WebchatRow>, i64)> {
    tx::read(db, move |conn| {
        let total: i64 = conn
            .query_row(
                r#"SELECT COUNT(*) FROM "IntegrationWebchat" WHERE "workspaceId" = ?1"#,
                [workspace_id],
                |r| r.get(0),
            )
            .map_err(repo_err)?;
        let mut statement = conn
            .prepare(&format!(
                r#"SELECT {COLUMNS} FROM "IntegrationWebchat" WHERE "workspaceId" = ?1
                   ORDER BY "createdAt" DESC, "id" DESC LIMIT ?2 OFFSET ?3"#
            ))
            .map_err(repo_err)?;
        let rows = statement
            .query_map(
                params![workspace_id, per_page, (page.max(1) - 1) * per_page],
                row,
            )
            .map_err(repo_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(repo_err)?;
        Ok((rows, (total + per_page - 1) / per_page))
    })
    .await
}

/// `update`: the supplied keys only (menus branded); a new name also renames
/// the connection.
pub async fn update(
    db: &Db,
    workspace_id: i64,
    id: i64,
    patch: WebchatPatch,
    app_url: &str,
) -> Result<()> {
    let menus = patch
        .persistent_menus
        .clone()
        .map(|menus| brand_webchat_menus(menus, app_url));
    let now = chatbotx_db::repo::now_ms();
    tx::write(db, move |tx| {
        let mut sets: Vec<(&str, SqlValue)> = Vec::new();
        if let Some(name) = &patch.name {
            sets.push(("name", name.clone().into()));
        }
        if let Some(flow_id) = patch.welcome_flow_id {
            let resolved = resolve_welcome_flow_id(tx, flow_id, workspace_id)?;
            sets.push(("welcomeFlowId", resolved.map_or(SqlValue::Null, SqlValue::Integer)));
        }
        if let Some(domains) = &patch.authorized_domains {
            let values: Vec<Value> = domains.iter().map(|d| json!(d)).collect();
            sets.push(("authorizedDomains", to_json_text(&values).into()));
        }
        if let Some(starters) = &patch.conversation_starters {
            sets.push(("conversationStarters", to_json_text(starters).into()));
        }
        if let Some(menus) = &menus {
            sets.push(("persistentMenus", to_json_text(menus).into()));
        }
        if let Some(color) = &patch.brand_color {
            sets.push(("brandColor", color.clone().into()));
        }
        for (column, value) in [
            ("hideHeader", patch.hide_header),
            ("showLogo", patch.show_logo),
            ("hideMessageInput", patch.hide_message_input),
            ("enable", patch.enable),
        ] {
            if let Some(value) = value {
                sets.push((column, i64::from(value).into()));
            }
        }
        if let Some(css) = &patch.custom_css {
            sets.push(("customCss", css.clone().map_or(SqlValue::Null, SqlValue::Text)));
        }
        sets.push(("updatedAt", now.into()));
        let assignments = sets
            .iter()
            .enumerate()
            .map(|(index, (column, _))| format!(r#""{column}" = ?{}"#, index + 3))
            .collect::<Vec<_>>()
            .join(", ");
        let mut binds: Vec<SqlValue> = vec![id.into(), workspace_id.into()];
        binds.extend(sets.into_iter().map(|(_, value)| value));
        tx.execute(
            &format!(
                r#"UPDATE "IntegrationWebchat" SET {assignments} WHERE "id" = ?1 AND "workspaceId" = ?2"#
            ),
            chatbotx_db::rusqlite::params_from_iter(binds.iter()),
        )
        .map_err(repo_err)?;
        if let Some(name) = &patch.name {
            tx.execute(
                r#"UPDATE "Connection" SET "displayName" = ?3, "updatedAt" = ?4
                   WHERE "workspaceId" = ?1 AND "provider" = 'webchat' AND "sourceId" = ?2"#,
                params![workspace_id, id.to_string(), name, now],
            )
            .map_err(repo_err)?;
        }
        Ok(())
    })
    .await
}

/// `delete`: the webchat row and the connection / inbox disconnect.
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    let now = chatbotx_db::repo::now_ms();
    tx::write(db, move |tx| {
        let webchat = find(tx, id, workspace_id)?
            .ok_or_else(|| BusinessError::not_found("Integration Webchat not found"))?;
        tx.execute(
            r#"DELETE FROM "IntegrationWebchat" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
        )
        .map_err(repo_err)?;
        // `connectionStateService.disconnectInbox`.
        let current = tx
            .query_row(
                &format!(
                    r#"SELECT {} FROM "Connection" WHERE "inboxId" = ?1 AND "workspaceId" = ?2"#,
                    connection::COLUMNS
                ),
                params![webchat.inbox_id, workspace_id],
                connection::row,
            )
            .optional()
            .map_err(repo_err)?;
        if let Some(current) = current {
            connection::mark_disconnected(tx, &current, now)?;
        }
        Ok(())
    })
    .await
}
