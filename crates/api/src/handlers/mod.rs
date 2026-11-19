//! Handlers of the public `/v1` routes (`apps/builder/src/features/*/api/public*.ts`),
//! one module per builder router. [`dispatch`] runs the handler of a route after
//! its auth; `None` = the route has no Rust handler yet (the "not ported" answer).

pub mod ads;
pub mod ads_campaigns;
pub mod ai_agents;
pub mod ai_files;
pub mod ai_functions;
pub mod ai_handover;
pub mod ai_mcp_servers;
pub mod analytics;
pub mod appointments;
pub mod bot_fields;
pub mod broadcasts;
pub mod capabilities;
pub mod capi;
pub mod channel_integrations;
pub mod channel_settings;
pub mod channels;
pub mod comment_automations;
pub mod comment_channels;
pub mod comment_crud;
pub mod conditions;
pub mod connect_sessions;
pub mod connections;
pub mod contact_custom_fields;
pub mod contact_exports;
pub mod contact_filter;
pub mod contact_imports;
pub mod contact_inboxes;
pub mod contact_messages;
pub mod contact_notes;
pub mod contact_scans;
pub mod contact_sequences;
pub mod contacts;
pub mod contacts_bulk;
pub mod conversations;
pub mod coupons;
pub mod custom_fields;
pub mod discount_codes;
pub mod ecommerce_analytics;
pub mod ecommerce_settings;
pub mod email_topics;
pub mod error_logs;
pub mod external_webhooks;
pub mod facebook_lead_ads;
pub mod flows;
pub mod folders;
pub mod google_ads;
pub mod inbound_webhooks;
pub mod inbox_teams;
pub mod inboxes;
pub mod input;
pub mod integrations_ai;
pub mod keywords;
pub mod magic_links;
pub mod marketing_messages;
pub mod me;
pub mod media_library;
pub mod member_scope;
pub mod messages;
pub mod messenger_menus;
pub mod messenger_templates;
pub mod meta_catalog;
pub mod mini_apps;
pub mod minigames;
pub mod open_hours;
pub mod orders;
pub mod payment_providers;
pub mod pipelines;
pub mod places;
pub mod polls;
pub mod product_categories;
pub mod product_imports;
pub mod product_vendors;
pub mod products;
pub mod qr_codes;
pub mod questionnaires;
pub mod reflinks;
pub mod response_sla;
pub mod saved_replies;
pub mod sequences;
pub mod smtp_integrations;
pub mod spreadsheets;
pub mod store_connections;
pub mod tags;
pub mod templates;
pub mod triggers;
pub mod webchats;
pub mod webhooks;
pub mod webshop;
pub mod whatsapp_calls;
pub mod whatsapp_channels;
pub mod whatsapp_coexist;
pub mod whatsapp_flows;
pub mod whatsapp_templates;
pub mod working_hours;
pub mod workspace_members;
pub mod workspace_settings;

use std::sync::Arc;

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chatbotx_business::BusinessError;
use chatbotx_business::audit::AuditService;
use chatbotx_business::events::{Event, EventBus, TriggerEvent};
use chatbotx_db::Db;
use chatbotx_jobs::Enqueuer;
use chatbotx_kv::Kv;
use chatbotx_rt::Realtime;
use serde_json::Value;

use crate::orpc::{OrpcError, common_api_error};

