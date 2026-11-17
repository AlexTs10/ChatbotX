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
    let app_url = app_url.to_string();
    tx::write(db, move |tx| {
        create_in(tx, workspace_id, created_by, data, &app_url, now)
    })
    .await
}

/// [`create`] on the caller's transaction (the template install).
pub fn create_in(
    tx: &Connection,
    workspace_id: i64,
    created_by: Option<i64>,
    data: WebchatData,
    app_url: &str,
    now: i64,
) -> Result<(WebchatRow, ConnectionRow)> {
    let menus = brand_webchat_menus(data.persistent_menus.clone(), app_url);
    {
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
    }
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
        connection::disconnect_inbox(tx, webchat.inbox_id, workspace_id, now)
    })
    .await
}

// ---- guest access (`webchat-access-token.ts`, `authorized-domain.ts`,
// `guest-conversation-id.ts`) -------------------------------------------------

/// `TOKEN_TTL_SECONDS`.
const ACCESS_TOKEN_TTL_SECONDS: i64 = 30 * 60;

fn normalize_host(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .trim_start_matches('.')
        .trim_end_matches('.')
        .to_string()
}

/// `getHostFromOrigin`.
pub fn host_from_origin(origin: Option<&str>) -> Option<String> {
    let value = origin?.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(url) = url::Url::parse(value)
        && let Some(host) = url.host_str()
    {
        return Some(normalize_host(host));
    }
    let rest = match value.find("://") {
        Some(at) if at > 0 && value[..at].bytes().all(|b| b.is_ascii_alphabetic()) => {
            &value[at + 3..]
        }
        _ => value,
    };
    let host = rest.split(['/', ':', '?', '#']).next().unwrap_or("");
    (!host.is_empty()).then(|| normalize_host(host))
}

