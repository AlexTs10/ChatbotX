//! `template-save.ts` (`saveTemplate`), `selectable-resources.ts`,
//! `selectable-cursor.ts`, `preview-selection-dependencies.ts` and
//! `install-preview.ts` `previewDependencies`, plus the template queue
//! producers (`queues/template`). One SQLite write transaction runs the
//! snapshot and the save, so no row lock is needed.

use chatbotx_db::Db;
use chatbotx_db::repo::models::Template as TemplateRow;
use chatbotx_db::repo::template::{
    SaveWithVersion, TemplateVersionSnapshot, create_with_first_version, find_live,
    save_with_version,
};
use chatbotx_db::repo::template_selectable_resource::{
    self as selectable, CategoryInput, KeysetCursor,
};
use chatbotx_db::repo::{Json, new_id};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_jobs::{Enqueuer, JobOptions, QueueName, Retention};
use serde_json::{Value, json};

use super::collect::{CATEGORIES, RESOURCE_CATEGORIES};
use super::permissions::{Actor, assert_can_manage, assert_can_read, assert_can_use_categories};
use super::snapshot::{
    self, Snapshot, describe_version_changes, entries_for_category, hash_content,
};
use super::{Blocking, blocking_ids_in, decode_text_cursor, encode_text_cursor};
use crate::errors::{BusinessError, Result};
use crate::tx;

/// `SaveTemplateInput` (the request fields, already validated).
#[derive(Debug, Clone)]
pub struct SaveInput {
    pub workspace_id: i64,
    pub created_by: Option<i64>,
    pub name: String,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub publisher_name: Option<String>,
    pub youtube_video_id: Option<String>,
    pub test_link: Option<String>,
    pub selection: Value,
    pub default_permissions: Value,
    pub create_install_folder: bool,
    pub default_auto_update: bool,
    pub existing_template_id: Option<i64>,
}

/// What a save did: the row, and whether an update advanced the version.
#[derive(Debug, Clone)]
pub struct Saved {
    pub template: TemplateRow,
    pub version_advanced: bool,
}

/// `context.workspace.tenantId`.
pub(crate) fn workspace_tenant_id(
    conn: &chatbotx_db::rusqlite::Connection,
    workspace_id: i64,
) -> Result<i64> {
    conn.query_row(
        r#"SELECT "tenantId" FROM "Workspace" WHERE "id" = ?1"#,
        [workspace_id],
        |r| r.get(0),
    )
    .map_err(super::db_error)
}

fn categories_in_use(counts: &Value) -> Vec<String> {
    counts
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, n)| n.as_i64().unwrap_or(0) > 0)
        .map(|(c, _)| c.clone())
        .collect()
}

fn assert_cover_image(
    conn: &chatbotx_db::rusqlite::Connection,
    workspace_id: i64,
    image: Option<&str>,
    current: Option<&str>,
) -> Result<()> {
    let Some(image) = image.filter(|i| !i.is_empty() && Some(*i) != current) else {
        return Ok(());
    };
    let file = chatbotx_db::repo::media_library_file::find_by_path(conn, workspace_id, image)?;
    if !file.is_some_and(|f| f.mime_type.starts_with("image/")) {
        return Err(BusinessError::validation(
            "imageUrl",
            "Pick the cover image from the images in this workspace's media library",
        ));
    }
    Ok(())
}

fn text(value: &Option<String>) -> Sql {
    value.clone().map_or(Sql::Null, Sql::Text)
}

fn json_text(value: &Value) -> Sql {
    Sql::Text(value.to_string())
}