/// What the handlers need: the database, the public app URL and the
/// cross-cutting services (P3.6-X1). Build it with [`ApiState::new`] and the
/// `with_*` methods; a service that is not given is off (no realtime, no kv,
/// no jobs: audit records are dropped, the event bus has no subscribers).
#[derive(Clone)]
pub struct ApiState {
    pub db: Arc<Db>,
    /// The tenant `appUrl` (`NEXT_PUBLIC_BUILDER_URL`, no trailing `/`).
    pub app_url: String,
    /// The realtime producer (`publishWorkspaceRealtimeEvent`); `None` = the
    /// gateway is off (`REALTIME_BROADCAST_SECRET` unset, tests).
    pub realtime: Option<Realtime>,
    /// The kv store (caches, rate limits).
    pub kv: Option<Arc<Kv>>,
    /// The job store (`queue.add`).
    pub jobs: Option<Arc<dyn Enqueuer>>,
    /// The in-process event bus (flow triggers, custom-field changes, ...).
    pub events: EventBus,
    /// The audit log writer (`auditService`).
    pub audit: AuditService,
    /// The file store (`uploader`; presigned uploads, imports, exports).
    /// `None` = off (tests that need no files).
    pub files: Option<Arc<chatbotx_files::Uploader>>,
    /// `NEXT_PUBLIC_EDITION` (read once at start; tests set it).
    pub edition: chatbotx_business::keys::Edition,
    /// `encryptUtils` (`ENCRYPTION_KEY*`, read once at start: credential
    /// writes, connect-session auth, platform credentials); `None` = the keys
    /// are unset (credential writes fail, an OAuth connect is not configured).
    pub encryption: Option<Arc<chatbotx_core::encryption::Encryption>>,
    /// `LICENSE_KEY` (read once at start; tests set it).
    pub license_token: Option<String>,
    /// The license verification keys (`LICENSE_PUBLIC_KEYS`; tests set them).
    pub license_keys: &'static [(&'static str, &'static str)],
    /// The query embedding client (`aiFiles.search`); the provider APIs by
    /// default.
    pub embedder: Arc<dyn chatbotx_business::ai::knowledge::Embedder>,
    /// The Meta Graph API base (thread control, WhatsApp templates / flows / calls);
    /// tests point it at a local server.
    pub graph_url: String,
    /// The auth state (`POST /v1/me/password`); `None` = off (tests).
    pub sessions: Option<Arc<crate::session::AuthState>>,
}

impl ApiState {
    /// The database and the app URL; every other service is off.
    pub fn new(db: Arc<Db>, app_url: impl Into<String>) -> ApiState {
        ApiState {
            db,
            app_url: app_url.into(),
            realtime: None,
            kv: None,
            jobs: None,
            events: EventBus::new(),
            audit: AuditService::disabled(),
            files: None,
            edition: chatbotx_business::keys::Edition::from_env(),
            encryption: chatbotx_core::encryption::Keys::from_env(&chatbotx_core::env::ProcessEnv)
                .ok()
                .map(|keys| Arc::new(chatbotx_core::encryption::Encryption::new(keys))),
            license_token: std::env::var("LICENSE_KEY").ok(),
            license_keys: chatbotx_business::license::LICENSE_PUBLIC_KEYS,
            embedder: Arc::new(chatbotx_business::ai::knowledge::HttpEmbedder::new()),
            graph_url: chatbotx_business::whatsapp::GRAPH_API_URL.to_owned(),
            sessions: None,
        }
    }

    /// Verifies `token` with `keys` instead of `LICENSE_KEY` and the built-in keys.
    pub fn with_license(
        mut self,
        token: Option<String>,
        keys: &'static [(&'static str, &'static str)],
    ) -> ApiState {
        self.license_token = token;
        self.license_keys = keys;
        self
    }

    /// `assertEnterpriseFeatures()`: 403 `enterpriseFeatureRequired` unless
    /// the edition is not community and the license is valid or in grace.
    pub async fn assert_enterprise_features(&self) -> chatbotx_business::Result<()> {
        let entitled = !self.edition.is_community()
            && chatbotx_business::license::resolve_license(
                &self.db,
                self.license_token.as_deref(),
                self.edition,
                self.license_keys,
                chatbotx_core::js::now_ms(),
            )
            .await
            .state
            .is_entitled();
        if entitled {
            Ok(())
        } else {
            Err(chatbotx_business::license::enterprise_feature_required())
        }
    }

    /// Uses the `ENCRYPTION_KEY` keys of the process env (a missing or
    /// invalid key is logged; OAuth connects are then not configured).
    pub fn with_encryption_from_env(self) -> ApiState {
        match chatbotx_core::encryption::Keys::from_env(&chatbotx_core::env::ProcessEnv) {
            Ok(keys) => {
                self.with_encryption(Arc::new(chatbotx_core::encryption::Encryption::new(keys)))
            }
            Err(error) => {
                tracing::warn!(error = %error, "OAuth connects off: ENCRYPTION_KEY is not valid");
                self
            }
        }
    }

    /// Uses `encryption` (`encryptUtils`).
    pub fn with_encryption(
        mut self,
        encryption: Arc<chatbotx_core::encryption::Encryption>,
    ) -> ApiState {
        self.encryption = Some(encryption);
        self
    }

    /// Runs the services as this edition.
    pub fn with_edition(mut self, edition: chatbotx_business::keys::Edition) -> ApiState {
        self.edition = edition;
        self
    }