/// `isOriginAuthorized`: no origin = a direct open (allowed); else the host
/// or a subdomain of one of the domains.
pub fn is_origin_authorized(origin: Option<&str>, authorized_domains: &[String]) -> bool {
    if origin.is_none() {
        return true;
    }
    let domains: Vec<String> = authorized_domains
        .iter()
        .map(|d| normalize_host(d))
        .filter(|d| !d.is_empty())
        .collect();
    if domains.is_empty() {
        return false;
    }
    let Some(host) = host_from_origin(origin) else {
        return false;
    };
    domains
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

/// `isEmbedOriginAllowed`: an empty allowlist = embeddable anywhere.
pub fn is_embed_origin_allowed(origin: Option<&str>, authorized_domains: &[String]) -> bool {
    authorized_domains.is_empty() || is_origin_authorized(origin, authorized_domains)
}

/// `isFirstPartyOrigin`: no origin, or the app's own host.
pub fn is_first_party_origin(origin: Option<&str>, app_host: &str) -> bool {
    if origin.is_none() {
        return true;
    }
    host_from_origin(origin)
        .is_some_and(|host| !app_host.is_empty() && host == app_host.to_lowercase())
}

/// `isAllowedGuestOrigin`.
pub fn is_allowed_guest_origin(
    parent_origin: Option<&str>,
    authorized_domains: &[String],
    app_host: &str,
) -> bool {
    is_embed_origin_allowed(parent_origin, authorized_domains)
        || is_first_party_origin(parent_origin, app_host)
}

/// `GUEST_CONVERSATION_ID_REGEX`: digits, optionally `:<uuid>`.
pub fn is_valid_guest_conversation_id(value: &str) -> bool {
    let (digits, uuid) = match value.split_once(':') {
        Some((digits, uuid)) => (digits, Some(uuid)),
        None => (value, None),
    };
    let hex_groups = |uuid: &str| {
        let groups: Vec<&str> = uuid.split('-').collect();
        groups.len() == 5
            && groups
                .iter()
                .zip([8, 4, 4, 4, 12])
                .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && uuid.is_none_or(hex_groups)
}

/// `createGuestConversationId`: `<workspaceId>:<random uuid v4>`.
pub fn create_guest_conversation_id(workspace_id: i64) -> String {
    let mut b = [0u8; 16];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut b);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{workspace_id}:{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

fn b64url(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// `createWebchatAccessToken`: `base64url(payload).hmacHex` with
/// `BETTER_AUTH_SECRET` (`secret`).
pub fn create_access_token(
    secret: &str,
    workspace_id: &str,
    webchat_id: &str,
    origin: Option<&str>,
    now_secs: i64,
) -> String {
    // Keys in the original order (alphabetical, as serde_json's map).
    let payload = json!({
        "exp": now_secs + ACCESS_TOKEN_TTL_SECONDS,
        "originHost": host_from_origin(origin),
        "webchatId": webchat_id,
        "workspaceId": workspace_id,
    });
    let encoded = b64url(payload.to_string().as_bytes());
    let signature = chatbotx_core::utils::crypto::hmac_sha256_hex(secret, &encoded);
    format!("{encoded}.{signature}")
}

/// `verifyWebchatAccessToken`: signature, workspace, webchat, origin host
/// and expiry.
pub fn verify_access_token(
    secret: &str,
    token: Option<&str>,
    workspace_id: &str,
    webchat_id: &str,
    origin: Option<&str>,
    now_secs: i64,
) -> bool {
    use base64::Engine as _;
    let Some(token) = token.filter(|t| !t.is_empty()) else {
        return false;
    };
    let mut parts = token.split('.');
    let (Some(encoded), Some(signature)) = (parts.next(), parts.next()) else {
        return false;
    };
    if encoded.is_empty() || signature.is_empty() || secret.is_empty() {
        return false;
    }
    let expected = chatbotx_core::utils::crypto::hmac_sha256_hex(secret, encoded);
    if !chatbotx_core::utils::crypto::timing_safe_string_equal(signature, &expected) {
        return false;
    }
    // Node's base64url decoder also takes padded input.
    let Ok(bytes) =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded.trim_end_matches('='))
    else {
        return false;
    };
    let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    payload["workspaceId"].as_str() == Some(workspace_id)
        && payload["webchatId"].as_str() == Some(webchat_id)
        && payload["originHost"].as_str().map(str::to_string) == host_from_origin(origin)
        && payload["exp"]
            .as_f64()
            .is_some_and(|exp| exp >= now_secs as f64)
}

/// `findByIdForWorkspaceOrNull`.
pub async fn find_by_id_for_workspace_or_null(
    db: &Db,
    id: i64,
    workspace_id: i64,
) -> Result<Option<WebchatRow>> {
    tx::read(db, move |conn| find(conn, id, workspace_id)).await
}

/// The rows `findOrCreateGuestConversation` created.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestConversation {
    pub contact_id: i64,
    pub contact_inbox_id: i64,
    pub conversation_id: i64,
}

/// `findOrCreateGuestConversation` (the create half; the caller looked the
/// contact inbox up first), inside the caller's write transaction: a
/// "Guest" contact, its webchat contact inbox and a conversation, behind
/// the MAC gate. `None` = no MAC room (the original's `null`).
#[allow(clippy::too_many_arguments)]
pub fn create_guest_conversation(
    conn: &Connection,
    workspace_id: i64,
    owner_id: i64,
    inbox_id: i64,
    guest_conversation_id: &str,
    locale: Option<&str>,
    timezone: Option<&str>,
    parent_url: Option<&str>,
    now_ms: i64,
) -> Result<Option<GuestConversation>> {
    use chatbotx_core::variables::system_field::{normalize_locale, normalize_stored_timezone};
    // ponytail: `finalizeContactProfile` without the phone hint and the
    // region timezone table (a guest has no phone; the widget sends its zone).
    let locale = normalize_locale(locale);
    let timezone = normalize_stored_timezone(timezone);
    let language = crate::contact::update_fields::normalize_language(locale.as_deref());
    let created = crate::quota_enforcement::create_new_contact_with_mac(
        conn,
        owner_id,
        workspace_id,
        |conn, period_start| {
            let contact_id = chatbotx_db::repo::new_id();
            conn.execute(
                r#"INSERT INTO "Contact" ("id", "createdAt", "updatedAt", "workspaceId", "email",
                     "gender", "firstName", "lastName", "locale", "timezone")
                   VALUES (?1, ?2, ?2, ?3, ?4, 'unknown', 'Guest', ?5, ?6, ?7)"#,
                params![
                    contact_id,
                    now_ms,
                    workspace_id,
                    guest_conversation_id,
                    crate::contact::create::random_string(10),
                    locale,
                    timezone
                ],
            )
            .map_err(repo_err)?;
            let contact_inbox_id = chatbotx_db::repo::new_id();
            conn.execute(
                r#"INSERT INTO "ContactInbox" ("id", "createdAt", "updatedAt", "inboxId", "contactId",
                     "originalContactId", "source", "sourceId", "channel", "language", "webchatParentUrl")
                   VALUES (?1, ?2, ?2, ?3, ?4, ?4, 'webchat', ?5, 'webchat', ?6, ?7)"#,
                params![
                    contact_inbox_id,
                    now_ms,
                    inbox_id,
                    contact_id,
                    guest_conversation_id,
                    language,
                    parent_url
                ],
            )
            .map_err(repo_err)?;
            // ponytail: `messageCleanupService.cancelByInboxSource` has no
            // Rust store yet (no scheduled message cleanup rows to cancel).
            let conversation_id = chatbotx_db::repo::new_id();
            conn.execute(
                r#"INSERT INTO "Conversation" ("id", "createdAt", "updatedAt", "workspaceId", "contactId")
                   VALUES (?1, ?2, ?2, ?3, ?4)"#,
                params![conversation_id, now_ms, workspace_id, contact_id],
            )
            .map_err(repo_err)?;
            let counted = match period_start {
                Some(period_start) => crate::analytics::mac::claim_new_active_contact(
                    conn,
                    workspace_id,
                    (contact_id, contact_inbox_id),
                    inbox_id,
                    period_start,
                    now_ms,
                )?,
                None => false,
            };
            Ok((
                GuestConversation {
                    contact_id,
                    contact_inbox_id,
                    conversation_id,
                },
                counted,
            ))
        },
    )?;
    Ok(created.ok())
}

