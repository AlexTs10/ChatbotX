//! `features/templates/api/public/{templates,installations,previews}.ts`
//! (Q.D4 slices 1-5): the publisher reads, save, share settings and delete,
//! the locked-resource check, the installer routes (install, sync, restore,
//! uninstall, cancel) and the publisher's installer list and uninstall.

use chatbotx_business::template::collect::CATEGORIES;
use chatbotx_business::template::permissions::Actor;
use chatbotx_business::template::{
    RESOURCE_KINDS, installations, list_locked_resource_ids, request as install_request,
    save as template_save, service, syncs as template_syncs, uninstall as template_uninstall,
};
use serde_json::{Value, json};

use super::input::{Input, Num, Seg, Str, key};
use super::{
    ApiState, CREATING, Call, FINDING, LISTING, MUTATING, Principal, Reply, business_error, date,
    id, json, no_content, opt_date,
};
use crate::orpc::OrpcError;

/// The routes of this module.
pub const ROUTES: &[&str] = &[
    "templates.list",
    "templates.create",
    "templates.update",
    "templates.listSelectableResources",
    "templates.previewDependencies",
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
    "templateInstallations.install",
    "templateInstallations.retry",
    "templateInstallations.sync",
    "templateInstallations.restoreBackup",
    "templateInstallations.previewUninstall",
    "templateInstallations.uninstall",
    "templateInstallations.cancelUninstall",
    "templates.listInstallers",
    "templates.getInstallerIssues",
    "templates.previewUninstallInstaller",
    "templates.uninstallInstaller",
    "templateInstallations.get",
    "templateInstallations.listResources",
    "templatePreviews.get",
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
        "templates.create" => save(state, call, &actor, false).await,
        "templates.update" => save(state, call, &actor, true).await,
        "templates.listSelectableResources" => selectable(state, call, &actor).await,
        "templates.previewDependencies" => preview_dependencies(state, call, &actor).await,
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
        "templateInstallations.install" => install(state, call, actor).await,
        "templateInstallations.retry" => retry(state, call, actor).await,
        "templateInstallations.sync" => sync(state, call, actor).await,
        "templateInstallations.restoreBackup" => restore_backup(state, call, actor).await,
        "templateInstallations.previewUninstall" => preview_uninstall(state, call, &actor).await,
        "templateInstallations.uninstall" => uninstall(state, call, &actor).await,
        "templateInstallations.cancelUninstall" => cancel_uninstall(state, call, &actor).await,
        "templates.listInstallers" => list_installers(state, call, &actor).await,
        "templates.getInstallerIssues" => installer_issues(state, call, &actor).await,
        "templates.previewUninstallInstaller" => {
            preview_uninstall_installer(state, call, &actor).await
        }
        "templates.uninstallInstaller" => uninstall_installer(state, call, &actor).await,
        "templateInstallations.get" => get_installation(state, call, actor).await,
        "templateInstallations.listResources" => list_resources(state, call).await,
        "templatePreviews.get" => preview(state, call, principal).await,
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

/// `templateSelectionSchema`: `{ [category]: {mode:"all", lockAll?} | {mode:"ids", ids, lockedIds?} }`.
fn selection_at(input: &mut Input, name: &str) -> Value {
    let path = key(name);
    let Some(value) = input.get(name).cloned() else {
        input.issue(
            &path,
            "invalid_type",
            "Invalid input: expected record, received undefined",
        );
        return json!({});
    };
    let Value::Object(map) = value else {
        input.issue(
            &path,
            "invalid_type",
            "Invalid input: expected record, received string",
        );
        return json!({});
    };
    let mut out = serde_json::Map::new();
    for (category, chosen) in map {
        let mut at = path.clone();
        at.push(Seg::Key(category.clone()));
        if !CATEGORIES.contains(&category.as_str()) {
            input.issue(&at, "invalid_key", "Invalid key in record");
            continue;
        }
        let strings = |v: &Value| -> Option<Vec<Value>> {
            v.as_array()
                .filter(|items| items.iter().all(Value::is_string))
                .cloned()
        };
        let normalized = match chosen["mode"].as_str() {
            Some("all") if chosen.get("lockAll").is_none_or(Value::is_boolean) => {
                let mut c = json!({ "mode": "all" });
                if let Some(lock) = chosen.get("lockAll") {
                    c["lockAll"] = lock.clone();
                }
                Some(c)
            }
            Some("ids") => match (
                strings(&chosen["ids"]),
                chosen.get("lockedIds").map(strings),
            ) {
                (Some(ids), None) => Some(json!({ "mode": "ids", "ids": ids })),
                (Some(ids), Some(Some(locked))) => {
                    Some(json!({ "mode": "ids", "ids": ids, "lockedIds": locked }))
                }
                _ => None,
            },
            _ => None,
        };
        match normalized {
            Some(c) => {
                out.insert(category, c);
            }
            None => input.issue(&at, "invalid_union", "Invalid input"),
        }
    }
    Value::Object(out)
}

/// `templatePermissionsSchema` (defaults: edit on, delete off).
fn permissions_at(input: &mut Input, name: &str) -> Value {
    let path = key(name);
    let value = input.get(name).cloned();
    let Some(Value::Object(map)) = value else {
        input.issue(
            &path,
            "invalid_type",
            "Invalid input: expected object, received undefined",
        );
        return json!({});
    };
    let mut flag = |field: &str, default: bool| -> bool {
        let mut at = path.clone();
        at.push(Seg::Key(field.to_string()));
        input.bool_at(&at, map.get(field), false).unwrap_or(default)
    };
    let allow_edit = flag("allowEdit", true);
    let allow_delete = flag("allowDelete", false);
    json!({ "allowEdit": allow_edit, "allowDelete": allow_delete })
}

/// `optionalText(max)`: `""` = null, else trimmed, at most `max`.
fn optional_text(
    input: &mut Input,
    name: &str,
    max: usize,
    check: Option<fn(&str) -> bool>,
) -> Option<String> {
    match input.get(name) {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) if t.is_empty() => None,
        _ => input.opt_string(
            name,
            Str {
                trim: true,
                min: None,
                max: Some(max),
                pattern: check,
            },
        ),
    }
}