    /// Embeds search queries with `embedder`.
    pub fn with_embedder(
        mut self,
        embedder: Arc<dyn chatbotx_business::ai::knowledge::Embedder>,
    ) -> ApiState {
        self.embedder = embedder;
        self
    }

    /// Publishes realtime events through `realtime`.
    pub fn with_realtime(mut self, realtime: Realtime) -> ApiState {
        self.realtime = Some(realtime);
        self
    }

    /// Uses `kv`.
    pub fn with_kv(mut self, kv: Arc<Kv>) -> ApiState {
        self.kv = Some(kv);
        self
    }

    /// Enqueues jobs on `jobs`; the audit writer uses it too.
    pub fn with_jobs(mut self, jobs: Arc<dyn Enqueuer>) -> ApiState {
        self.audit = AuditService::new(Arc::clone(&jobs));
        self.jobs = Some(jobs);
        self
    }

    /// Uses `files` as the file store.
    pub fn with_files(mut self, files: Arc<chatbotx_files::Uploader>) -> ApiState {
        self.files = Some(files);
        self
    }

    /// Uses `events` as the event bus.
    pub fn with_events(mut self, events: EventBus) -> ApiState {
        self.events = events;
        self
    }

    /// Uses `audit` as the audit writer.
    pub fn with_audit(mut self, audit: AuditService) -> ApiState {
        self.audit = audit;
        self
    }

    /// `this.audit(action, detail)` (`BaseService.audit`): one audit record
    /// for the ambient actor (the token middleware sets it).
    pub async fn record_audit(&self, action: &str, detail: &str) {
        self.audit.audit(action, detail).await;
    }

    /// The `emit*` calls of `event-dispatcher.ts`: publishes trigger events
    /// on the in-process bus, in order.
    pub async fn emit_triggers(&self, events: impl IntoIterator<Item = TriggerEvent>) {
        self.events
            .publish_all(events.into_iter().map(Event::Trigger))
            .await;
    }

    /// The `pipelineItemChanged` realtime events the pipeline services return
    /// (`events::PipelineItemChanged`), one per change, in order.
    pub async fn publish_pipeline_changes(
        &self,
        changes: &[chatbotx_business::pipeline::events::PipelineItemChanged],
    ) {
        for change in changes {
            let mut data = serde_json::Map::new();
            data.insert(
                "pipelineId".into(),
                Value::String(change.pipeline_id.to_string()),
            );
            if let Some(item) = change.item_id {
                data.insert("itemId".into(), Value::String(item.to_string()));
            }
            if let Some(contact) = change.contact_id {
                data.insert("contactId".into(), Value::String(contact.to_string()));
            }
            data.insert("change".into(), Value::from(change.change));
            let event = chatbotx_rt::Envelope::new(
                chatbotx_rt::EventType::PipelineItemChanged,
                Value::Object(data),
            );
            self.queue_realtime(change.workspace_id, event).await;
        }
    }

    /// `queueWorkspaceRealtimeEvent`: publishes `event` to the workspace when
    /// the realtime gateway is on (an invalid event is logged).
    pub async fn queue_realtime(&self, workspace_id: i64, event: chatbotx_rt::Envelope) {
        if let Some(realtime) = &self.realtime {
            realtime
                .queue_workspace_event(&workspace_id.to_string(), event)
                .await;
        }
    }
}

/// One authenticated call of a public route.
#[derive(Debug, Clone, Copy)]
pub struct Call<'a> {
    /// `context.workspace.id`.
    pub workspace_id: i64,
    /// Path params (percent-decoded).
    pub params: &'a [(&'static str, String)],
    /// The raw query string.
    pub query: Option<&'a str>,
    /// The raw request body.
    pub body: &'a [u8],
}

tokio::task_local! {
    /// `context.apiToken.id` of the running dispatch ([`dispatch_as`]).
    static TOKEN_ID: Option<i64>;
    /// The signed-in member of a `/rpc` call that runs a public route
    /// ([`dispatch_as_member`]).
    static MEMBER: Option<Member>;
    /// The mobile member of the running [`dispatch_as`] (Q.A1 §3).
    static MOBILE: Option<Arc<crate::auth::MobileMember>>;
}

/// `context.member` of a mobile principal; `None` for a workspace token,
/// MCP OAuth or a `/rpc` member call.
pub fn mobile_member() -> Option<Arc<crate::auth::MobileMember>> {
    MOBILE.try_with(Clone::clone).ok().flatten()
}

