//! `features/templates/api/public/{templates,installations}.ts` (Q.D4 slice 1):
//! the publisher reads, share settings and delete, the locked-resource check,
//! and the installer's list, settings and sync history. The routes that need
//! the snapshot save, install, sync, uninstall or preview engines answer the
//! "not ported" 404 (specs/Q.D4.md).

use chatbotx_business::template::permissions::Actor;
use chatbotx_business::template::{
    RESOURCE_KINDS, installations, list_locked_resource_ids, service,
};
use serde_json::{Value, json};

use super::input::{Input, Num, Str, key};
use super::{
    ApiState, Call, FINDING, LISTING, MUTATING, Principal, Reply, business_error, date, id, json,
    no_content, opt_date,
};
use crate::orpc::OrpcError;

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "templates.list",
    "templates.get",
    "templates.delete",
    "templates.listLockedResources",
    "templates.getLink",
    "templates.updateShare",
    "templates.rotateShare",
    "templates.listVersions",
    "templates.getAnalytics",
    "templateInstallations.list",
    "templateInstallations.update",
    "templateInstallations.listSyncs",
    "templateInstallations.getSync",
];

const INSTALLATION_STATUSES: &[&str] = &[
    "pending",
    "installing",
    "completed",
    "partial",
    "failed",
    "uninstalled",
];

/// Runs one of [`ROUTES`].
pub async fn handle(
    state: &ApiState,
    route_id: &str,
    call: Call<'_>,
    principal: &Principal,
) -> Reply {
    // `tokenActor`: anything but an explicit `full` token is read-only.
    let actor = Actor::ApiToken {
        full: !principal.read_only,
    };
    match route_id {
        "templates.list" => list(state, call).await,
        "templates.get" => get(state, call).await,
        "templates.delete" => delete(state, call, &actor).await,
        "templates.listLockedResources" => locked_resources(state, call).await,
        "templates.getLink" => get_link(state, call, &actor).await,
        "templates.updateShare" => update_share(state, call, &actor).await,
        "templates.rotateShare" => rotate_share(state, call, &actor).await,
        "templates.listVersions" => list_versions(state, call).await,
        "templates.getAnalytics" => analytics(state, call, &actor).await,
        "templateInstallations.list" => list_installations(state, call, &actor).await,
        "templateInstallations.update" => update_installation(state, call, &actor).await,
        "templateInstallations.listSyncs" => list_syncs(state, call).await,
        "templateInstallations.getSync" => get_sync(state, call).await,
        _ => Err(crate::routes::not_ported_error()),
    }
}

/// `publicTemplateBaseResource` (no selection).
fn base_json(template: &service::Template) -> Value {
    json!({
        "id": id(template.id),
        "name": template.name,
        "description": template.description,
        "imageUrl": template.image_url,
        "publisherName": template.publisher_name,
        "youtubeVideoId": template.youtube_video_id,
        "testLink": template.test_link,
        "defaultPermissions": template.default_permissions,
        "createInstallFolder": template.create_install_folder,
        "defaultAutoUpdate": template.default_auto_update,
        "shareEnabled": template.share_enabled,
        "shareExpiresAt": opt_date(template.share_expires_at),
        "categoryCounts": template.category_counts,
        "createdAt": date(template.created_at),
        "updatedAt": date(template.updated_at),
    })
}

/// `shareLink`: the install link and the id / key pair.
fn link_json(state: &ApiState, template: &service::Template) -> Value {
    json!({
        "url": service::share_url(&state.app_url, template.id, &template.share_token),
        "shareEnabled": template.share_enabled,
        "shareExpiresAt": opt_date(template.share_expires_at),
        "templateId": id(template.id),
        "templateKey": template.share_token,
    })
}

/// The `id` path param (`zodBigintAsString`).
fn path_id(call: Call<'_>, name: &str) -> Result<i64, OrpcError> {
    let mut input = Input::from_query(call.params, None);
    let value = input.digits_id(name);
    input.finish()?;
    Ok(value.unwrap_or(-1))
}