/// `saveTemplate`: snapshot the selection and persist it with a new version.
pub async fn save_template(db: &Db, input: SaveInput, actor: &Actor) -> Result<Saved> {
    assert_can_manage(actor)?;
    let actor = actor.clone();
    tx::write(db, move |tx| {
        let conn: &chatbotx_db::rusqlite::Connection = tx;
        let tenant_id = workspace_tenant_id(conn, input.workspace_id)?;
        let Snapshot {
            payload,
            category_counts,
            ..
        } = snapshot::build_snapshot(conn, input.workspace_id, tenant_id, &input.selection)?;
        // Checked on the final payload: pulled-in dependencies count too.
        let used = categories_in_use(&category_counts);
        assert_can_use_categories(&actor, used.iter().map(String::as_str))?;
        let payload_hash = hash_content(&payload);
        let format_version = payload["formatVersion"].as_i64().unwrap_or(1);
        let contents: Vec<(&'static str, Sql)> = vec![
            ("name", Sql::Text(input.name.clone())),
            ("description", text(&input.description)),
            ("imageUrl", text(&input.image_url)),
            ("publisherName", text(&input.publisher_name)),
            ("youtubeVideoId", text(&input.youtube_video_id)),
            ("testLink", text(&input.test_link)),
            ("selection", json_text(&input.selection)),
            ("payload", json_text(&payload)),
            ("categoryCounts", json_text(&category_counts)),
            ("formatVersion", Sql::Integer(format_version)),
            ("defaultPermissions", json_text(&input.default_permissions)),
            (
                "createInstallFolder",
                Sql::Integer(input.create_install_folder.into()),
            ),
            (
                "defaultAutoUpdate",
                Sql::Integer(input.default_auto_update.into()),
            ),
        ];
        let Some(existing_id) = input.existing_template_id else {
            assert_cover_image(conn, input.workspace_id, input.image_url.as_deref(), None)?;
            let mut values = contents;
            values.extend([
                ("workspaceId", Sql::Integer(input.workspace_id)),
                ("tenantId", Sql::Integer(tenant_id)),
                (
                    "createdBy",
                    input.created_by.map_or(Sql::Null, Sql::Integer),
                ),
                (
                    "shareToken",
                    Sql::Text(super::service::generate_share_token()),
                ),
                ("shareEnabled", Sql::Integer(0)),
                ("version", Sql::Integer(1)),
                ("payloadHash", Sql::Text(payload_hash.clone())),
            ]);
            let template = create_with_first_version(
                conn,
                &values,
                new_id(),
                &TemplateVersionSnapshot {
                    payload: Json(payload),
                    payload_hash,
                    format_version,
                    changes: None,
                },
            )?;
            return Ok(Saved {
                template,
                version_advanced: false,
            });
        };
        let existing = find_live(conn, existing_id, input.workspace_id)?
            .ok_or_else(|| BusinessError::not_found("Template not found"))?;
        assert_cover_image(
            conn,
            input.workspace_id,
            input.image_url.as_deref(),
            existing.image_url.as_deref(),
        )?;
        // The stored content hashed with today's algorithm: an older stored
        // hash must not advance the version when nothing changed.
        let legacy_payload_hash = hash_content(&existing.payload.0);
        let changes =
            chatbotx_core::flow_config::import_export::parse_template_export(&existing.payload.0)
                .ok()
                .map(|previous| Json(describe_version_changes(&previous, &payload)));
        let saved = save_with_version(
            conn,
            &SaveWithVersion {
                id: existing.id,
                workspace_id: input.workspace_id,
                version_id: new_id(),
                set: contents,
                snapshot: TemplateVersionSnapshot {
                    payload: Json(payload),
                    payload_hash,
                    format_version,
                    changes,
                },
                legacy_payload_hash: Some(legacy_payload_hash),
                based_on_version: existing.version,
            },
        )?
        .ok_or_else(|| BusinessError::not_found("Template not found"))?;
        Ok(Saved {
            version_advanced: saved.version != existing.version,
            template: saved,
        })
    })
    .await
}

// ---------------------------------------------------------------------------
// Template queue producers
// ---------------------------------------------------------------------------

/// `TemplateJobPriority`.
pub const PRIORITY_INTERACTIVE: u32 = 1;
pub const PRIORITY_BACKGROUND: u32 = 10;

async fn enqueue(
    jobs: &dyn Enqueuer,
    name: &str,
    data: Value,
    job_id: String,
    priority: u32,
) -> Result<()> {
    jobs.add(
        QueueName::Template,
        name,
        json!({ "type": name, "data": data }),
        JobOptions {
            job_id: Some(job_id),
            priority: Some(priority),
            remove_on_complete: Some(Retention::Remove),
            remove_on_fail: Some(Retention::Remove),
            ..Default::default()
        },
    )
    .await
    .map(|_| ())
    .map_err(|e| BusinessError::Repo(chatbotx_db::repo::Error::msg(e.to_string())))
}

/// `enqueueFanOutTemplateUpdate`.
pub async fn enqueue_fan_out(
    jobs: &dyn Enqueuer,
    template_id: i64,
    workspace_id: i64,
    version: i64,
    after_id: Option<i64>,
    priority: u32,
) -> Result<()> {
    enqueue(
        jobs,
        "fanOutTemplateUpdate",
        json!({
            "templateId": template_id.to_string(),
            "workspaceId": workspace_id.to_string(),
            "version": version,
            "afterId": after_id.map(|id| id.to_string()),
        }),
        format!(
            "tpl-fanout-{template_id}-{version}-{}",
            after_id.map_or("first".to_string(), |id| id.to_string())
        ),
        priority,
    )
    .await
}