/// A workspace member calling a public route through `/rpc` (P4): the
/// `ContactPermissionScope` and the export owner filter of the builder's
/// private procedures. Public token calls have none (unscoped, full PII).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Member {
    pub user_id: i64,
    /// `isWorkspaceAdminMember` (sees every member's exports).
    pub is_admin: bool,
    pub can_view_email_and_phone: bool,
    pub restrict_to_assigned_user_id: Option<i64>,
}

/// The member of the running [`dispatch_as_member`]; `None` for a token call.
pub fn member() -> Option<Member> {
    MEMBER.try_with(|member| *member).ok().flatten()
}

/// Runs the public route `route_id` for a signed-in member; `None` = not ported.
pub async fn dispatch_as_member(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    member: Member,
) -> Option<Response> {
    MEMBER
        .scope(
            Some(member),
            dispatch_as(state, route_id, call, &Principal::default()),
        )
        .await
}

impl Call<'_> {
    /// `context.apiToken.id` (`actorSourceId`): the token of the running
    /// [`dispatch_as`]; `None` outside it or for [`dispatch`].
    pub fn token_id(&self) -> Option<i64> {
        TOKEN_ID.try_with(|id| *id).ok().flatten()
    }
}

/// The calling workspace token (`context.apiToken`). The default (no token
/// id, `scopes: None` = every scope) is what [`dispatch`] uses.
#[derive(Debug, Clone, Default)]
pub struct Principal {
    /// `context.apiToken.id`.
    pub token_id: Option<i64>,
    /// `context.apiToken.scopes` (`None` = unrestricted).
    pub scopes: Option<Vec<String>>,
    /// `context.apiToken.permission === "read_only"` (default: a `full` token).
    pub read_only: bool,
    /// `context.member` of a mobile principal (Q.A1 §3); `None` for a token.
    pub mobile: Option<Arc<crate::auth::MobileMember>>,
}

/// A handler outcome.
pub type Reply = Result<Response, OrpcError>;

/// The error map entries of a route (`code`, `status`): a thrown error with
/// one of these codes and statuses is `defined: true`.
pub type ErrorMap = &'static [(&'static str, u16)];

/// `possibleErrorsOnFindingResource`.
pub const FINDING: ErrorMap = &[("notFound", 404), ("businessError", 400)];
/// `possibleErrorsOnListingResource`.
pub const LISTING: ErrorMap = &[("businessError", 400)];
/// `possibleErrorsOnMutatingResource` / `possibleErrorsOnDeletingResource`.
pub const MUTATING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];
/// `possibleErrorsOnCreatingResource`.
pub const CREATING: ErrorMap = &[
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
];

/// `200` (or `status`) with a JSON body.
pub fn json(status: u16, body: Value) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// `204` without a body.
pub fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

/// A Date output field: ISO 8601 with milliseconds.
pub fn date(ms: i64) -> Value {
    Value::String(chatbotx_db::repo::iso(ms))
}

/// An optional Date output field.
pub fn opt_date(ms: Option<i64>) -> Value {
    ms.map_or(Value::Null, date)
}

/// A bigint id output field (a string).
pub fn id(value: i64) -> Value {
    Value::String(value.to_string())
}

/// An optional bigint id output field.
pub fn opt_id(value: Option<i64>) -> Value {
    value.map_or(Value::Null, id)
}

/// A JavaScript number: whole values print without a fraction (`10`, not `10.0`).
pub fn number(value: f64) -> Value {
    if value.fract() == 0.0 && value.abs() < 9_007_199_254_740_992.0 {
        // Whole and in the safe range: the cast is exact (`-0` prints as `0`).
        Value::from(value as i64)
    } else {
        serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
    }
}

/// `toKnownOrpcError` of a service error, `defined` per the route's error map
/// (plus `commonApiErrors`). A repository error is a 500.
pub fn business_error(error: BusinessError, errors: ErrorMap) -> OrpcError {
    match error {
        BusinessError::Exception {
            message,
            code,
            status,
            data,
            ..
        } => {
            let message = display_message(&message, &code, data.as_ref());
            let mut out = OrpcError::new(&code, Some(status), Some(&message));
            out.defined = errors.iter().any(|(c, s)| *c == code && *s == status)
                || common_api_error(&code).is_some_and(|d| d.status == status);
            out.data = if out.defined { data } else { None };
            out
        }
        BusinessError::Repo(error) => internal_error(&error.message()),
    }
}

