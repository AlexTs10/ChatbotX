//! `dynamic-image/service.ts` (`dynamicImageService`): templates whose static
//! layers are baked into a PNG background on every save, and the per-contact
//! render of the `/dynamic-images` trigger URL. Rendering: [`render`].
//!
//! Image URLs are read through the store when they point at the public
//! storage base, else with an SSRF check before every hop (the original
//! `fetch` has no guard).

pub mod render;

use std::collections::HashSet;
use std::time::Duration;

use chatbotx_db::Db;
use chatbotx_db::repo::models::{ContactInbox, DynamicImage};
use chatbotx_db::repo::utils::{order_by_sql, parse_order_by};
use chatbotx_db::repo::{
    self, Model, PaginationInput, SortItem, find_one, get_pagination_with_defaults, like_contains,
    new_id, now_ms,
};
use chatbotx_db::rusqlite::types::Value as Sql;
use chatbotx_db::rusqlite::{OptionalExtension, params, params_from_iter};
use chatbotx_files::{Acl, PutOptions, Uploader};
use serde_json::Value;

use crate::errors::{BusinessError, Result};
use crate::tx;
use render::{Document, Element, Images};

/// The largest image a layer may read.
const MAX_IMAGE_BYTES: usize = 15 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

fn not_found() -> BusinessError {
    BusinessError::not_found("Dynamic image not found")
}

fn internal(message: impl Into<String>) -> BusinessError {
    BusinessError::Repo(repo::Error::msg(message.into()))
}

fn sql_err(error: chatbotx_db::rusqlite::Error) -> BusinessError {
    BusinessError::Repo(error.into())
}

/// `BACKGROUND_FILES_PREFIX`.
pub fn background_prefix(workspace_id: i64, id: i64) -> String {
    format!("public/space/{workspace_id}/dynamic-images/{id}/background")
}

/// `CONTACT_IMAGES_FOLDER`.
pub fn contact_images_folder(workspace_id: i64, id: i64) -> String {
    format!("public/space/{workspace_id}/dynamic-images/{id}/contacts/")
}

/// `getBackgroundVersion`: `<v>` of `background_<v>.png`.
pub fn background_version(background_url: &str) -> String {
    background_url
        .strip_suffix(".png")
        .and_then(|rest| rest.rsplit_once("background_"))
        .map(|(_, v)| v)
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
        .map_or_else(|| now_ms().to_string(), str::to_owned)
}

/// The services a save or a render needs.
#[derive(Clone, Copy)]
pub struct Deps<'a> {
    pub db: &'a Db,
    pub files: &'a Uploader,
    /// The public storage base (`<appUrl>/storage/`, with the trailing `/`).
    pub storage_url: &'a str,
}

/// `ListInput`.
#[derive(Debug, Clone, Default)]
pub struct ListInput {
    pub workspace_id: i64,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    pub name: Option<String>,
    pub sort: Option<Vec<SortItem>>,
}

/// `list` result.
#[derive(Debug, Clone)]
pub struct Page {
    pub data: Vec<DynamicImage>,
    pub page_count: i64,
}

/// `UpsertInput` (`data` = the checked document with its defaults).
#[derive(Debug, Clone)]
pub struct Upsert {
    pub workspace_id: i64,
    pub name: String,
    pub custom_field_id: Option<i64>,
    pub data: Value,
}