async fn list(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let cursor = input.opt_string("cursor", Str::max(512));
    let limit = input.opt_coerce_int("limit", Num::int(1.0, 100.0));
    input.finish()?;
    let page = service::list_page(&state.db, call.workspace_id, cursor, limit)
        .await
        .map_err(|e| business_error(e, LISTING))?;
    let items: Vec<Value> = page
        .items
        .iter()
        .map(|(template, count)| {
            let mut item = base_json(template);
            item["installCount"] = json!(count);
            item
        })
        .collect();
    Ok(json(
        200,
        json!({ "items": items, "nextCursor": page.next_cursor }),
    ))
}

async fn get(state: &ApiState, call: Call<'_>) -> Reply {
    let template_id = path_id(call, "id")?;
    let template = service::get_detail(&state.db, call.workspace_id, template_id)
        .await
        .map_err(|e| business_error(e, FINDING))?;
    let mut out = base_json(&template);
    out["selection"] = template.selection.clone();
    Ok(json(200, out))
}

async fn delete(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let template_id = path_id(call, "id")?;
    service::soft_delete(&state.db, call.workspace_id, template_id, actor)
        .await
        .map_err(|e| business_error(e, MUTATING))?;
    Ok(no_content())
}

async fn locked_resources(state: &ApiState, call: Call<'_>) -> Reply {
    let (kind, ids) = locked_input(&Input::from_body(call.params, call.body))?;
    let (locked, undeletable) = list_locked_resource_ids(&state.db, call.workspace_id, &kind, &ids)
        .await
        .map_err(|e| business_error(e, LISTING))?;
    Ok(json(
        200,
        json!({ "lockedIds": locked, "undeletableIds": undeletable }),
    ))
}

/// `{ resourceKind, resourceIds }` (1..=200 bigint ids; the `/rpc` procedure
/// allows an empty list).
pub(crate) fn locked_input(input: &Input) -> Result<(String, Vec<i64>), OrpcError> {
    let mut input = input.clone();
    let kind_value = input.get("resourceKind").cloned();
    let kind = input.enum_at(
        &key("resourceKind"),
        kind_value.as_ref(),
        RESOURCE_KINDS,
        true,
    );
    let ids = match input.get("resourceIds").cloned() {
        Some(Value::Array(items)) => {
            if items.len() > 200 {
                input.issue(
                    &key("resourceIds"),
                    "too_big",
                    "Too big: expected array to have <=200 items",
                );
            }
            let mut ids = Vec::new();
            for (index, item) in items.iter().enumerate() {
                let mut path = key("resourceIds");
                path.push(super::input::Seg::Index(index));
                match item.as_str() {
                    Some(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
                        ids.push(text.parse().unwrap_or(-1));
                    }
                    _ => input.issue(
                        &path,
                        "invalid_format",
                        "Invalid string: must match pattern /^\\d+$/",
                    ),
                }
            }
            ids
        }
        other => {
            input.issue(
                &key("resourceIds"),
                "invalid_type",
                &format!(
                    "Invalid input: expected array, received {}",
                    if other.is_none() {
                        "undefined"
                    } else {
                        "string"
                    }
                ),
            );
            Vec::new()
        }
    };
    input.finish()?;
    Ok((kind.unwrap_or_default(), ids))
}

async fn get_link(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let template_id = path_id(call, "id")?;
    // The link is an install capability: reading it is a manage-level act.
    chatbotx_business::template::permissions::assert_can_manage(actor)
        .map_err(|e| business_error(e, FINDING))?;
    let template = service::get_detail(&state.db, call.workspace_id, template_id)
        .await
        .map_err(|e| business_error(e, FINDING))?;
    Ok(json(200, link_json(state, &template)))
}