#[cfg(test)]
mod guest_tests {
    //! `apps/builder/__tests__/webchat-guest-session.test.ts` (the server
    //! helpers: authorized domains, access token, guest conversation id;
    //! the browser store cases are the SPA's, P4).
    use super::*;

    const WS: &str = "workspace-1";
    const WC: &str = "webchat-1";
    const SECRET: &str = "test-better-auth-secret";
    const NOW: i64 = 1_700_000_000;

    fn mint(origin: Option<&str>) -> String {
        create_access_token(SECRET, WS, WC, origin, NOW)
    }

    fn verify(token: Option<&str>, ws: &str, origin: Option<&str>) -> bool {
        verify_access_token(SECRET, token, ws, WC, origin, NOW)
    }

    #[test]
    fn creates_a_guest_conversation_id_scoped_to_the_workspace_with_an_unguessable_suffix() {
        let first = create_guest_conversation_id(1);
        let second = create_guest_conversation_id(1);
        for id in [&first, &second] {
            let (ws, uuid) = id.split_once(':').unwrap();
            assert_eq!(ws, "1");
            assert!(is_valid_guest_conversation_id(id));
            assert_eq!(uuid.len(), 36);
        }
        assert_ne!(first, second);
        assert!(is_valid_guest_conversation_id("20260101000000001"));
        assert!(!is_valid_guest_conversation_id(
            "11588416937148416:11609574568902656"
        ));
        assert!(!is_valid_guest_conversation_id("not-a-valid-id"));
    }

    #[test]
    fn rejects_origins_when_no_domains_are_configured_fail_closed() {
        assert!(!is_origin_authorized(Some("https://example.com"), &[]));
    }

    #[test]
    fn allows_a_missing_origin_even_when_no_domains_are_configured() {
        assert!(is_origin_authorized(None, &[]));
    }

