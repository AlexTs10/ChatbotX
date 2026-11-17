//! `workspaceSettingsPublicRouter` (`features/workspaces/api/public.ts`):
//! `GET` / `PATCH /v1/workspace/settings` (Q.D11, scope `settings`).

use chatbotx_business::workspace::{self, SettingsPatch, WorkspaceWrite, codes};
use chatbotx_db::repo::models::Workspace;
use serde_json::{Value, json};

use super::input::{Input, Str, key};
use super::{ApiState, Call, FINDING, MUTATING, Reply, business_error, json};

/// The routes of this module.
pub const ROUTES: &[&str] = &["workspaceSettings.get", "workspaceSettings.update"];

/// `SMART_RESPONSE_DELAY_OPTIONS`.
pub const SMART_RESPONSE_DELAY_OPTIONS: &[i64] = &[3, 5, 10, 15, 20, 30, 45, 60, 120, 180];

/// `defaultReplyFrequencies`.
pub const DEFAULT_REPLY_FREQUENCIES: &[&str] = &["allTime", "oncePerHour", "oncePerDay"];

fn is_brand_color(text: &str) -> bool {
    text.len() == 7 && text.starts_with('#') && text[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// `smartResponseDelaySeconds` (`z.number().int().refine(option).nullable()`).
fn smart_delay(input: &mut Input) -> Option<Option<i64>> {
    match input.get("smartResponseDelaySeconds").cloned() {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => {
            let path = key("smartResponseDelaySeconds");
            let number = input.number_at(
                &path,
                Some(&value),
                super::input::Num {
                    int: true,
                    ..Default::default()
                },
                true,
            )?;
            let seconds = number as i64;
            if SMART_RESPONSE_DELAY_OPTIONS.contains(&seconds) {
                Some(Some(seconds))
            } else {
                let options = SMART_RESPONSE_DELAY_OPTIONS
                    .iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                input.issue(&path, "custom", &format!("Must be one of {options}"));
                None
            }
        }
    }
}

/// A string refined against a value list (`z.string().refine(...)`).
fn refined(
    input: &mut Input,
    name: &str,
    allowed: impl Fn(&str) -> bool,
    message: &str,
) -> Option<String> {
    let text = input.opt_string(name, Str::default())?;
    if allowed(&text) {
        Some(text)
    } else {
        input.issue(&key(name), "custom", message);
        None
    }
}

/// `updateWorkspaceSettingsRequest`.
fn settings_request(input: &mut Input) -> SettingsPatch {
    let default_reply = input.nullish_id("defaultReply");
    let default_reply_frequency = {
        let value = input.get("defaultReplyFrequency").cloned();
        input.enum_at(
            &key("defaultReplyFrequency"),
            value.as_ref(),
            DEFAULT_REPLY_FREQUENCIES,
            false,
        )
    };
    let smart_response_delay_seconds = smart_delay(input);
    let capi_limited_data_use = input.opt_bool("capiLimitedDataUse");
    let logo = match input.get("logo").cloned() {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => {
            let path = key("logo");
            input
                .string_at(&path, Some(&value), Str::max(2048), true)
                .and_then(|text| {
                    let ok = url::Url::parse(&text)
                        .is_ok_and(|url| matches!(url.scheme(), "http" | "https"));
                    if ok {
                        Some(Some(text))
                    } else {
                        input.issue(&path, "invalid_format", "Invalid URL");
                        None
                    }
                })
        }
    };
    let target_country = refined(
        input,
        "targetCountry",
        codes::is_country_code,
        "Must be an ISO 3166-1 alpha-2 country code or \"unknown\"",
    );
    let language = {
        let value = input.get("language").cloned();
        input.enum_at(
            &key("language"),
            value.as_ref(),
            codes::LANGUAGE_CODES,
            false,
        )
    };
    let timezone = refined(
        input,
        "timezone",
        codes::is_timezone_code,
        "Must be an IANA timezone such as Asia/Ho_Chi_Minh",
    );
    let brand_color = {
        let value = input.get("brandColor").cloned();
        input.string_at(
            &key("brandColor"),
            value.as_ref(),
            Str {
                pattern: Some(is_brand_color),
                ..Str::default()
            },
            false,
        )
    };
    let development_mode = input.opt_bool("developmentMode");
    SettingsPatch {
        default_reply,
        default_reply_frequency,
        smart_response_delay_seconds,
        capi_limited_data_use,
        logo,
        target_country,
        language,
        timezone,
        brand_color,
        development_mode,
    }
}

/// `toResource`: the settings fields; a stored logo path becomes a URL.
pub fn settings_json(state: &ApiState, workspace: &Workspace) -> Value {
    let storage = super::contacts::tenant_urls(state).storage_url;
    let logo = workspace.logo.as_deref().map(|path| {
        chatbotx_core::utils::storage::get_public_file_url(path, &storage)
            .unwrap_or_else(|_| path.to_string())
    });
    json!({
        "defaultReply": workspace.default_reply,
        "defaultReplyFrequency": workspace.default_reply_frequency,
        "smartResponseDelaySeconds": workspace.smart_response_delay_seconds,
        "capiLimitedDataUse": workspace.capi_limited_data_use,
        "logo": logo,
        "targetCountry": workspace.target_country,
        "language": workspace.language,
        "timezone": workspace.timezone,
        "brandColor": workspace.brand_color,
        "developmentMode": workspace.development_mode,
    })
}

/// The side effects of a workspace write: its audit row and pipeline refresh.
pub async fn after_write(state: &ApiState, write: &WorkspaceWrite) {
    if let Some(detail) = &write.audit {
        state.record_audit("update", detail).await;
    }
    state
        .publish_pipeline_changes(&write.currency_changes)
        .await;
}

/// Runs one of [`ROUTES`].
pub async fn handle(state: &ApiState, route_id: &str, call: Call<'_>) -> Reply {
    let ws = call.workspace_id;
    match route_id {
        "workspaceSettings.get" => {
            Input::from_query(call.params, call.query).finish()?;
            let workspace = workspace::find_by_id(&state.db, ws)
                .await
                .map_err(|e| business_error(e, FINDING))?;
            Ok(json(200, settings_json(state, &workspace)))
        }
        "workspaceSettings.update" => {
            let mut input = Input::from_body(call.params, call.body);
            let patch = settings_request(&mut input);
            input.finish()?;
            let write = workspace::update_settings(&state.db, ws, patch)
                .await
                .map_err(|e| business_error(e, MUTATING))?;
            after_write(state, &write).await;
            Ok(json(200, settings_json(state, &write.workspace)))
        }
        _ => Err(crate::routes::not_ported_error()),
    }
}