async fn update_share(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let template_id = input.digits_id("id");
    let enabled = input.bool("enabled");
    let expires_at = match input.get("expiresAt").cloned() {
        None => None,
        Some(Value::Null) => Some(None),
        Some(_) => {
            // `z.string().datetime()`: UTC (`Z`) only.
            let ends_z = input
                .get("expiresAt")
                .and_then(Value::as_str)
                .is_some_and(|t| t.ends_with('Z'));
            let parsed = super::polls::datetime_at(&mut input, "expiresAt", true);
            match parsed {
                Some(ms) if ends_z => Some(Some(ms)),
                Some(_) => {
                    input.issue(&key("expiresAt"), "invalid_format", "Invalid ISO datetime");
                    None
                }
                None => None,
            }
        }
    };
    input.finish()?;
    let template = service::update_share_settings(
        &state.db,
        call.workspace_id,
        template_id.unwrap_or(-1),
        enabled.unwrap_or_default(),
        expires_at,
        actor,
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?;
    Ok(json(200, link_json(state, &template)))
}

async fn rotate_share(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let template_id = path_id(call, "id")?;
    let template = service::rotate_share_token(&state.db, call.workspace_id, template_id, actor)
        .await
        .map_err(|e| business_error(e, MUTATING))?;
    Ok(json(200, link_json(state, &template)))
}

async fn list_versions(state: &ApiState, call: Call<'_>) -> Reply {
    let template_id = path_id(call, "id")?;
    let items = service::list_versions(&state.db, call.workspace_id, template_id)
        .await
        .map_err(|e| business_error(e, FINDING))?;
    let items: Vec<Value> = items
        .iter()
        .map(|v| json!({ "version": v.version, "createdAt": date(v.created_at), "changes": v.changes }))
        .collect();
    Ok(json(200, json!({ "items": items })))
}

async fn analytics(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let template_id = path_id(call, "id")?;
    let out = service::install_analytics(&state.db, call.workspace_id, template_id, actor)
        .await
        .map_err(|e| business_error(e, FINDING))?;
    Ok(json(200, out))
}

async fn list_installations(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let status_value = input.get("status").cloned();
    let status = input.enum_at(
        &key("status"),
        status_value.as_ref(),
        INSTALLATION_STATUSES,
        false,
    );
    let active_only = input.opt_coerced_bool("activeOnly");
    let before_id = if input.has("cursor") {
        input.digits_id("cursor")
    } else {
        None
    };
    let limit = input.opt_coerce_int("limit", Num::int(1.0, 100.0));
    input.finish()?;
    let out = installations::list_page(
        &state.db,
        call.workspace_id,
        installations::ListInstallations {
            status,
            active_only: active_only.unwrap_or_default(),
            before_id,
            limit,
        },
        actor,
    )
    .await
    .map_err(|e| business_error(e, LISTING))?;
    Ok(json(200, out))
}

async fn update_installation(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let installation_id = input.digits_id("id");
    let auto_update = input.opt_bool("autoUpdate");
    let grant = input.opt_bool("publisherUninstallGrant");
    input.finish()?;
    let installation_id = installation_id.unwrap_or(-1);
    if let Some(granted) = grant {
        installations::set_publisher_uninstall_grant(
            &state.db,
            call.workspace_id,
            installation_id,
            actor,
            granted,
        )
        .await
        .map_err(|e| business_error(e, MUTATING))?;
    }
    if let Some(auto_update) = auto_update {
        installations::set_auto_update(
            &state.db,
            call.workspace_id,
            installation_id,
            actor,
            auto_update,
        )
        .await
        .map_err(|e| business_error(e, MUTATING))?;
    }
    Ok(json(200, json!({ "ok": true })))
}

async fn list_syncs(state: &ApiState, call: Call<'_>) -> Reply {
    let installation_id = path_id(call, "id")?;
    let items = installations::list_syncs(&state.db, call.workspace_id, installation_id)
        .await
        .map_err(|e| business_error(e, FINDING))?;
    Ok(json(200, json!({ "items": items })))
}

async fn get_sync(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, None);
    let installation_id = input.digits_id("id");
    let sync_id = input.digits_id("syncId");
    input.finish()?;
    let out = installations::get_sync(
        &state.db,
        call.workspace_id,
        installation_id.unwrap_or(-1),
        sync_id.unwrap_or(-1),
    )
    .await
    .map_err(|e| business_error(e, FINDING))?;
    Ok(json(200, out))
}
