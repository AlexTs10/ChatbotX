//! `app/dynamic-images/route.ts`: `GET /dynamic-images?dynamicImageId=&userId=`,
//! the trigger URL of a dynamic image. Lives here (not in `chatbotx-api`)
//! because the `{{variable}}` sources are the worker's.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::extract::{RawQuery, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_api::handlers::ApiState;
use chatbotx_business::dynamic_image::{self as service, Deps, render};
use chatbotx_business::tx;
use serde_json::{Value, json};

#[derive(Clone)]
struct TriggerState {
    api: Arc<ApiState>,
    deps: chatbotx_worker::Deps,
}

pub fn router(api: Arc<ApiState>, deps: chatbotx_worker::Deps) -> Router {
    Router::new()
        .route("/dynamic-images", get(trigger))
        .with_state(TriggerState { api, deps })
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn message(status: StatusCode, text: &str) -> Response {
    json_response(status, json!({ "message": text }))
}

fn redirect(url: &str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, url)]).into_response()
}

fn server_error(error: impl std::fmt::Display) -> Response {
    tracing::error!(%error, "dynamic-image: render failed");
    message(StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error")
}

async fn servable(api: &ApiState, workspace_id: i64) -> bool {
    tx::read(&api.db, move |c| {
        Ok(c.query_row(
            r#"SELECT count(*) FROM "Workspace" WHERE "id" = ?1 AND "scheduledDeletionAt" IS NULL"#,
            [workspace_id],
            |r| r.get::<_, i64>(0),
        )
        .map_err(chatbotx_db::repo::Error::from)?
            > 0)
    })
    .await
    .unwrap_or(false)
}

async fn trigger(State(state): State<TriggerState>, RawQuery(query): RawQuery) -> Response {
    let query: HashMap<String, String> =
        url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
            .into_owned()
            .collect();
    let api = &state.api;
    let Some(image_id) = query.get("dynamicImageId").filter(|v| !v.is_empty()) else {
        return message(StatusCode::BAD_REQUEST, "dynamicImageId is required");
    };
    let row = match image_id.parse::<i64>() {
        Ok(id) => match service::find_unscoped(&api.db, id).await {
            Ok(row) => row,
            Err(error) => return server_error(error),
        },
        Err(_) => None,
    };
    let Some(row) = row else {
        return message(StatusCode::NOT_FOUND, "Dynamic image not found");
    };
    if !row.enabled {
        return message(StatusCode::NOT_FOUND, "Dynamic image is disabled");
    }
    if !servable(api, row.workspace_id).await {
        return json_response(
            StatusCode::GONE,
            json!({ "code": "workspaceScheduledDeletion" }),
        );
    }
    let storage = chatbotx_api::handlers::contacts::tenant_urls(api).storage_url;
    let to_background = || match service::background_url(&row, &storage) {
        Some(url) => redirect(&url),
        None => message(
            StatusCode::NOT_FOUND,
            "Dynamic image has no rendered background",
        ),
    };
    let Some(user_id) = query.get("userId").filter(|v| !v.is_empty()) else {
        return to_background();
    };
    let inbox = match service::find_latest_contact_inbox(&api.db, row.workspace_id, user_id.clone())
        .await
    {
        Ok(Some(inbox)) => inbox,
        Ok(None) => return to_background(),
        Err(error) => return server_error(error),
    };
    let contact_id = inbox.contact_id;
    match service::find_cached_url_for_contact(&api.db, &row, contact_id).await {
        Ok(Some(url)) => return redirect(&url),
        Ok(None) => {}
        Err(error) => return server_error(error),
    }
    let Some(files) = api.files.as_deref() else {
        return server_error("File storage is not configured");
    };
    let Ok(document) = serde_json::from_value::<render::Document>(row.data.0.clone()) else {
        return server_error("Invalid dynamic image document");
    };
    let dynamic = render::dynamic_element_ids(&document);
    let urls = chatbotx_api::handlers::contacts::tenant_urls(api);
    let profile = chatbotx_business::contact::channel_profile::ProfileDeps {
        db: &api.db,
        http: &chatbotx_business::channel_api::LiveChannelHttp,
        uploader: Some(files),
    };
    let avatar = match service::contact_avatar_url_mirrored(
        profile,
        row.workspace_id,
        contact_id,
        Some((inbox.id, inbox.channel.as_str())),
        &urls,
    )
    .await
    {
        Ok(avatar) => avatar,
        Err(error) => return server_error(error),
    };
    let with_images =
        match service::resolve_dynamic_elements(&api.db, contact_id, avatar, &row.data.0).await {
            Ok(value) => value,
            Err(error) => return server_error(error),
        };
    let resolved = match &state.deps.variables {
        Some(variables) => match resolve(variables, contact_id, inbox.id, &with_images).await {
            Ok(value) => value,
            Err(error) => return server_error(error),
        },
        None => with_images,
    };
    let deps = Deps {
        db: &api.db,
        files,
        storage_url: &storage,
    };
    match service::render_for_contact(deps, &row, contact_id, &resolved, &dynamic).await {
        Ok(url) => redirect(&url),
        Err(error) => server_error(error),
    }
}

/// `resolveContactVariablesDeep(contactId, document, { contactInbox })`.
async fn resolve(
    variables: &chatbotx_worker::variables::VariableDeps,
    contact_id: i64,
    contact_inbox_id: i64,
    document: &Value,
) -> Result<Value, String> {
    use chatbotx_core::variables::Variables;
    use chatbotx_core::variables::contact_variable::ContactInboxArg;
    use chatbotx_core::variables::deep_replace::{DeepOptions, DeepSource};
    let clock = chatbotx_kv::SystemClock;
    Variables::new(
        &variables.sources,
        &variables.cache,
        &variables.encryption,
        &clock,
    )
    .resolve_contact_variables_deep(
        &contact_id.to_string(),
        document,
        DeepSource {
            contact_inbox: ContactInboxArg::Id(contact_inbox_id.to_string()),
            conversation: None,
            workspace: None,
            appointment_id: None,
            sent_by_user_id: None,
        },
        DeepOptions::default(),
    )
    .await
    .map_err(|e| e.to_string())
}