fn is_youtube_id(text: &str) -> bool {
    text.len() == 11
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn is_http_url(text: &str) -> bool {
    url::Url::parse(text).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

/// `templates.create` / `templates.update`.
async fn save(state: &ApiState, call: Call<'_>, actor: &Actor, update: bool) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let template_id = if update { input.digits_id("id") } else { None };
    let name = input.string("name", Str::trimmed(1, 255));
    let description = optional_text(&mut input, "description", 2000, None);
    let image_url = optional_text(&mut input, "imageUrl", 500, None);
    let publisher_name = optional_text(&mut input, "publisherName", 255, None);
    let youtube_video_id = optional_text(&mut input, "youtubeVideoId", 11, Some(is_youtube_id));
    let test_link = match input.get("testLink") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) if t.is_empty() => None,
        Some(Value::String(t)) if is_http_url(t) => Some(t.clone()),
        Some(_) => {
            input.issue(&key("testLink"), "invalid_union", "Invalid input");
            None
        }
    };
    let selection = selection_at(&mut input, "selection");
    let default_permissions = permissions_at(&mut input, "defaultPermissions");
    let create_install_folder = input.bool("createInstallFolder");
    let default_auto_update = input.bool("defaultAutoUpdate");
    input.finish()?;
    let saved = template_save::save_template(
        &state.db,
        template_save::SaveInput {
            workspace_id: call.workspace_id,
            created_by: None,
            name: name.unwrap_or_default(),
            description,
            image_url,
            publisher_name,
            youtube_video_id,
            test_link,
            selection,
            default_permissions,
            create_install_folder: create_install_folder.unwrap_or_default(),
            default_auto_update: default_auto_update.unwrap_or_default(),
            existing_template_id: template_id,
        },
        actor,
    )
    .await
    .map_err(|e| business_error(e, if update { MUTATING } else { CREATING }))?;
    if saved.version_advanced {
        fan_out(state, &saved.template).await;
    }
    let template = service::get_detail(&state.db, call.workspace_id, saved.template.id)
        .await
        .map_err(|e| business_error(e, FINDING))?;
    let mut out = base_json(&template);
    out["selection"] = template.selection.clone();
    Ok(json(200, out))
}