/// JS `String(value)`.
fn js_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(n) => n
            .as_f64()
            .map_or_else(|| n.to_string(), |f| number(f).to_string()),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                if item.is_null() {
                    String::new()
                } else {
                    js_string(item)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

/// `toDisplayMessage` (`apps/builder/src/orpc.ts`): a `ChatbotXException`
/// with `data` shows it as ` (key=value, ...)`. A 422 `invalidRequestData`
/// carries zod issues (a `ZodError` in the original), so it is left as it is.
fn display_message(message: &str, code: &str, data: Option<&Value>) -> String {
    let Some(Value::Object(map)) = data else {
        return message.to_string();
    };
    if code == "invalidRequestData" || map.is_empty() {
        return message.to_string();
    }
    let params = map
        .iter()
        .map(|(key, value)| format!("{key}={}", js_string(value)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{message} ({params})")
}

/// `500 INTERNAL_SERVER_ERROR` (the cause is logged, not sent).
pub fn internal_error(cause: &str) -> OrpcError {
    tracing::error!(cause, "public API handler failed");
    let mut out = OrpcError::new(
        "INTERNAL_SERVER_ERROR",
        Some(500),
        Some("Internal server error"),
    );
    out.defined = false;
    out
}

/// Runs the handler of `route_id` with an unrestricted token; `None` = not
/// ported yet.
pub async fn dispatch(state: &ApiState, route_id: &str, call: Call<'_>) -> Option<Response> {
    dispatch_as(state, route_id, call, &Principal::default()).await
}

/// Runs the handler of `route_id` for `principal`; `None` = not ported yet.
pub async fn dispatch_as(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    principal: &Principal,
) -> Option<Response> {
    // Fail closed: a mobile principal only reaches the allowlisted routes
    // (also checked before the member lookup, `resolve_mobile_principal`).
    if principal.mobile.is_some() && !crate::auth::is_mobile_member_route_id(route_id) {
        return Some(
            OrpcError::new(
                "FORBIDDEN",
                None,
                Some("This operation is not available to mobile sessions"),
            )
            .into_response(),
        );
    }
    MOBILE
        .scope(
            principal.mobile.clone(),
            TOKEN_ID.scope(principal.token_id, route(state, route_id, call, principal)),
        )
        .await
}

async fn route(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    principal: &Principal,
) -> Option<Response> {
    let reply = if working_hours::ROUTES.contains(&route_id) {
        working_hours::handle(state, route_id, call).await
    } else if appointments::ROUTES.contains(&route_id) {
        appointments::handle(state, route_id, call, principal).await
    } else if contact_scans::ROUTES.contains(&route_id) {
        contact_scans::handle(state, route_id, call).await
    } else if comment_automations::ROUTES.contains(&route_id) {
        comment_automations::handle(state, route_id, call).await
    } else if comment_crud::ROUTES.contains(&route_id) {
        comment_crud::handle(state, route_id, call).await
    } else if comment_channels::ROUTES.contains(&route_id) {
        comment_channels::handle(state, route_id, call).await
    } else if connect_sessions::ROUTES.contains(&route_id) {
        connect_sessions::handle(state, route_id, call, principal).await
    } else if connections::ROUTES.contains(&route_id) {
        connections::handle(state, route_id, call, principal).await?
    } else if integrations_ai::ROUTES.contains(&route_id) {
        integrations_ai::handle(state, route_id, call).await
    } else if ai_files::ROUTES.contains(&route_id) {
        ai_files::handle(state, route_id, call).await
    } else if ai_handover::ROUTES.contains(&route_id) {
        ai_handover::handle(state, route_id, call).await
    } else if ai_agents::ROUTES.contains(&route_id) {
        ai_agents::handle(state, route_id, call).await
    } else if ai_functions::ROUTES.contains(&route_id) {
        ai_functions::handle(state, route_id, call).await
    } else if ai_mcp_servers::ROUTES.contains(&route_id) {
        ai_mcp_servers::handle(state, route_id, call).await
    } else if analytics::ROUTES.contains(&route_id) {
        analytics::handle(state, route_id, call).await
    } else if contact_sequences::ROUTES.contains(&route_id) {
        contact_sequences::handle(state, route_id, call).await
    } else if email_topics::ROUTES.contains(&route_id) {
        email_topics::handle(state, route_id, call).await
    } else if triggers::ROUTES.contains(&route_id) {
        triggers::handle(state, route_id, call).await
    } else if sequences::ROUTES.contains(&route_id) {
        sequences::handle(state, route_id, call).await
    } else if broadcasts::ROUTES.contains(&route_id) {
        broadcasts::handle(state, route_id, call).await
    } else if bot_fields::ROUTES.contains(&route_id) {
        bot_fields::handle(state, route_id, call).await
    } else if contact_custom_fields::ROUTES.contains(&route_id) {
        contact_custom_fields::handle(state, route_id, call).await
    } else if contact_inboxes::ROUTES.contains(&route_id) {
        contact_inboxes::handle(state, route_id, call).await
    } else if contact_notes::ROUTES.contains(&route_id) {
        contact_notes::handle(state, route_id, call).await
    } else if contact_messages::ROUTES.contains(&route_id) {
        contact_messages::handle(state, route_id, call).await
    } else if conversations::ROUTES.contains(&route_id) {
        conversations::handle(state, route_id, call).await
    } else if messages::ROUTES.contains(&route_id) {
        messages::handle(state, route_id, call).await
    } else if keywords::ROUTES.contains(&route_id) {
        keywords::handle(state, route_id, call).await
    } else if templates::ROUTES.contains(&route_id) {
        templates::handle(state, route_id, call, principal).await
    } else if store_connections::ROUTES.contains(&route_id) {
        store_connections::handle(state, route_id, call).await
    } else if coupons::ROUTES.contains(&route_id) {
        coupons::handle(state, route_id, call).await
    } else if webshop::ROUTES.contains(&route_id) {
        webshop::handle(state, route_id, call, principal).await
    } else if tags::ROUTES.contains(&route_id) {
        tags::handle(state, route_id, call).await
    } else if contacts_bulk::ROUTES.contains(&route_id) {
        contacts_bulk::handle(state, route_id, call).await
    } else if contacts::ROUTES.contains(&route_id) {
        contacts::handle(state, route_id, call).await
    } else if custom_fields::ROUTES.contains(&route_id) {
        custom_fields::handle(state, route_id, call).await
    } else if orders::ROUTES.contains(&route_id) {
        orders::handle(state, route_id, call).await
    } else if payment_providers::ROUTES.contains(&route_id) {
        payment_providers::handle(state, route_id, call).await
    } else if ecommerce_analytics::ROUTES.contains(&route_id) {
        ecommerce_analytics::handle(state, route_id, call).await
    } else if discount_codes::ROUTES.contains(&route_id) {
        discount_codes::handle(state, route_id, call).await
    } else if ecommerce_settings::ROUTES.contains(&route_id) {
        ecommerce_settings::handle(state, route_id, call).await
    } else if folders::ROUTES.contains(&route_id) {
        folders::handle(state, route_id, call).await
    } else if flows::ROUTES.contains(&route_id) {
        flows::handle(state, route_id, call).await
    } else if smtp_integrations::ROUTES.contains(&route_id) {
        smtp_integrations::handle(state, route_id, call).await
    } else if webchats::ROUTES.contains(&route_id) {
        webchats::handle(state, route_id, call).await
    } else if channel_integrations::ROUTES.contains(&route_id) {
        channel_integrations::handle(state, route_id, call).await
    } else if places::ROUTES.contains(&route_id) {
        places::handle(state, route_id, call).await
    } else if open_hours::ROUTES.contains(&route_id) {
        open_hours::handle(state, route_id, call).await
    } else if pipelines::ROUTES.contains(&route_id) {
        pipelines::handle(state, route_id, call).await
    } else if polls::ROUTES.contains(&route_id) {
        polls::handle(state, route_id, call).await
    } else if product_categories::ROUTES.contains(&route_id) {
        product_categories::handle(state, route_id, call).await
    } else if product_vendors::ROUTES.contains(&route_id) {
        product_vendors::handle(state, route_id, call).await
    } else if products::ROUTES.contains(&route_id) {
        products::handle(state, route_id, call).await
    } else if ads_campaigns::ROUTES.contains(&route_id) {
        ads_campaigns::handle(state, route_id, call, principal).await
    } else if ads::ROUTES.contains(&route_id) {
        ads::handle(state, route_id, call).await
    } else if capi::ROUTES.contains(&route_id) {
        capi::handle(state, route_id, call).await
    } else if facebook_lead_ads::ROUTES.contains(&route_id) {
        facebook_lead_ads::handle(state, route_id, call).await
    } else if google_ads::ROUTES.contains(&route_id) {
        google_ads::handle(state, route_id, call).await
    } else if marketing_messages::ROUTES.contains(&route_id) {
        marketing_messages::handle(state, route_id, call).await
    } else if mini_apps::ROUTES.contains(&route_id) {
        mini_apps::handle(state, route_id, call).await
    } else if qr_codes::ROUTES.contains(&route_id) {
        qr_codes::handle(state, route_id, call).await
    } else if meta_catalog::ROUTES.contains(&route_id) {
        meta_catalog::handle(state, route_id, call).await
    } else if media_library::ROUTES.contains(&route_id) {
        media_library::handle(state, route_id, call).await
    } else if minigames::ROUTES.contains(&route_id) {
        minigames::handle(state, route_id, call).await
    } else if questionnaires::ROUTES.contains(&route_id) {
        questionnaires::handle(state, route_id, call).await
    } else if reflinks::ROUTES.contains(&route_id) {
        reflinks::handle(state, route_id, call).await
    } else if workspace_members::ROUTES.contains(&route_id) {
        workspace_members::handle(state, route_id, call).await
    } else if contact_exports::ROUTES.contains(&route_id) {
        contact_exports::handle(state, route_id, call).await
    } else if product_imports::ROUTES.contains(&route_id) {
        product_imports::handle(state, route_id, call).await
    } else if contact_imports::ROUTES.contains(&route_id) {
        contact_imports::handle(state, route_id, call).await
    } else if contact_filter::ROUTES.contains(&route_id) {
        contact_filter::handle(state, route_id, call).await
    } else if workspace_settings::ROUTES.contains(&route_id) {
        workspace_settings::handle(state, route_id, call).await
    } else if error_logs::ROUTES.contains(&route_id) {
        error_logs::handle(state, route_id, call).await
    } else if inboxes::ROUTES.contains(&route_id) {
        inboxes::handle(state, route_id, call).await
    } else if inbox_teams::ROUTES.contains(&route_id) {
        inbox_teams::handle(state, route_id, call).await
    } else if capabilities::ROUTES.contains(&route_id) {
        capabilities::handle(state, route_id, call).await
    } else if messenger_menus::ROUTES.contains(&route_id) {
        messenger_menus::handle(state, route_id, call).await
    } else if messenger_templates::ROUTES.contains(&route_id) {
        messenger_templates::handle(state, route_id, call).await
    } else if whatsapp_templates::ROUTES.contains(&route_id) {
        whatsapp_templates::handle(state, route_id, call).await
    } else if whatsapp_flows::ROUTES.contains(&route_id) {
        whatsapp_flows::handle(state, route_id, call).await
    } else if whatsapp_calls::ROUTES.contains(&route_id) {
        whatsapp_calls::handle(state, route_id, call).await
    } else if whatsapp_coexist::ROUTES.contains(&route_id) {
        whatsapp_coexist::handle(state, route_id, call).await
    } else if whatsapp_channels::ROUTES.contains(&route_id) {
        whatsapp_channels::handle(state, route_id, call).await
    } else if saved_replies::ROUTES.contains(&route_id) {
        saved_replies::handle(state, route_id, call).await
    } else if response_sla::ROUTES.contains(&route_id) {
        response_sla::handle(state, route_id, call).await
    } else if webhooks::ROUTES.contains(&route_id) {
        webhooks::handle(state, route_id, call).await
    } else if inbound_webhooks::ROUTES.contains(&route_id) {
        inbound_webhooks::handle(state, route_id, call, principal).await
    } else if external_webhooks::ROUTES.contains(&route_id) {
        external_webhooks::handle(state, route_id, call).await
    } else if magic_links::ROUTES.contains(&route_id) {
        magic_links::handle(state, route_id, call).await
    } else if spreadsheets::ROUTES.contains(&route_id) {
        spreadsheets::handle(state, route_id, call).await
    } else if working_hours::ROUTES.contains(&route_id) {
        working_hours::handle(state, route_id, call).await
    } else {
        return None;
    };
    Some(match reply {
        Ok(response) => response,
        Err(error) => error.into_response(),
    })
}
