//! The WhatsApp channel routes of `routers/public.ts` `whatsappChannels`
//! that are not calling, CAPI or coexist: `createChannelReadRoutes("whatsapp")`,
//! `createHandoverResumeFlowRoute("whatsapp")`,
//! `createWhatsappRoutingRolesRoutes()` (scope `channels`) and
//! `whatsappPaymentPublicRouter` (scope `integrations`).
//!
//! Also the Graph reads and writes of the builder's WhatsApp channel screens
//! (`integrations/whatsapp/src/api/{phone-number,waba,verification,
//! waba-setup,routing-roles}.ts`), used by `rpc_procedures::channels`.
//!
//! ponytail: no `assertReferencable` template locks (P3.6-D1 rule).

use chatbotx_business::{BusinessError, tx};
use chatbotx_db::repo::models::IntegrationWhatsapp;
use chatbotx_db::repo::{Json, find_one, integration_whatsapp, now_ms};
use chatbotx_db::rusqlite::params;
use reqwest::Method;
use serde_json::{Value, json};

use super::input::{Input, key};
use super::messenger_templates::{Fail, graph_call};
use super::tags::digits_at;
use super::{ApiState, Call, ErrorMap, FINDING, LISTING, MUTATING, Reply, business_error};
use super::{json, no_content};

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "whatsappChannels.list",
    "whatsappChannels.get",
    "whatsappChannels.updateHandoverResumeFlow",
    "whatsappChannels.getRoutingRoles",
    "whatsappChannels.syncRoutingRoles",
    "whatsappChannels.getPaymentSettings",
    "whatsappChannels.updatePaymentSettings",
];

/// `possibleErrorsOnSyncingRoutingRoles`.
const SYNCING: ErrorMap = &[
    ("notFound", 404),
    ("businessError", 400),
    ("idempotencyKeyInvalid", 422),
    ("idempotencyKeyReused", 422),
    ("idempotencyKeyConflict", 409),
    ("routingRolesUnavailable", 502),
];

/// `DEFAULT_API_VERSION` of `integrations/whatsapp`.
pub const DEFAULT_API_VERSION: &str = "v23.0";
/// `BUSINESS_URL`.
pub const BUSINESS_URL: &str = "https://business.facebook.com";

/// `summarize("whatsapp", ...)` of `channelIntegrationService`.
pub fn summary(row: &IntegrationWhatsapp) -> Value {
    let disconnected = row.capi_disconnected_at.is_some();
    json!({
        "id": row.id.to_string(),
        "channel": "whatsapp",
        "inboxId": row.inbox_id.to_string(),
        "name": row.name,
        "externalId": row.phone_number_id,
        "wabaId": row.waba_id,
        "displayPhoneNumber": (!row.display_phone_number.is_empty()).then_some(&row.display_phone_number),
        "username": Value::Null,
        "coexistEnabled": row.coexist_enabled,
        "isCoexist": row.is_coexist,
        "hasCapiScope": row.has_capi_scope,
        // `resolveCapiConnection`: WhatsApp needs no CAPI scope.
        "capiConnected": !disconnected && row.dataset_id.is_some(),
        "datasetId": row.dataset_id,
        "capiTestEventCode": row.capi_test_event_code,
        "capiDisconnected": disconnected,
        "syncTagEnabledAt": Value::Null,
        "handoverResumeFlowId": row.handover_resume_flow_id.map(|id| id.to_string()),
        "tokenRefreshError": row.token_refresh_error,
        "adsEligible": true,
    })
}

/// The workspace's number (`None` = not in this workspace).
pub async fn find(
    db: &chatbotx_db::Db,
    ws: i64,
    id: i64,
) -> Result<Option<IntegrationWhatsapp>, BusinessError> {
    tx::read(db, move |conn| {
        Ok(integration_whatsapp::find_by_id_for_workspace(
            conn, id, ws,
        )?)
    })
    .await
}