/// `requestAutoUpdateFanOut`: best effort; the sweep catches up a miss.
pub(crate) async fn fan_out(state: &ApiState, template: &chatbotx_db::repo::models::Template) {
    let Some(jobs) = state.jobs.as_ref() else {
        return;
    };
    if let Err(error) = template_save::enqueue_fan_out(
        jobs.as_ref(),
        template.id,
        template.workspace_id,
        template.version,
        None,
        template_save::PRIORITY_BACKGROUND,
    )
    .await
    {
        tracing::warn!(template_id = template.id, error = %error.message(), "Could not enqueue the template auto-update fan-out; the sweep will");
    }
}

async fn selectable(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let category_value = input.get("category").cloned();
    let category = input.enum_at(&key("category"), category_value.as_ref(), CATEGORIES, true);
    let keyword = input.opt_string(
        "keyword",
        Str {
            trim: true,
            min: None,
            max: Some(255),
            pattern: None,
        },
    );
    let cursor = input.opt_string("cursor", Str::default());
    let limit = input.opt_coerce_int("limit", Num::int(1.0, 200.0));
    input.finish()?;
    let out = template_save::list_selectable_resources(
        &state.db,
        call.workspace_id,
        category.unwrap_or_default(),
        keyword,
        cursor,
        limit,
        actor,
    )
    .await
    .map_err(|e| business_error(e, LISTING))?;
    Ok(json(200, out))
}

async fn preview_dependencies(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let selection = selection_at(&mut input, "selection");
    input.finish()?;
    let out = template_save::preview_selection_dependencies(
        &state.db,
        call.workspace_id,
        selection,
        actor,
    )
    .await
    .map_err(|e| business_error(e, LISTING))?;
    Ok(json(200, out))
}

// ---------------------------------------------------------------------------
// Slice 3: install, retry, detail, resources, preview
// ---------------------------------------------------------------------------

/// `inboxBindingsSchema`: `zodBigintAsString` keys and values.
fn inbox_bindings_at(input: &mut Input, name: &str) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let Some(value) = input.get(name).cloned() else {
        return out;
    };
    let Value::Object(map) = value else {
        input.issue(&key(name), "invalid_type", "Invalid input: expected record");
        return out;
    };
    for (source, target) in map {
        let mut at = key(name);
        at.push(Seg::Key(source.clone()));
        let ok_key = super::input::is_bigint_id(&source);
        match target.as_str() {
            Some(t) if ok_key && super::input::is_bigint_id(t) => {
                out.insert(source, t.to_string());
            }
            _ => input.issue(&at, "invalid_type", "Invalid input"),
        }
    }
    out
}

async fn install(state: &ApiState, call: Call<'_>, actor: Actor) -> Reply {
    let mut input = Input::from_body(call.params, call.body);
    let share_token = input.string("shareToken", Str::trimmed(1, usize::MAX));
    let inbox_bindings = inbox_bindings_at(&mut input, "inboxBindings");
    let allow_duplicate = input.opt_bool("allowDuplicate");
    let accept = input.opt_bool("acceptPublisherUninstall");
    let auto_update = input.opt_bool("autoUpdate");
    input.finish()?;
    let requested = install_request::request(
        &state.db,
        state.jobs.as_deref(),
        install_request::InstallRequest {
            share_token: share_token.unwrap_or_default(),
            workspace_id: call.workspace_id,
            installed_by: None,
            allow_duplicate: allow_duplicate.unwrap_or_default(),
            accept_publisher_uninstall: accept,
            auto_update,
            idempotency_key: None,
            inbox_bindings,
        },
        actor,
    )
    .await
    .map_err(|e| business_error(e, CREATING))?;
    let mut out = json!({ "installationId": id(requested.installation_id) });
    if let Some(sync_id) = requested.sync_id {
        out["syncId"] = json!(id(sync_id));
    }
    Ok(json(202, out))
}