/// `list`.
pub async fn list(db: &Db, input: ListInput) -> Result<Page> {
    tx::read(db, move |conn| {
        let pagination = get_pagination_with_defaults(PaginationInput {
            page: input.page,
            per_page: input.per_page,
        });
        let name = input
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(like_contains);
        let where_sql = r#""workspaceId" = ?1 AND (?2 IS NULL OR ilike("name", ?2))"#;
        let order = parse_order_by(DynamicImage::COLUMNS, input.sort.as_deref(), None);
        let sql = format!(
            "{} WHERE {where_sql} {} LIMIT ?3 OFFSET ?4",
            DynamicImage::select_sql(),
            order_by_sql(&order, None),
        );
        let args = vec![
            Sql::Integer(input.workspace_id),
            name.clone().map_or(Sql::Null, Sql::Text),
            Sql::Integer(pagination.limit),
            Sql::Integer(pagination.offset),
        ];
        let mut stmt = conn.prepare(&sql).map_err(sql_err)?;
        let data = stmt
            .query_map(params_from_iter(args), DynamicImage::from_row)
            .map_err(sql_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sql_err)?;
        let total: i64 = conn
            .query_row(
                &format!(r#"SELECT count(*) FROM "DynamicImage" WHERE {where_sql}"#),
                params![input.workspace_id, name],
                |r| r.get(0),
            )
            .map_err(sql_err)?;
        let per_page = input.per_page.unwrap_or(pagination.limit).max(1);
        Ok(Page {
            data,
            page_count: (total + per_page - 1) / per_page,
        })
    })
    .await
}

/// `find` (404 `Dynamic image not found`).
pub async fn find(db: &Db, workspace_id: i64, id: i64) -> Result<DynamicImage> {
    tx::read(db, move |conn| {
        find_one::<DynamicImage>(
            conn,
            r#""id" = ?1 AND "workspaceId" = ?2"#,
            &[&id, &workspace_id],
        )?
        .ok_or_else(not_found)
    })
    .await
}

/// `findUnscoped`: by id alone (the public trigger route).
pub async fn find_unscoped(db: &Db, id: i64) -> Result<Option<DynamicImage>> {
    tx::read(db, move |conn| {
        Ok(find_one::<DynamicImage>(conn, r#""id" = ?1"#, &[&id])?)
    })
    .await
}

fn document(data: &Value) -> Result<Document> {
    serde_json::from_value(data.clone()).map_err(|e| internal(format!("Invalid document: {e}")))
}

/// `renderAndUploadBackground`: the storage key of the new background.
async fn render_and_upload_background(
    deps: Deps<'_>,
    workspace_id: i64,
    id: i64,
    data: &Value,
) -> Result<String> {
    let doc = document(data)?;
    let dynamic = render::dynamic_element_ids(&doc);
    let urls = render::image_urls(&doc, &|e: &Element| !dynamic.contains(e.id()));
    let images = load_images(deps, &urls).await?;
    let png = render::render_static_layer(&doc, &images).map_err(|e| internal(e.0))?;
    let path = format!("{}_{}.png", background_prefix(workspace_id, id), now_ms());
    deps.files
        .put_object(
            &path,
            png,
            PutOptions {
                acl: Some(Acl::PublicRead),
                content_type: Some("image/png".into()),
                cache_control: Some("public, max-age=31536000, immutable".into()),
                ..PutOptions::default()
            },
            None,
        )
        .await
        .map_err(|e| internal(e.to_string()))?;
    delete_objects_by_prefix(
        deps.files,
        &background_prefix(workspace_id, id),
        Some(&path),
    )
    .await;
    Ok(path)
}

/// `deleteObjectsByPrefix`: best effort (a failure is logged).
async fn delete_objects_by_prefix(files: &Uploader, prefix: &str, except: Option<&str>) {
    let keys = match files.list_objects(prefix).await {
        Ok(keys) => keys,
        Err(error) => {
            tracing::warn!(prefix, %error, "dynamic-image: failed to clean up old files under prefix");
            return;
        }
    };
    for key in keys.iter().filter(|k| Some(k.as_str()) != except) {
        if let Err(error) = files.delete_object(key).await {
            tracing::warn!(prefix, %error, "dynamic-image: failed to clean up old files under prefix");
        }
    }
}

/// `create`.
pub async fn create(deps: Deps<'_>, input: Upsert) -> Result<DynamicImage> {
    let id = new_id();
    let background =
        render_and_upload_background(deps, input.workspace_id, id, &input.data).await?;
    let ws = input.workspace_id;
    tx::write(deps.db, move |tx| {
        tx.execute(
            r#"INSERT INTO "DynamicImage" ("id", "workspaceId", "name", "customFieldId", "data", "backgroundUrl")
               VALUES (?1, ?2, ?3, ?4, ?5, ?6)"#,
            params![
                id,
                input.workspace_id,
                input.name,
                input.custom_field_id,
                input.data.to_string(),
                background
            ],
        )
        .map_err(sql_err)?;
        Ok(())
    })
    .await?;
    find(deps.db, ws, id).await
}

/// `update`: a new background, then the old per-contact renders are cleared.
pub async fn update(deps: Deps<'_>, id: i64, input: Upsert) -> Result<DynamicImage> {
    let ws = input.workspace_id;
    let existing = find(deps.db, ws, id).await?;
    let background = render_and_upload_background(deps, ws, id, &input.data).await?;
    tx::write(deps.db, move |tx| {
        tx.execute(
            r#"UPDATE "DynamicImage" SET "name" = ?1, "customFieldId" = ?2, "data" = ?3, "backgroundUrl" = ?4,
               "updatedAt" = ?5 WHERE "id" = ?6 AND "workspaceId" = ?7"#,
            params![
                input.name,
                input.custom_field_id,
                input.data.to_string(),
                background,
                now_ms(),
                id,
                ws
            ],
        )
        .map_err(sql_err)?;
        Ok(())
    })
    .await?;
    clear_rendered_contact_images(deps, ws, id, existing.custom_field_id).await?;
    find(deps.db, ws, id).await
}

/// `clearRenderedContactImages`.
async fn clear_rendered_contact_images(
    deps: Deps<'_>,
    workspace_id: i64,
    id: i64,
    custom_field_id: Option<i64>,
) -> Result<()> {
    delete_objects_by_prefix(deps.files, &contact_images_folder(workspace_id, id), None).await;
    if let Some(field) = custom_field_id {
        // `deleteByCustomFieldId` (no events: a bulk clear).
        tx::write(deps.db, move |tx| {
            tx.execute(
                r#"DELETE FROM "ContactCustomField" WHERE "customFieldId" = ?1
                   AND "contactId" IN (SELECT "id" FROM "Contact" WHERE "workspaceId" = ?2)"#,
                params![field, workspace_id],
            )
            .map_err(sql_err)?;
            Ok(())
        })
        .await?;
    }
    Ok(())
}

/// `setEnabled`.
pub async fn set_enabled(
    db: &Db,
    workspace_id: i64,
    id: i64,
    enabled: bool,
) -> Result<DynamicImage> {
    find(db, workspace_id, id).await?;
    tx::write(db, move |tx| {
        tx.execute(
            r#"UPDATE "DynamicImage" SET "enabled" = ?1, "updatedAt" = ?2 WHERE "id" = ?3 AND "workspaceId" = ?4"#,
            params![enabled, now_ms(), id, workspace_id],
        )
        .map_err(sql_err)?;
        Ok(())
    })
    .await?;
    find(db, workspace_id, id).await
}

/// `delete`.
pub async fn delete(deps: Deps<'_>, workspace_id: i64, id: i64) -> Result<()> {
    let existing = find(deps.db, workspace_id, id).await?;
    clear_rendered_contact_images(deps, workspace_id, id, existing.custom_field_id).await?;
    delete_objects_by_prefix(deps.files, &background_prefix(workspace_id, id), None).await;
    tx::write(deps.db, move |tx| {
        tx.execute(
            r#"DELETE FROM "DynamicImage" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            params![id, workspace_id],
        )
        .map_err(sql_err)?;
        Ok(())
    })
    .await
}

/// `toPublicStorageUrl(backgroundUrl, storageUrl)`.
pub fn background_url(row: &DynamicImage, storage_url: &str) -> Option<String> {
    row.background_url.as_deref().and_then(|key| {
        chatbotx_core::variables::helpers::public_storage_url(key, storage_url).ok()
    })
}

/// One custom field value of a contact.
async fn field_value(db: &Db, contact_id: i64, custom_field_id: i64) -> Result<Option<String>> {
    tx::read(db, move |conn| {
        conn.query_row(
            r#"SELECT "value" FROM "ContactCustomField" WHERE "contactId" = ?1 AND "customFieldId" = ?2"#,
            params![contact_id, custom_field_id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(sql_err)
    })
    .await
}

fn is_http_url(value: &str) -> bool {
    let lower = value.get(..8).unwrap_or(value).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// `findCachedUrlForContact`.
pub async fn find_cached_url_for_contact(
    db: &Db,
    row: &DynamicImage,
    contact_id: i64,
) -> Result<Option<String>> {
    let Some(field) = row.custom_field_id else {
        return Ok(None);
    };
    Ok(field_value(db, contact_id, field)
        .await?
        .filter(|v| is_http_url(v)))
}

/// `contactInboxService.findLatestBySourceId` (workspace-scoped).
pub async fn find_latest_contact_inbox(
    db: &Db,
    workspace_id: i64,
    source_id: String,
) -> Result<Option<ContactInbox>> {
    tx::read(db, move |conn| {
        Ok(repo::find_many::<ContactInbox>(
            conn,
            r#"WHERE "sourceId" = ?1 AND "inboxId" IN (SELECT "id" FROM "Inbox" WHERE "workspaceId" = ?2)
               ORDER BY "lastMessageAt" IS NULL, "lastMessageAt" DESC LIMIT 1"#,
            &[&source_id, &workspace_id],
        )?
        .into_iter()
        .next())
    })
    .await
}

/// `findContactAvatarUrl`: the stored picture as a public URL, the default
/// avatar for the no-avatar sentinel, `None` without one (or another
/// workspace's contact).
///
/// ponytail: no on-demand channel fetch (`ensureAvatarMirrored`): the
/// channel profile APIs are not ported (`profile_refresh`).
pub async fn contact_avatar_url(
    db: &Db,
    workspace_id: i64,
    contact_id: i64,
    urls: &crate::contact::avatar::TenantUrls,
) -> Result<Option<String>> {
    let avatar = tx::read(db, move |conn| {
        Ok(conn
            .query_row(
                r#"SELECT "avatar" FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                params![contact_id, workspace_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(sql_err)?
            .flatten())
    })
    .await?;
    Ok(avatar.and_then(|avatar| {
        crate::contact::avatar::resolve_contact_avatar_url(
            workspace_id,
            Some(&avatar),
            &[],
            urls,
            now_ms(),
        )
    }))
}

/// `resolveDynamicElements`: `url` of `avatarUser` / `customField` image
/// layers from the contact's data (`avatar_url` = the resolved avatar).
pub async fn resolve_dynamic_elements(
    db: &Db,
    contact_id: i64,
    avatar_url: Option<String>,
    document: &Value,
) -> Result<Value> {
    let mut document = document.clone();
    let Some(elements) = document.get_mut("elements").and_then(Value::as_array_mut) else {
        return Ok(document);
    };
    for element in elements.iter_mut() {
        if element.get("type").and_then(Value::as_str) != Some("image") {
            continue;
        }
        let url = match element.get("imageType").and_then(Value::as_str) {
            Some("avatarUser") => avatar_url.clone(),
            Some("customField") => {
                let field = element
                    .get("customFieldId")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<i64>().ok());
                match field {
                    Some(field) => match field_value(db, contact_id, field).await? {
                        Some(value)
                            if !crate::net::ssrf_guard::assert_public_url(&value, "Image URL")
                                .await
                                .is_err() =>
                        {
                            Some(value)
                        }
                        _ => None,
                    },
                    None => None,
                }
            }
            _ => None,
        };
        if let (Some(url), Some(object)) = (url, element.as_object_mut()) {
            object.insert("url".into(), Value::String(url));
        }
    }
    Ok(document)
}

/// `renderForContact`: the public URL of the contact's render (cached onto
/// the custom field when set). `dynamic` comes from the unresolved document.
pub async fn render_for_contact(
    deps: Deps<'_>,
    row: &DynamicImage,
    contact_id: i64,
    resolved: &Value,
    dynamic: &HashSet<String>,
) -> Result<String> {
    let Some(background_key) = row.background_url.as_deref() else {
        return Err(BusinessError::not_found(
            "Dynamic image has no rendered background",
        ));
    };
    let background = deps
        .files
        .get_object(background_key)
        .await
        .map_err(|e| internal(e.to_string()))?;
    let doc = document(resolved)?;
    let urls = render::image_urls(&doc, &|e: &Element| dynamic.contains(e.id()));
    let images = load_images(deps, &urls).await?;
    let png = render::render_dynamic_layer(&doc, &background, dynamic, &images)
        .map_err(|e| internal(e.0))?;
    let path = format!(
        "{}{contact_id}.png",
        contact_images_folder(row.workspace_id, row.id)
    );
    deps.files
        .put_object(
            &path,
            png,
            PutOptions {
                acl: Some(Acl::PublicRead),
                content_type: Some("image/png".into()),
                cache_control: Some("no-cache".into()),
                ..PutOptions::default()
            },
            None,
        )
        .await
        .map_err(|e| internal(e.to_string()))?;
    let base = chatbotx_core::variables::helpers::public_storage_url(&path, deps.storage_url)
        .map_err(|_| internal("Failed to build public URL for rendered dynamic image"))?;
    let url = format!("{base}?timestamp={}", background_version(background_key));
    if let Some(field) = row.custom_field_id {
        crate::contact_custom_field::set_values(
            deps.db,
            crate::contact_custom_field::SetValuesInput {
                workspace_id: row.workspace_id,
                contact_id,
                fields: vec![(field, url.clone())],
                source_timezone: None,
                source_timezone_override: None,
                source_timezone_strategy: None,
                temporal_input_parsing: None,
                fill_empty_temporal_with_now: false,
                touch_unchanged: false,
            },
        )
        .await?;
    }
    Ok(url)
}

// ---- image loading ----------------------------------------------------------------

async fn load_images(deps: Deps<'_>, urls: &[String]) -> Result<Images> {
    let mut images = Images::new();
    for url in urls {
        let bytes = load_image(deps, url)
            .await
            .map_err(|e| internal(format!("Failed to fetch image: {e}")))?;
        images.insert(url.clone(), bytes);
    }
    Ok(images)
}

/// The bytes of one layer image: a public storage URL is read from the
/// store, any other URL is fetched with an SSRF check on every hop.
async fn load_image(deps: Deps<'_>, url: &str) -> std::result::Result<Vec<u8>, String> {
    if let Some(key) = url.strip_prefix(deps.storage_url) {
        let key = key.split(['?', '#']).next().unwrap_or_default();
        if !key.is_empty() && !key.contains("..") {
            return deps
                .files
                .get_object(key)
                .await
                .map(|b| b.to_vec())
                .map_err(|e| e.to_string());
        }
    }
    fetch_public(url).await
}

fn http_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(FETCH_TIMEOUT)
            .build()
            .unwrap_or_default()
    })
}

/// `fetch(url)` following redirects by hand, the SSRF guard before each hop.
pub async fn fetch_public(url: &str) -> std::result::Result<Vec<u8>, String> {
    let mut current = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        crate::net::ssrf_guard::assert_public_url(&current, "Image URL").await?;
        let mut response = http_client()
            .get(&current)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or("Redirect without a location")?;
            current = url::Url::parse(&current)
                .and_then(|base| base.join(location))
                .map_err(|e| e.to_string())?
                .to_string();
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("{} {}", response.status().as_u16(), current));
        }
        let mut out = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            out.extend_from_slice(&chunk);
            if out.len() > MAX_IMAGE_BYTES {
                return Err("Image too large".into());
            }
        }
        return Ok(out);
    }
    Err("Too many redirects".into())
}

#[cfg(test)]
mod tests {
    #[test]
    fn background_version_is_the_save_stamp() {
        assert_eq!(
            super::background_version(
                "public/space/1/dynamic-images/2/background_1700000000000.png"
            ),
            "1700000000000"
        );
        assert!(!super::background_version("x.png").is_empty());
    }
}
