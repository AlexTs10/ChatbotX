//! `packages/utils/src/connection.ts`: connection / connect-session
//! vocabularies, the connection config field catalog, and the connect
//! error redirect helpers.

use serde::{Deserialize, Serialize};
use serde_json::Value;

string_enum! {
    /// `connectionStatuses`.
    pub enum ConnectionStatus {
        Connected = "connected",
        Degraded = "degraded",
        NeedsReauth = "needs_reauth",
        Paused = "paused",
        Disconnected = "disconnected",
    }
}

/// `ACTIVE_CONNECTION_STATUSES`.
pub const ACTIVE_CONNECTION_STATUSES: &[ConnectionStatus] =
    &[ConnectionStatus::Connected, ConnectionStatus::Degraded];
/// `INACTIVE_CONNECTION_STATUSES`.
pub const INACTIVE_CONNECTION_STATUSES: &[ConnectionStatus] = &[
    ConnectionStatus::NeedsReauth,
    ConnectionStatus::Paused,
    ConnectionStatus::Disconnected,
];

/// Whether a status is in the active partition.
pub fn is_active_connection_status(status: ConnectionStatus) -> bool {
    ACTIVE_CONNECTION_STATUSES.contains(&status)
}

string_enum! {
    /// `connectionStatusReasons`.
    pub enum ConnectionStatusReason {
        Manual = "manual",
        WorkspacePurge = "workspace_purge",
        TrialExpired = "trial_expired",
        TenantSuspended = "tenant_suspended",
        TokenRevoked = "token_revoked",
        ProviderRevoked = "provider_revoked",
        ProviderOffboarded = "provider_offboarded",
        RefreshFailed = "refresh_failed",
        VerifyFailed = "verify_failed",
        QuotaExceeded = "quota_exceeded",
        OrphanedWebhook = "orphaned_webhook",
    }
}

string_enum! {
    /// `providerAccountEventKinds`.
    pub enum ProviderAccountEventKind {
        Revoked = "revoked",
        Offboarded = "offboarded",
        Reconnected = "reconnected",
    }
}

/// `isProviderOffboardedConnection`.
pub fn is_provider_offboarded_connection(
    status: Option<ConnectionStatus>,
    reason: Option<ConnectionStatusReason>,
) -> bool {
    status == Some(ConnectionStatus::Degraded)
        && reason == Some(ConnectionStatusReason::ProviderOffboarded)
}

/// `PROVIDER_ACCOUNT_TARGET_BATCH_SIZE`.
pub const PROVIDER_ACCOUNT_TARGET_BATCH_SIZE: usize = 200;

string_enum! {
    /// `inboxDisconnectReasons`.
    pub enum InboxDisconnectReason {
        Manual = "manual",
        WorkspacePurge = "workspace_purge",
        TrialExpired = "trial_expired",
        TenantSuspended = "tenant_suspended",
        TokenRevoked = "token_revoked",
    }
}

/// `CONNECTION_TO_INBOX_DISCONNECT_REASON[reason]`.
pub fn connection_to_inbox_disconnect_reason(
    reason: ConnectionStatusReason,
) -> InboxDisconnectReason {
    use ConnectionStatusReason as R;
    use InboxDisconnectReason as I;
    match reason {
        R::Manual => I::Manual,
        R::WorkspacePurge => I::WorkspacePurge,
        R::TrialExpired => I::TrialExpired,
        R::TenantSuspended => I::TenantSuspended,
        R::TokenRevoked => I::TokenRevoked,
        R::ProviderRevoked => I::TokenRevoked,
        R::ProviderOffboarded => I::Manual,
        R::RefreshFailed => I::TokenRevoked,
        R::VerifyFailed => I::Manual,
        R::QuotaExceeded => I::Manual,
        R::OrphanedWebhook => I::Manual,
    }
}

string_enum! {
    /// `connectionKinds`.
    pub enum ConnectionKind {
        Channel = "channel",
        Integration = "integration",
    }
}

/// A field of a connection config item (`items.fields[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfigItemField {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `connectionConfigItemSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfigItem {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<ConnectionConfigItemField>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// `connectionConfigFieldSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionConfigField {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<ConnectionConfigItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The JSON Schema subset `z.toJSONSchema` emits for a provider config
/// object (the Rust input in place of the zod object; property order is
/// kept).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct JsonSchema {
    pub kind: Option<String>,
    pub any_of: Vec<JsonSchema>,
    pub one_of: Vec<JsonSchema>,
    pub const_value: Option<Value>,
    pub default: Option<Value>,
    pub description: Option<String>,
    pub enum_values: Option<Vec<Value>>,
    pub format: Option<String>,
    pub pattern: Option<String>,
    pub properties: Option<Vec<(String, JsonSchema)>>,
    pub required: Option<Vec<String>>,
    pub items: Option<Box<JsonSchema>>,
}