/// `templateInstallations.sync`: queues a sync to the newest version (202).
async fn sync(state: &ApiState, call: Call<'_>, actor: Actor) -> Reply {
    let installation_id = path_id(call, "id")?;
    let requested = template_syncs::request(
        &state.db,
        state.jobs.as_deref(),
        template_syncs::SyncRequest {
            workspace_id: call.workspace_id,
            installation_id,
            trigger: "manual",
            requested_by: None,
            auto_update: None,
            accept_publisher_uninstall: None,
            inbox_bindings: None,
        },
        &actor,
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?;
    Ok(json(202, json!({ "syncId": id(requested.sync_id) })))
}

/// `templateInstallations.restoreBackup`.
async fn restore_backup(state: &ApiState, call: Call<'_>, actor: Actor) -> Reply {
    let installation_id = path_id(call, "id")?;
    let backup_id = path_id(call, "backupId")?;
    template_syncs::restore_backup(
        &state.db,
        call.workspace_id,
        installation_id,
        backup_id,
        &actor,
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?;
    Ok(json(200, json!({ "ok": true })))
}

/// `templateInstallations.previewUninstall`.
async fn preview_uninstall(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let installation_id = path_id(call, "id")?;
    let counts = template_uninstall::preview_as_installer(
        &state.db,
        call.workspace_id,
        installation_id,
        actor,
    )
    .await
    .map_err(|e| business_error(e, FINDING))?;
    Ok(json(200, counts))
}

/// `templateInstallations.uninstall`: 202, runs in the background.
async fn uninstall(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let installation_id = path_id(call, "id")?;
    let requested = template_uninstall::uninstall_as_installer(
        &state.db,
        state.jobs.as_deref(),
        call.workspace_id,
        installation_id,
        actor,
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?;
    Ok(json(
        202,
        json!({
            "installationId": id(installation_id),
            "operationId": id(requested.operation_id),
            "uninstallState": requested.state,
        }),
    ))
}

/// `templateInstallations.cancelUninstall`.
async fn cancel_uninstall(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let installation_id = path_id(call, "id")?;
    let mut out = template_uninstall::cancel_as_installer(
        &state.db,
        call.workspace_id,
        installation_id,
        actor,
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?;
    out["installationId"] = id(installation_id);
    Ok(json(200, out))
}

fn is_publisher_ref(text: &str) -> bool {
    (16..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The `{ id, installerId }` path params (`publisherInstallerRequest`).
fn installer_params(call: Call<'_>) -> Result<(i64, String), OrpcError> {
    let mut input = Input::from_query(call.params, None);
    let template_id = input.digits_id("id");
    let installer = input.string(
        "installerId",
        Str {
            trim: true,
            pattern: Some(is_publisher_ref),
            ..Str::default()
        },
    );
    input.finish()?;
    Ok((template_id.unwrap_or(-1), installer.unwrap_or_default()))
}

/// `templates.listInstallers`.
async fn list_installers(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let template_id = input.digits_id("id");
    let search = input.opt_string("search", Str::trimmed(0, 255));
    let cursor = input.opt_string("cursor", Str::max(512));
    let limit = input.opt_coerce_int("limit", Num::int(1.0, 100.0));
    input.finish()?;
    let page = template_uninstall::list_installers(
        &state.db,
        call.workspace_id,
        template_id.unwrap_or(-1),
        actor,
        search,
        cursor,
        limit,
    )
    .await
    .map_err(|e| business_error(e, LISTING))?;
    Ok(json(200, page))
}

/// `templates.getInstallerIssues`.
async fn installer_issues(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let (template_id, installer) = installer_params(call)?;
    let out = template_uninstall::installer_issues(
        &state.db,
        call.workspace_id,
        template_id,
        installer,
        actor,
    )
    .await
    .map_err(|e| business_error(e, FINDING))?;
    Ok(json(200, out))
}

/// `templates.previewUninstallInstaller`.
async fn preview_uninstall_installer(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let (template_id, installer) = installer_params(call)?;
    let out = template_uninstall::preview_as_publisher(
        &state.db,
        call.workspace_id,
        template_id,
        installer,
        actor,
    )
    .await
    .map_err(|e| business_error(e, FINDING))?;
    Ok(json(200, out))
}

/// `templates.uninstallInstaller`: 202, runs in the installer's workspace.
async fn uninstall_installer(state: &ApiState, call: Call<'_>, actor: &Actor) -> Reply {
    let (template_id, installer) = installer_params(call)?;
    let requested = template_uninstall::uninstall_as_publisher(
        &state.db,
        state.jobs.as_deref(),
        call.workspace_id,
        template_id,
        installer.clone(),
        actor,
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?;
    Ok(json(
        202,
        json!({ "publisherRef": installer, "uninstallState": requested.state }),
    ))
}

async fn retry(state: &ApiState, call: Call<'_>, actor: Actor) -> Reply {
    let installation_id = path_id(call, "id")?;
    let retried = install_request::retry(
        &state.db,
        state.jobs.as_deref(),
        call.workspace_id,
        installation_id,
        actor,
        None,
    )
    .await
    .map_err(|e| business_error(e, MUTATING))?;
    Ok(json(202, json!({ "installationId": id(retried) })))
}

async fn get_installation(state: &ApiState, call: Call<'_>, actor: Actor) -> Reply {
    let installation_id = path_id(call, "id")?;
    let detail = install_request::get_detail(&state.db, call.workspace_id, installation_id, actor)
        .await
        .map_err(|e| business_error(e, FINDING))?;
    Ok(json(200, detail))
}

async fn list_resources(state: &ApiState, call: Call<'_>) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let installation_id = input.digits_id("id");
    let after_id = if input.has("cursor") {
        input.digits_id("cursor")
    } else {
        None
    };
    let limit = input.opt_coerce_int("limit", Num::int(1.0, 200.0));
    let category_value = input.get("category").cloned();
    let category = input.enum_at(&key("category"), category_value.as_ref(), CATEGORIES, false);
    input.finish()?;
    let page = install_request::list_resources(
        &state.db,
        call.workspace_id,
        installation_id.unwrap_or(-1),
        after_id,
        limit,
        category,
    )
    .await
    .map_err(|e| business_error(e, FINDING))?;
    Ok(json(200, page))
}

/// `templatePreviews.get`: a `read_only` token sees `installable: false`
/// (`templateManagePermissionDenied`) and never the collisions.
async fn preview(state: &ApiState, call: Call<'_>, principal: &Principal) -> Reply {
    let mut input = Input::from_query(call.params, call.query);
    let share_token = input.string("id", Str::trimmed(1, usize::MAX));
    input.finish()?;
    let preview = install_request::preview(
        &state.db,
        share_token.unwrap_or_default(),
        call.workspace_id,
        Actor::ApiToken { full: true },
        !principal.read_only,
    )
    .await
    .map_err(|e| business_error(e, FINDING))?;
    if principal.read_only && preview["installable"] == true {
        return Ok(json(
            200,
            json!({ "installable": false, "reason": "templateManagePermissionDenied" }),
        ));
    }
    Ok(json(200, preview))
}