fn id_param(input: &mut Input) -> i64 {
    let value = input.get("id").cloned();
    digits_at(input, &key("id"), value.as_ref(), true).unwrap_or_default()
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let db = &state.db;
    let ws = call.workspace_id;
    match route_id {
        "whatsappChannels.list" => tx::read(db, move |conn| {
            Ok(chatbotx_db::repo::find_many::<IntegrationWhatsapp>(
                conn,
                r#"WHERE "workspaceId" = ?1"#,
                &[&ws],
            )?)
        })
        .await
        .map(|rows| json(200, rows.iter().map(summary).collect()))
        .map_err(|e| business_error(e, LISTING)),
        "whatsappChannels.get" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_param(&mut input);
            input.finish()?;
            find(db, ws, id)
                .await
                .and_then(|row| {
                    row.ok_or_else(|| BusinessError::not_found("Channel integration not found"))
                })
                .map(|row| json(200, summary(&row)))
                .map_err(|e| business_error(e, FINDING))
        }
        "whatsappChannels.updateHandoverResumeFlow" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_param(&mut input);
            let flow = match input.get("handoverResumeFlowId").cloned() {
                Some(Value::Null) => Some(None),
                value => digits_at(
                    &mut input,
                    &key("handoverResumeFlowId"),
                    value.as_ref(),
                    true,
                )
                .map(Some),
            };
            input.finish()?;
            update_handover_resume_flow(db, ws, id, flow.flatten())
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        "whatsappChannels.getRoutingRoles" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_param(&mut input);
            input.finish()?;
            find_routing_roles(db, ws, id)
                .await
                .map(|roles| json(200, roles))
                .map_err(|e| business_error(e, FINDING))
        }
        "whatsappChannels.syncRoutingRoles" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_param(&mut input);
            input.finish()?;
            sync_routing_roles(state, ws, id)
                .await
                .map(|roles| json(200, roles))
                .map_err(|e| business_error(e, SYNCING))
        }
        "whatsappChannels.getPaymentSettings" => {
            let mut input = Input::from_query(call.params, call.query);
            let id = id_param(&mut input);
            input.finish()?;
            find(db, ws, id)
                .await
                .and_then(|row| {
                    row.ok_or_else(|| BusinessError::not_found("WhatsApp channel not found"))
                })
                .map(|row| {
                    json(
                        200,
                        json!({ "pixRedirectEnabled": row.pix_redirect_enabled }),
                    )
                })
                .map_err(|e| business_error(e, FINDING))
        }
        "whatsappChannels.updatePaymentSettings" => {
            let mut input = Input::from_body(call.params, call.body);
            let id = id_param(&mut input);
            let enabled = input.bool("pixRedirectEnabled");
            input.finish()?;
            update_pix_redirect(db, ws, id, enabled.unwrap_or_default())
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            Ok(no_content())
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// `integrationWhatsappService.updatePixRedirectEnabled`.
pub async fn update_pix_redirect(
    db: &chatbotx_db::Db,
    ws: i64,
    id: i64,
    enabled: bool,
) -> Result<(), BusinessError> {
    tx::write(db, move |conn| {
        integration_whatsapp::update_pix_redirect_enabled(conn, id, ws, enabled)?
            .map(|_| ())
            .ok_or_else(|| BusinessError::not_found("WhatsApp integration not found"))
    })
    .await
}

/// `integrationWhatsappService.updateHandoverResumeFlow`: a flow must be an
/// active flow of the workspace (`flowService.findActiveById`).
pub async fn update_handover_resume_flow(
    db: &chatbotx_db::Db,
    ws: i64,
    id: i64,
    flow: Option<i64>,
) -> Result<(), BusinessError> {
    tx::write(db, move |conn| {
        if let Some(flow) = flow {
            let active: Option<i64> = conn
                .query_row(
                    r#"SELECT "id" FROM "Flow" WHERE "id" = ?1 AND "workspaceId" = ?2 AND "active" = 1"#,
                    params![flow, ws],
                    |r| r.get(0),
                )
                .ok();
            if active.is_none() {
                return Err(BusinessError::not_found("Handover flow not found"));
            }
        }
        integration_whatsapp::update_handover_resume_flow(conn, id, ws, flow)?
            .map(|_| ())
            .ok_or_else(|| BusinessError::not_found("WhatsApp integration not found"))
    })
    .await
}

/// `readRoutingRoles`: the stored snapshot when it has the snapshot shape.
fn read_routing_roles(value: Option<&Json>) -> Value {
    let Some(Json(value)) = value else {
        return Value::Null;
    };
    let valid = value.get("enabled").is_some_and(Value::is_boolean)
        && value.get("isStandby").is_some_and(Value::is_boolean)
        && value.get("checkedAt").is_some_and(Value::is_string)
        && value
            .get("roles")
            .and_then(Value::as_array)
            .is_some_and(|roles| roles.iter().all(Value::is_string));
    if !valid {
        return Value::Null;
    }
    json!({
        "enabled": value["enabled"],
        "roles": value["roles"],
        "isStandby": value["isStandby"],
        "checkedAt": value["checkedAt"],
    })
}

/// `integrationWhatsappService.findRoutingRoles`.
pub async fn find_routing_roles(
    db: &chatbotx_db::Db,
    ws: i64,
    id: i64,
) -> Result<Value, BusinessError> {
    let row = find(db, ws, id)
        .await?
        .ok_or_else(|| BusinessError::not_found("WhatsApp integration not found"))?;
    Ok(read_routing_roles(row.routing_roles.as_ref()))
}

/// `integrationWhatsappService.findRoutingRolesByInbox`.
pub async fn find_routing_roles_by_inbox(
    db: &chatbotx_db::Db,
    ws: i64,
    inbox_id: i64,
) -> Result<Value, BusinessError> {
    let row = tx::read(db, move |conn| {
        Ok(find_one::<IntegrationWhatsapp>(
            conn,
            r#""inboxId" = ?1 AND "workspaceId" = ?2"#,
            &[&inbox_id, &ws],
        )?)
    })
    .await?;
    Ok(row
        .map(|row| read_routing_roles(row.routing_roles.as_ref()))
        .unwrap_or(Value::Null))
}

/// `ROUTING_NOT_ENABLED_RE` on a code 100 error.
fn routing_not_enabled(error: &GraphError) -> bool {
    error.code == Some(100)
        && error
            .message
            .to_lowercase()
            .contains("conversation routing is not enabled")
}

/// `syncWhatsappRoutingRoles` (lib + `integrationWhatsappService.syncRoutingRoles`):
/// asks Meta (`GET /{phone_number_id}/routing_roles`), stores the answer
/// unless a newer one is stored, returns the snapshot. A Meta failure is a
/// 502 `routingRolesUnavailable` and keeps the stored snapshot.
pub async fn sync_routing_roles(
    state: &ApiState,
    ws: i64,
    id: i64,
) -> Result<Value, BusinessError> {
    let row = find(&state.db, ws, id)
        .await?
        .ok_or_else(|| BusinessError::not_found("WhatsApp integration not found"))?;
    let auth = WhatsappAuth::of(&row);
    let path = format!("{}/routing_roles", row.phone_number_id);
    let answer = match auth.get(&state.graph_url, &path, &[]).await {
        Ok(body) => {
            let roles = body.get("roles").and_then(Value::as_array);
            let standby = body.get("is_standby").and_then(Value::as_bool);
            match (roles, standby) {
                (Some(roles), Some(standby)) if roles.iter().all(Value::is_string) => {
                    json!({ "enabled": true, "roles": roles, "isStandby": standby })
                }
                _ => return Err(unavailable("Invalid input")),
            }
        }
        Err(error) if routing_not_enabled(&error) => {
            json!({ "enabled": false, "roles": [], "isStandby": false })
        }
        Err(error) => return Err(unavailable(&error.message)),
    };
    let checked_at = chatbotx_db::repo::iso(now_ms());
    let mut snapshot = answer;
    snapshot["checkedAt"] = Value::String(checked_at);
    let stored = Json(snapshot.clone());
    let saved = tx::write(&state.db, move |conn| {
        Ok(integration_whatsapp::update_routing_roles_if_newer(
            conn, id, ws, &stored,
        )?)
    })
    .await?;
    if saved.is_some() {
        return Ok(snapshot);
    }
    match find_routing_roles(&state.db, ws, id).await? {
        Value::Null => Err(BusinessError::not_found("WhatsApp integration not found")),
        current => Ok(current),
    }
}

fn unavailable(reason: &str) -> BusinessError {
    BusinessError::exception(
        format!("Meta did not return the routing roles ({reason}). Try again."),
        "routingRolesUnavailable",
        502,
    )
}

// ---- Graph -----------------------------------------------------------------

/// A failed WhatsApp Graph call (`WhatsappException` / `ChannelError`
/// fields).
#[derive(Debug, Clone, PartialEq)]
pub struct GraphError {
    /// The formatted message (`format_graph_error`).
    pub message: String,
    pub code: Option<i64>,
    pub sub_code: Option<i64>,
    pub kind: Option<String>,
    pub user_title: Option<String>,
    pub user_message: Option<String>,
    pub fbtrace_id: Option<String>,
    /// The Meta `error.message` as sent.
    pub raw_message: Option<String>,
}

impl GraphError {
    fn of(fail: Fail, error: &Value) -> GraphError {
        let message = match fail {
            Fail::Sdk(message, _) => message,
            Fail::Business(error) => error.to_string(),
        };
        let text = |key: &str| error.get(key).and_then(Value::as_str).map(str::to_owned);
        let number = |key: &str| {
            error.get(key).and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
        };
        GraphError {
            message,
            code: number("code"),
            sub_code: number("error_subcode"),
            kind: text("type"),
            user_title: text("error_user_title"),
            user_message: text("error_user_msg"),
            fbtrace_id: text("fbtrace_id"),
            raw_message: text("message"),
        }
    }

    /// `buildWhatsappApiActionErrorMessage`: Meta's user sentence first.
    pub fn action_message(&self, fallback: &str) -> String {
        [&self.user_message, &self.user_title]
            .into_iter()
            .flatten()
            .find(|text| !text.is_empty())
            .cloned()
            .or_else(|| (!self.message.is_empty()).then(|| self.message.clone()))
            .unwrap_or_else(|| fallback.to_owned())
    }

    /// `serializeRegistrationError` (`at` = now).
    pub fn registration_error(&self) -> Value {
        let mut out = json!({
            "code": self.code.map_or(Value::Null, Value::from),
            "subCode": self.sub_code,
            "message": self.message,
            "at": chatbotx_db::repo::iso(now_ms()),
        });
        for (key, value) in [
            ("type", &self.kind),
            ("userTitle", &self.user_title),
            ("userMessage", &self.user_message),
            ("fbtraceId", &self.fbtrace_id),
        ] {
            if let Some(value) = value {
                out[key] = Value::String(value.clone());
            }
        }
        out
    }
}

/// The `WhatsappAuthValue` fields the screens read.
#[derive(Debug, Clone)]
pub struct WhatsappAuth {
    pub token: String,
    pub version: String,
    pub phone_number_id: String,
    pub waba_id: String,
    pub business_id: String,
    pub webhook_url: Option<String>,
}

impl WhatsappAuth {
    pub fn of(row: &IntegrationWhatsapp) -> WhatsappAuth {
        let auth = &row.auth.0;
        let text = |pointer: &str| {
            auth.pointer(pointer)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        WhatsappAuth {
            token: text("/tokens/accessToken").unwrap_or_default(),
            version: text("/version")
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| DEFAULT_API_VERSION.to_owned()),
            phone_number_id: text("/metadata/phoneNumber/id")
                .unwrap_or_else(|| row.phone_number_id.clone()),
            waba_id: text("/metadata/wabaId").unwrap_or_else(|| row.waba_id.clone()),
            business_id: text("/metadata/businessId").unwrap_or_else(|| row.business_id.clone()),
            webhook_url: text("/metadata/webhookUrl"),
        }
    }

    async fn call(
        &self,
        method: Method,
        graph_url: &str,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Value, GraphError> {
        let url = format!("{graph_url}/{}/{path}", self.version);
        graph_call(method, Some(&self.token), &url, query, body)
            .await
            .map_err(|failure| {
                let (fail, error) = *failure;
                GraphError::of(fail, &error)
            })
    }

    pub async fn get(
        &self,
        graph_url: &str,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Value, GraphError> {
        self.call(Method::GET, graph_url, path, query, None).await
    }

    pub async fn post(
        &self,
        graph_url: &str,
        path: &str,
        body: &Value,
    ) -> Result<Value, GraphError> {
        self.call(Method::POST, graph_url, path, &[], Some(body))
            .await
    }

    /// `getUrls(auth)`: the Meta Business Suite links of the number.
    pub fn urls(&self) -> Value {
        let (b, w) = (&self.business_id, &self.waba_id);
        json!({
            "addPaymentMethod": format!("{BUSINESS_URL}/billing_hub/accounts/details?business_id={b}&asset_id={w}"),
            "analytics": format!("{BUSINESS_URL}/wa/manage/insights/?business_id={b}&waba_id={w}"),
            "flows": format!("{BUSINESS_URL}/wa/manage/flows/?business_id={b}&waba_id={w}"),
            "linkCatalog": format!("{BUSINESS_URL}/wa/manage/catalog-linking?business_id={b}&waba_id={w}"),
            "paymentHistory": format!("{BUSINESS_URL}/billing_hub/payment_activity?business_id={b}&asset_id={w}"),
            "paymentMethods": format!("{BUSINESS_URL}/billing_hub/payment_methods?business_id={b}&asset_id={w}"),
            "phoneNumbers": format!("{BUSINESS_URL}/wa/manage/phone-numbers?business_id={b}&waba_id={w}"),
            "templates": format!("{BUSINESS_URL}/wa/manage/message-templates?business_id={b}&waba_id={w}"),
            "ecommerce": format!("{BUSINESS_URL}/commerce/get_started?business_id={b}"),
        })
    }
}

/// `PHONE_NUMBER_DETAIL_FIELDS`.
const PHONE_NUMBER_DETAIL_FIELDS: &str = "id,quality_rating,messaging_limit_tier,whatsapp_business_manager_messaging_limit,code_verification_status,account_mode,display_phone_number,name_status,verified_name,webhook_configuration,throughput,last_onboarded_time,platform_type,certificate,health_status";
/// `PHONE_NUMBER_REGISTER_FIELDS`.
const PHONE_NUMBER_REGISTER_FIELDS: &str =
    "id,display_phone_number,verified_name,code_verification_status,status,platform_type";

/// `findPhoneNumberDetail`.
pub async fn phone_number_detail(
    graph_url: &str,
    auth: &WhatsappAuth,
) -> Result<Value, GraphError> {
    auth.get(
        graph_url,
        &auth.phone_number_id,
        &[("fields", PHONE_NUMBER_DETAIL_FIELDS)],
    )
    .await
}

/// `findWaba` with `fields`.
pub async fn waba(graph_url: &str, auth: &WhatsappAuth, fields: &str) -> Result<Value, GraphError> {
    auth.get(graph_url, &auth.waba_id, &[("fields", fields)])
        .await
}

/// `findConversationalAutomation`.
pub async fn conversational_automation(
    graph_url: &str,
    auth: &WhatsappAuth,
) -> Result<Value, GraphError> {
    let body = auth
        .get(
            graph_url,
            &auth.phone_number_id,
            &[("fields", "conversational_automation")],
        )
        .await?;
    let automation = body.get("conversational_automation");
    let pick = |key: &str, default: Value| {
        automation
            .and_then(|a| a.get(key))
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or(default)
    };
    Ok(json!({
        "enable_welcome_message": pick("enable_welcome_message", json!(false)),
        "prompts": pick("prompts", json!([])),
        "commands": pick("commands", json!([])),
    }))
}

/// `requestVerificationCode` (language `en_US`).
pub async fn request_verification_code(
    graph_url: &str,
    auth: &WhatsappAuth,
    phone_number_id: &str,
    code_method: &str,
) -> Result<Value, GraphError> {
    auth.post(
        graph_url,
        &format!("{phone_number_id}/request_code"),
        &json!({ "code_method": code_method, "language": "en_US" }),
    )
    .await
}

/// `verifyCode`.
pub async fn verify_code(
    graph_url: &str,
    auth: &WhatsappAuth,
    phone_number_id: &str,
    code: &str,
) -> Result<Value, GraphError> {
    auth.post(
        graph_url,
        &format!("{phone_number_id}/verify_code"),
        &json!({ "code": code }),
    )
    .await
}

/// `RegisterPhoneNumberResult` as `RegistrationOutcome`
/// (`toRegistrationOutcome`): `(status, error)`.
pub type Registration = (&'static str, Option<Value>);

/// `generatePin`: the last 6 digits of `phoneNumberId + wabaId`.
pub fn registration_pin(phone_number_id: &str, waba_id: &str) -> String {
    let sum = phone_number_id.parse::<u128>().unwrap_or(0) + waba_id.parse::<u128>().unwrap_or(0);
    let text = sum.to_string();
    text[text.len().saturating_sub(6)..].to_owned()
}

fn plain_error(message: &str, code: i64, kind: &str) -> Value {
    json!({
        "code": code,
        "subCode": Value::Null,
        "message": message,
        "type": kind,
        "at": chatbotx_db::repo::iso(now_ms()),
    })
}

/// `registerPhoneNumber` + `toRegistrationOutcome`: lists the WABA's numbers;
/// a Cloud API number that is `CONNECTED` is registered already; an
/// unverified one needs the code; else `POST /{id}/register` with the pin.
pub async fn register_phone_number(
    graph_url: &str,
    auth: &WhatsappAuth,
    phone_number_id: &str,
) -> Result<Registration, GraphError> {
    let list = auth
        .get(
            graph_url,
            &format!("{}/phone_numbers", auth.waba_id),
            &[("fields", PHONE_NUMBER_REGISTER_FIELDS)],
        )
        .await?;
    let number = list["data"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == phone_number_id));
    let Some(number) = number else {
        return Ok((
            "failed",
            Some(plain_error(
                "WhatsApp phone number was not found in the selected WhatsApp Business Account.",
                -1,
                "PhoneNumberNotFound",
            )),
        ));
    };
    if number["status"] == "CONNECTED" && number["platform_type"] == "CLOUD_API" {
        return Ok(("registered", None));
    }
    if number["code_verification_status"] != "VERIFIED" {
        return Ok((
            "pending_verification",
            Some(plain_error(
                "WhatsApp phone number verification is required before registration.",
                133_006,
                "PhoneVerificationRequired",
            )),
        ));
    }
    let pin = registration_pin(phone_number_id, &auth.waba_id);
    match auth
        .post(
            graph_url,
            &format!("{phone_number_id}/register"),
            &json!({ "messaging_product": "whatsapp", "pin": pin }),
        )
        .await
    {
        Ok(_) => Ok(("registered", None)),
        Err(error) if error.code == Some(133_006) || error.sub_code == Some(2_593_005) => {
            Ok(("pending_verification", Some(error.registration_error())))
        }
        Err(error) => Ok(("failed", Some(error.registration_error()))),
    }
}

/// `recordRegistrationOutcome`: the stored `registrationError`.
pub async fn record_registration(
    db: &chatbotx_db::Db,
    ws: i64,
    id: i64,
    (status, error): Registration,
) -> Result<Option<Value>, BusinessError> {
    tx::write(db, move |conn| {
        let values = vec![
            (
                "registrationStatus",
                chatbotx_db::rusqlite::types::Value::Text(status.to_owned()),
            ),
            (
                "registrationError",
                match &error {
                    Some(error) => chatbotx_db::rusqlite::types::Value::Text(error.to_string()),
                    None => chatbotx_db::rusqlite::types::Value::Null,
                },
            ),
        ];
        Ok(integration_whatsapp::update_registration(conn, id, ws, &values)?.map(|j| j.0))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_is_the_last_six_digits_of_the_sum() {
        assert_eq!(
            registration_pin("100000000000001", "200000000123456"),
            "123457"
        );
        assert_eq!(registration_pin("12", "30"), "42");
    }

    #[test]
    fn routing_roles_reads_only_the_snapshot_shape() {
        let good = Json(
            json!({ "enabled": true, "roles": ["escalation"], "isStandby": false, "checkedAt": "2026-01-01T00:00:00.000Z" }),
        );
        assert_eq!(
            read_routing_roles(Some(&good))["roles"],
            json!(["escalation"])
        );
        let bad = Json(json!({ "enabled": true, "roles": [1] }));
        assert_eq!(read_routing_roles(Some(&bad)), Value::Null);
        assert_eq!(read_routing_roles(None), Value::Null);
    }

    #[test]
    fn action_message_prefers_meta_user_text() {
        let error = GraphError {
            message: "(#100) Label".into(),
            code: Some(100),
            sub_code: None,
            kind: None,
            user_title: Some("Title".into()),
            user_message: Some("Reason".into()),
            fbtrace_id: None,
            raw_message: None,
        };
        assert_eq!(error.action_message("fallback"), "Reason");
        let bare = GraphError {
            message: String::new(),
            user_title: None,
            user_message: None,
            ..error
        };
        assert_eq!(bare.action_message("fallback"), "fallback");
    }
}