impl JsonSchema {
    /// `{ "type": kind }`.
    pub fn of_type(kind: &str) -> JsonSchema {
        JsonSchema {
            kind: Some(kind.to_owned()),
            ..JsonSchema::default()
        }
    }
}

fn merge_schemas(schemas: &[JsonSchema]) -> JsonSchema {
    if schemas.len() == 1 {
        return schemas[0].clone();
    }
    let mut property_names: Vec<String> = Vec::new();
    for schema in schemas {
        for (name, _) in schema.properties.iter().flatten() {
            if !property_names.contains(name) {
                property_names.push(name.clone());
            }
        }
    }
    let required: Vec<String> = property_names
        .iter()
        .filter(|name| {
            schemas
                .iter()
                .all(|s| s.required.as_ref().is_some_and(|r| r.contains(name)))
        })
        .cloned()
        .collect();
    let values: Vec<Value> = schemas
        .iter()
        .flat_map(|s| match (&s.enum_values, &s.const_value) {
            (Some(values), _) => values.clone(),
            (None, Some(c)) => vec![c.clone()],
            (None, None) => Vec::new(),
        })
        .collect();
    let mut types: Vec<String> = Vec::new();
    for kind in schemas
        .iter()
        .filter_map(|s| s.kind.clone())
        .filter(|k| !k.is_empty())
    {
        if !types.contains(&kind) {
            types.push(kind);
        }
    }
    let description = schemas
        .iter()
        .find_map(|s| s.description.clone().filter(|d| !d.is_empty()));
    JsonSchema {
        kind: (types.len() == 1).then(|| types[0].clone()),
        enum_values: (!values.is_empty()).then_some(values),
        properties: schemas.iter().any(|s| s.properties.is_some()).then(|| {
            property_names
                .iter()
                .map(|name| {
                    let parts: Vec<JsonSchema> = schemas
                        .iter()
                        .filter_map(|s| {
                            s.properties
                                .as_ref()
                                .and_then(|p| p.iter().find(|(n, _)| n == name))
                                .map(|(_, schema)| schema.clone())
                        })
                        .collect();
                    (name.clone(), merge_schemas(&parts))
                })
                .collect()
        }),
        required: (!required.is_empty()).then_some(required),
        description,
        ..JsonSchema::default()
    }
}

fn resolve_non_null_schema(schema: &JsonSchema) -> JsonSchema {
    let variants: Vec<JsonSchema> = schema
        .any_of
        .iter()
        .chain(&schema.one_of)
        .filter(|item| item.kind.as_deref() != Some("null"))
        .map(resolve_non_null_schema)
        .collect();
    if variants.is_empty() {
        return schema.clone();
    }
    let merged = merge_schemas(&variants);
    // `{ ...schema, ...merged }`: keys the merge produced win.
    JsonSchema {
        kind: merged.kind.or_else(|| schema.kind.clone()),
        enum_values: merged.enum_values.or_else(|| schema.enum_values.clone()),
        properties: merged.properties.or_else(|| schema.properties.clone()),
        required: merged.required.or_else(|| schema.required.clone()),
        description: merged.description.or_else(|| schema.description.clone()),
        ..schema.clone()
    }
}

fn config_type_for_schema(schema: &JsonSchema) -> Result<String, String> {
    if schema.enum_values.is_some() {
        return Ok("enum".to_owned());
    }
    match schema.kind.as_deref() {
        Some(kind @ ("array" | "boolean" | "number" | "string")) => {
            Ok(if schema.format.as_deref() == Some("uri") {
                "url".to_owned()
            } else {
                kind.to_owned()
            })
        }
        Some("integer") => Ok("number".to_owned()),
        Some("object") => Ok("object".to_owned()),
        other => Err(format!(
            "Unsupported JSON Schema config type: {}",
            other.unwrap_or("unknown")
        )),
    }
}

fn format_for_schema(schema: &JsonSchema) -> Option<String> {
    if schema.pattern.as_deref() == Some(super::zod::DIGITS_REGEX) {
        Some("bigint-string".to_owned())
    } else {
        schema.format.clone()
    }
}

fn is_required(schema: &JsonSchema, required: bool) -> bool {
    required && schema.default.is_none()
}

fn string_enum_values(schema: &JsonSchema) -> Option<Vec<String>> {
    schema.enum_values.as_ref().map(|values| {
        values
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect()
    })
}