/// `enqueueInstallTemplate`.
pub async fn enqueue_install(
    jobs: &dyn Enqueuer,
    installation_id: i64,
    workspace_id: i64,
) -> Result<()> {
    enqueue(
        jobs,
        "installTemplate",
        json!({ "installationId": installation_id.to_string(), "workspaceId": workspace_id.to_string() }),
        format!("install-template-{installation_id}"),
        PRIORITY_INTERACTIVE,
    )
    .await
}

/// `enqueueSyncTemplateInstallation`.
pub async fn enqueue_sync(
    jobs: &dyn Enqueuer,
    sync_id: i64,
    workspace_id: i64,
    priority: u32,
) -> Result<()> {
    enqueue(
        jobs,
        "syncTemplateInstallation",
        json!({ "syncId": sync_id.to_string(), "workspaceId": workspace_id.to_string() }),
        format!("tpl-sync-{sync_id}"),
        priority,
    )
    .await
}

/// `enqueueUninstallTemplate`.
pub async fn enqueue_uninstall(
    jobs: &dyn Enqueuer,
    installation_id: i64,
    workspace_id: i64,
    attempt: i64,
) -> Result<()> {
    enqueue(
        jobs,
        "uninstallTemplate",
        json!({
            "installationId": installation_id.to_string(),
            "workspaceId": workspace_id.to_string(),
            "attempt": attempt,
        }),
        format!("tpl-uninstall-{installation_id}-{attempt}"),
        PRIORITY_INTERACTIVE,
    )
    .await
}

// ---------------------------------------------------------------------------
// Selectable resources
// ---------------------------------------------------------------------------

/// `SELECTABLE_RESOURCE_PAGE_SIZE`.
const PAGE_SIZE: i64 = 100;

fn invalid_cursor() -> BusinessError {
    BusinessError::exception("Invalid cursor", "invalidCursor", 400)
}

/// `assertCanListTemplateCategory`.
fn assert_can_list_category(actor: &Actor, category: &str) -> Result<()> {
    assert_can_manage(actor)?;
    assert_can_use_categories(actor, [category])
}

/// `listSelectableResources`: one page of a category for the picker, the
/// View-only copies flagged `locked`.
pub async fn list_selectable_resources(
    db: &Db,
    workspace_id: i64,
    category: String,
    keyword: Option<String>,
    cursor: Option<String>,
    limit: Option<i64>,
    actor: &Actor,
) -> Result<Value> {
    assert_can_list_category(actor, &category)?;
    let limit = limit.unwrap_or(PAGE_SIZE);
    let mut input = CategoryInput {
        workspace_id,
        keyword,
        offset: 0,
        limit,
        after: None,
    };
    match cursor.as_deref().filter(|c| !c.is_empty()) {
        None => {}
        Some(c) if c.bytes().all(|b| b.is_ascii_digit()) => {
            // A legacy offset cursor.
            if c.len() > 9 {
                return Err(invalid_cursor());
            }
            input.offset = c.parse().unwrap_or(0);
        }
        Some(c) => {
            let (key, id) = decode_text_cursor(c).ok_or_else(invalid_cursor)?;
            // Keywords page by `createdAt`: the key is its unix-ms text here.
            if category == "keywords"
                && !(!key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()))
            {
                return Err(invalid_cursor());
            }
            input.after = Some(KeysetCursor {
                key,
                id: id.to_string(),
            });
        }
    }
    tx::read(db, move |conn| {
        let page = match category.as_str() {
            "settings" => selectable::list_settings(conn, &input)?,
            "keywords" => selectable::list_keywords(conn, &input)?,
            c if selectable::CATEGORIES.contains(&c) => selectable::list(conn, c, &input)?,
            _ => return Ok(json!({ "items": [], "nextCursor": null, "total": 0 })),
        };
        let ids: Vec<i64> = page.rows.iter().filter_map(|r| r.id.parse().ok()).collect();
        let mut locked = std::collections::BTreeSet::new();
        for kind in snapshot::view_only_check_kinds(&category) {
            locked.extend(blocking_ids_in(
                conn,
                workspace_id,
                kind,
                &ids,
                Blocking::Edit,
            )?);
        }
        let items: Vec<Value> = page
            .rows
            .iter()
            .map(|row| {
                let mut item = json!({ "id": row.id, "name": row.name });
                if let Some(badge) = &row.badge {
                    item["badge"] = json!(badge);
                }
                if locked.contains(&row.id) {
                    item["locked"] = json!(true);
                }
                item
            })
            .collect();
        let next_cursor = match (page.has_more, page.rows.last(), page.last_key.as_ref()) {
            (true, Some(last), Some(key)) => {
                last.id.parse().ok().map(|id| encode_text_cursor(key, id))
            }
            _ => None,
        };
        let mut out = json!({ "items": items, "nextCursor": next_cursor, "total": page.total });
        if let Some(all) = page.all_ids {
            out["allIds"] = json!(all);
        }
        Ok(out)
    })
    .await
}