    #[test]
    fn allows_exact_hosts_and_subdomains() {
        let domains = vec!["example.com".to_string()];
        assert!(is_origin_authorized(Some("https://example.com"), &domains));
        assert!(is_origin_authorized(
            Some("https://www.example.com"),
            &domains
        ));
    }

    #[test]
    fn rejects_mismatched_origins_when_domains_are_configured() {
        let domains = vec!["example.com".to_string()];
        assert!(!is_origin_authorized(
            Some("https://attacker.test"),
            &domains
        ));
    }

    #[test]
    fn allows_a_missing_origin_even_when_domains_are_configured() {
        assert!(is_origin_authorized(None, &["example.com".to_string()]));
    }

    #[test]
    fn authorizes_a_token_minted_for_the_same_workspace_webchat_origin() {
        let token = mint(Some("https://example.com"));
        assert!(verify(Some(&token), WS, Some("https://example.com")));
    }

    #[test]
    fn rejects_a_missing_token() {
        assert!(!verify(None, WS, Some("https://example.com")));
    }

    #[test]
    fn rejects_a_token_whose_workspace_does_not_match() {
        let token = mint(Some("https://example.com"));
        assert!(!verify(
            Some(&token),
            "workspace-2",
            Some("https://example.com")
        ));
    }

    #[test]
    fn rejects_a_tampered_signature() {
        let token = mint(Some("https://example.com"));
        let payload = token.split('.').next().unwrap();
        let tampered = format!("{payload}.deadbeef");
        assert!(!verify(Some(&tampered), WS, Some("https://example.com")));
    }

    #[test]
    fn rejects_a_token_presented_from_a_different_origin() {
        let token = mint(Some("https://example.com"));
        assert!(!verify(Some(&token), WS, Some("https://attacker.test")));
    }

    #[test]
    fn matches_a_bare_origin_against_a_referer_shaped_origin_used_at_mint_time() {
        let token = mint(Some("https://example.com/pricing?ref=ad"));
        assert!(verify(Some(&token), WS, Some("https://example.com")));
    }

    #[test]
    fn matches_a_subdomain_consistent_origin_the_same_way_on_mint_and_verify() {
        let token = mint(Some("https://widget.example.com"));
        assert!(verify(
            Some(&token),
            WS,
            Some("https://widget.example.com/some/path")
        ));
    }

    #[test]
    fn treats_a_null_origin_at_mint_and_verify_as_consistent() {
        let token = mint(None);
        assert!(verify(Some(&token), WS, None));
    }

    #[test]
    fn rejects_when_minted_with_no_origin_but_presented_with_one() {
        let token = mint(None);
        assert!(!verify(Some(&token), WS, Some("https://example.com")));
    }

    #[test]
    fn an_expired_token_is_rejected() {
        let token = mint(None);
        assert!(!verify_access_token(
            SECRET,
            Some(&token),
            WS,
            WC,
            None,
            NOW + 1801
        ));
    }

    #[test]
    fn guest_origin_accepts_the_first_party_host_with_an_allowlist() {
        let domains = vec!["shop.com".to_string()];
        assert!(is_allowed_guest_origin(
            Some("https://app.io"),
            &domains,
            "app.io"
        ));
        assert!(!is_allowed_guest_origin(
            Some("https://evil.com"),
            &domains,
            "app.io"
        ));
        assert!(is_allowed_guest_origin(
            Some("https://evil.com"),
            &[],
            "app.io"
        ));
    }

    // `webchat-message-schema.test.ts`: the `guestConversationId` rule.
    #[test]
    fn accepts_the_minted_and_legacy_guest_ids_and_rejects_anything_else() {
        const WORKSPACE_ID: i64 = 11_530_439_376_896;
        assert!(is_valid_guest_conversation_id(
            &create_guest_conversation_id(WORKSPACE_ID)
        ));
        assert!(is_valid_guest_conversation_id("11616773281153025"));
        assert!(!is_valid_guest_conversation_id("../../not-an-id"));
        assert!(!is_valid_guest_conversation_id(&format!(
            "{WORKSPACE_ID}:guest-1"
        )));
    }
}