fn config_item_for_schema(raw: &JsonSchema) -> Result<ConnectionConfigItem, String> {
    let schema = resolve_non_null_schema(raw);
    let kind = config_type_for_schema(&schema)?;
    if kind == "array" {
        return Err("Nested array config fields are not supported".to_owned());
    }
    if kind == "object" {
        let required = schema.required.clone().unwrap_or_default();
        let fields = schema
            .properties
            .as_ref()
            .map(|props| {
                props
                    .iter()
                    .map(|(name, property)| {
                        config_item_field_for_schema(name, property, required.contains(name))
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        return Ok(ConnectionConfigItem {
            kind,
            enum_values: None,
            fields,
            format: None,
            pattern: None,
            description: schema.description.clone().filter(|d| !d.is_empty()),
        });
    }
    Ok(ConnectionConfigItem {
        kind,
        enum_values: string_enum_values(&schema),
        fields: None,
        format: format_for_schema(&schema).filter(|f| !f.is_empty()),
        pattern: schema.pattern.clone().filter(|p| !p.is_empty()),
        description: schema.description.clone().filter(|d| !d.is_empty()),
    })
}

fn config_item_field_for_schema(
    name: &str,
    raw: &JsonSchema,
    required: bool,
) -> Result<ConnectionConfigItemField, String> {
    let schema = resolve_non_null_schema(raw);
    let item = config_item_for_schema(&schema)?;
    if item.kind == "object" {
        return Err("Nested object config fields are not supported".to_owned());
    }
    Ok(ConnectionConfigItemField {
        name: name.to_owned(),
        kind: item.kind,
        required: is_required(&schema, required),
        enum_values: item.enum_values,
        format: item.format,
        pattern: item.pattern,
        description: item.description,
    })
}

fn config_field_for_schema(
    name: &str,
    raw: &JsonSchema,
    required: bool,
) -> Result<ConnectionConfigField, String> {
    let schema = resolve_non_null_schema(raw);
    let kind = config_type_for_schema(&schema)?;
    if kind == "array" {
        return Ok(ConnectionConfigField {
            name: name.to_owned(),
            kind,
            required: is_required(&schema, required),
            items: schema
                .items
                .as_deref()
                .map(config_item_for_schema)
                .transpose()?,
            description: schema.description.clone().filter(|d| !d.is_empty()),
            ..ConnectionConfigField::default()
        });
    }
    let item = config_item_for_schema(&schema)?;
    if item.kind == "object" {
        return Err("Top-level object config fields are not supported".to_owned());
    }
    Ok(ConnectionConfigField {
        name: name.to_owned(),
        kind: item.kind,
        required: is_required(&schema, required),
        enum_values: item.enum_values,
        format: item.format,
        pattern: item.pattern,
        description: item.description,
        ..ConnectionConfigField::default()
    })
}

/// `zodToConfigFields`: the connection catalog's field metadata of an
/// object schema (in the JSON Schema form `z.toJSONSchema` gives).
pub fn zod_to_config_fields(schema: &JsonSchema) -> Result<Vec<ConnectionConfigField>, String> {
    let required = schema.required.clone().unwrap_or_default();
    schema
        .properties
        .iter()
        .flatten()
        .map(|(name, property)| config_field_for_schema(name, property, required.contains(name)))
        .collect()
}

/// `connectSessionNextActionSchema`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConnectSessionNextAction {
    OpenUrl {
        url: String,
    },
    EnterInput {
        #[serde(rename = "inputFields")]
        input_fields: Vec<ConnectionConfigField>,
    },
    Wait,
}

/// `connectSessionNextActionSchema.safeParse`.
pub fn parse_connect_session_next_action(
    value: &Value,
) -> Result<ConnectSessionNextAction, String> {
    serde_json::from_value(value.clone()).map_err(|e| e.to_string())
}

string_enum! {
    /// `connectSessionPurposes`.
    pub enum ConnectSessionPurpose {
        Connect = "connect",
        Reconnect = "reconnect",
        FacebookAds = "facebook_ads",
        MessagingAds = "messaging_ads",
        LeadAds = "lead_ads",
        MetaCatalog = "meta_catalog",
    }
}

string_enum! {
    /// `connectSessionStatuses`.
    pub enum ConnectSessionStatus {
        Pending = "pending",
        Authorized = "authorized",
        AwaitingSelection = "awaiting_selection",
        Completed = "completed",
        Failed = "failed",
        Expired = "expired",
        Cancelled = "cancelled",
    }
}

/// `ACTIVE_CONNECT_SESSION_STATUSES`.
pub const ACTIVE_CONNECT_SESSION_STATUSES: &[ConnectSessionStatus] = &[
    ConnectSessionStatus::Pending,
    ConnectSessionStatus::Authorized,
    ConnectSessionStatus::AwaitingSelection,
];
/// `TERMINAL_CONNECT_SESSION_STATUSES`.
pub const TERMINAL_CONNECT_SESSION_STATUSES: &[ConnectSessionStatus] = &[
    ConnectSessionStatus::Completed,
    ConnectSessionStatus::Failed,
    ConnectSessionStatus::Expired,
    ConnectSessionStatus::Cancelled,
];

string_enum! {
    /// `connectSessionErrorCodes`.
    pub enum ConnectSessionErrorCode {
        StateMismatch = "state_mismatch",
        Expired = "expired",
        ProviderDenied = "provider_denied",
        ExchangeFailed = "exchange_failed",
        ProviderError = "provider_error",
        NoCandidates = "no_candidates",
        AlreadyConnected = "already_connected",
        QuotaExceeded = "quota_exceeded",
        TrialExpired = "trial_expired",
        InternalError = "internal_error",
    }
}

string_enum! {
    /// `connectFailureCauses`.
    pub enum ConnectFailureCause {
        DeveloperTokenMissing = "developer_token_missing",
        DeveloperTokenNotApproved = "developer_token_not_approved",
        ProjectNotApproved = "project_not_approved",
        PermissionDenied = "permission_denied",
        ApiNotEnabled = "api_not_enabled",
        CredentialsInvalid = "credentials_invalid",
        ScopeMissing = "scope_missing",
        LegacyUploadNotAllowed = "legacy_upload_not_allowed",
    }
}

/// `CONNECT_ERROR_QUERY_PARAM`.
pub const CONNECT_ERROR_QUERY_PARAM: &str = "connect_error";

/// Whether `code` is one of `connectErrorQueryCodes` (session error codes,
/// `provider_unavailable`, failure causes).
pub fn is_connect_error_query_code(code: &str) -> bool {
    ConnectSessionErrorCode::parse(code).is_some()
        || code == "provider_unavailable"
        || ConnectFailureCause::parse(code).is_some()
}

fn has_url_scheme(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    for c in chars {
        if c == ':' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || c == '+' || c == '.' || c == '-') {
            return false;
        }
    }
    false
}

/// `URLSearchParams#set`: the first pair keeps its place with the new value,
/// later pairs with the name are removed, else the pair is appended.
pub fn search_params_set(url: &mut url::Url, name: &str, value: &str) {
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let mut out: Vec<(String, String)> = Vec::with_capacity(pairs.len() + 1);
    let mut found = false;
    for (k, v) in pairs {
        if k == name {
            if !found {
                out.push((k, value.to_owned()));
                found = true;
            }
        } else {
            out.push((k, v));
        }
    }
    if !found {
        out.push((name.to_owned(), value.to_owned()));
    }
    url.query_pairs_mut().clear().extend_pairs(out);
}

fn is_safe_relative_redirect(value: &str) -> bool {
    if !value.starts_with('/') || value.starts_with("//") || value.contains('\\') {
        return false;
    }
    !value.chars().any(|c| (c as u32) <= 31 || c as u32 == 127)
}

/// `appendConnectError`: the return URL with `connect_error=<code>` set,
/// other params and the hash kept; a relative URL stays relative and never
/// becomes protocol-relative.
pub fn append_connect_error(return_url: &str, code: &str) -> String {
    let is_absolute = has_url_scheme(return_url);
    let base = url::Url::parse("http://relative.invalid").expect("valid base");
    let Ok(mut url) = base.join(return_url) else {
        return format!("/?{CONNECT_ERROR_QUERY_PARAM}={code}");
    };
    search_params_set(&mut url, CONNECT_ERROR_QUERY_PARAM, code);
    if is_absolute {
        return url.to_string();
    }
    let search = url.query().map(|q| format!("?{q}")).unwrap_or_default();
    let hash = url
        .fragment()
        .filter(|f| !f.is_empty())
        .map(|f| format!("#{f}"))
        .unwrap_or_default();
    let relative = format!("{}{search}{hash}", url.path());
    if is_safe_relative_redirect(&relative) {
        relative
    } else {
        format!("/?{CONNECT_ERROR_QUERY_PARAM}={code}")
    }
}

/// `connectFailureCauseOf`: an allow-listed cause from a thrown error's
/// `failureCause` or `data.cause` (the error as its JSON-like fields).
pub fn connect_failure_cause_of(error: &Value) -> Option<ConnectFailureCause> {
    let object = error.as_object()?;
    let candidate = if object.contains_key("failureCause") {
        object.get("failureCause")
    } else {
        match object.get("data") {
            Some(Value::Object(data)) => data.get("cause"),
            _ => None,
        }
    };
    candidate
        .and_then(Value::as_str)
        .and_then(ConnectFailureCause::parse)
}