// ---------------------------------------------------------------------------
// Dependency preview
// ---------------------------------------------------------------------------

/// `displayNameOf` (install-preview).
pub fn display_name_of(entry: &Value) -> String {
    for key in ["name", "shortcut"] {
        if let Some(text) = entry[key].as_str().filter(|t| !t.is_empty()) {
            return text.to_string();
        }
    }
    if let Some(words) = entry["keywords"].as_array().filter(|w| !w.is_empty()) {
        return words
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
    }
    entry["text"]
        .as_str()
        .filter(|t| !t.is_empty())
        .or_else(|| entry["sourceId"].as_str())
        .unwrap_or_default()
        .to_string()
}

/// `previewSelectionDependencies`.
pub async fn preview_selection_dependencies(
    db: &Db,
    workspace_id: i64,
    selection: Value,
    actor: &Actor,
) -> Result<Value> {
    assert_can_read(actor)?;
    let picked: Vec<String> = selection
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, chosen)| {
            chosen["mode"] == "all" || chosen["ids"].as_array().is_some_and(|ids| !ids.is_empty())
        })
        .map(|(c, _)| c.clone())
        .collect();
    assert_can_read_categories(actor, &picked)?;
    let actor = actor.clone();
    tx::read(db, move |conn| {
        let tenant_id = workspace_tenant_id(conn, workspace_id)?;
        let Snapshot { payload, category_counts, dropped_reference_count } =
            snapshot::build_snapshot(conn, workspace_id, tenant_id, &selection)?;
        assert_can_read_categories(&actor, &categories_in_use(&category_counts))?;
        let mut included = Vec::new();
        for category in RESOURCE_CATEGORIES {
            let chosen = &selection[*category];
            if chosen["mode"] == "all" {
                continue;
            }
            let selected: Vec<&str> = chosen["ids"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            for entry in entries_for_category(&payload["resources"], category) {
                let source_id = entry["sourceId"].as_str().unwrap_or_default();
                if !selected.contains(&source_id) {
                    included.push(json!({ "category": category, "sourceId": source_id, "name": display_name_of(&entry) }));
                }
            }
        }
        let manifests: Vec<Value> = [
            "folders",
            "customFields",
            "tags",
            "productCategories",
            "pipelines",
            "placeCategories",
            "inboxes",
        ]
        .iter()
        .filter_map(|kind| {
            let names: Vec<Value> = payload["manifests"][*kind]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(_, e)| e["name"].clone())
                .collect();
            (!names.is_empty()).then(|| json!({ "kind": kind, "count": names.len(), "names": names }))
        })
        .collect();
        Ok(json!({
            "includedDependencies": included,
            "manifests": manifests,
            "droppedReferenceCount": dropped_reference_count,
        }))
    })
    .await
}

/// `assertCanReadTemplateCategories`: a member needs each category's permission.
fn assert_can_read_categories(actor: &Actor, categories: &[String]) -> Result<()> {
    if matches!(actor, Actor::ApiToken { .. }) {
        return Ok(());
    }
    let known: Vec<&str> = categories
        .iter()
        .map(String::as_str)
        .filter(|c| CATEGORIES.contains(c))
        .collect();
    assert_can_use_categories(actor, known)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_fall_back() {
        assert_eq!(display_name_of(&json!({ "name": "Flow" })), "Flow");
        assert_eq!(
            display_name_of(&json!({ "keywords": ["hi", "hello"] })),
            "hi, hello"
        );
        assert_eq!(display_name_of(&json!({ "sourceId": "7" })), "7");
    }
}
