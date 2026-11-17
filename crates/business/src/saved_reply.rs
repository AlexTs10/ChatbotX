//! `packages/business/src/saved-reply/service.ts`: shortcut replies with
//! ordered media-library images. No template edit locks / delete guards
//! (P3.6-D1 rule).

use std::collections::HashSet;

use chatbotx_core::utils::inbox_image_support::{SAVED_REPLY_MAX_IMAGES, is_image_mime_type};
use chatbotx_core::utils::storage::get_public_file_url;
use chatbotx_db::Db;
use chatbotx_db::repo::models::SavedReply;
use chatbotx_db::repo::{self, media_library_file, new_id, now_ms, small_repos};
use chatbotx_db::rusqlite::Connection;
use serde_json::{Value, json};

use crate::errors::{BusinessError, Result};
use crate::tx;

const NOT_FOUND: &str = "Saved reply not found";

/// `SavedReplyImage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub id: i64,
    pub media_library_file_id: i64,
    pub name: String,
    pub mime_type: String,
    pub size: i64,
    pub url: String,
}

/// `SavedReplyWithImages`.
#[derive(Debug, Clone)]
pub struct WithImages {
    pub reply: SavedReply,
    pub images: Vec<Image>,
}

impl WithImages {
    /// `savedReplyResource`.
    pub fn to_json(&self) -> Value {
        let mut out = self.reply.to_json();
        out["images"] = self
            .images
            .iter()
            .map(|image| {
                json!({
                    "id": image.id.to_string(),
                    "mediaLibraryFileId": image.media_library_file_id.to_string(),
                    "name": image.name,
                    "mimeType": image.mime_type,
                    "size": image.size,
                    "url": image.url,
                })
            })
            .collect();
        out
    }
}

/// `savedReplyService.update` data: `None` keeps the stored value.
#[derive(Debug, Clone, Default)]
pub struct Update {
    pub shortcut: String,
    pub text: Option<String>,
    pub image_file_ids: Option<Vec<i64>>,
}

fn validation(field: &str, message: &str, data: Option<Value>) -> BusinessError {
    let mut error = BusinessError::validation(field, message);
    if let BusinessError::Exception { data: slot, .. } = &mut error {
        *slot = data;
    }
    error
}

fn assert_text_or_images(text: &str, image_count: usize) -> Result<()> {
    if text.is_empty() && image_count == 0 {
        return Err(validation(
            "text",
            "A saved reply needs text or at least one image",
            None,
        ));
    }
    Ok(())
}

/// Every id is an image file of the media library of this workspace.
fn assert_image_files(conn: &Connection, workspace_id: i64, ids: &[i64]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    if ids.len() > SAVED_REPLY_MAX_IMAGES {
        return Err(validation(
            "imageFileIds",
            &format!("A saved reply can carry at most {SAVED_REPLY_MAX_IMAGES} images"),
            Some(json!({ "max": SAVED_REPLY_MAX_IMAGES })),
        ));
    }
    if ids.iter().collect::<HashSet<_>>().len() != ids.len() {
        return Err(validation(
            "imageFileIds",
            "The same image is listed more than once",
            None,
        ));
    }
    let files = media_library_file::find_by_ids(conn, workspace_id, ids)?;
    let found: HashSet<i64> = files.iter().map(|file| file.id).collect();
    if let Some(missing) = ids.iter().find(|id| !found.contains(id)) {
        return Err(BusinessError::not_found(format!(
            "MediaLibraryFile {missing} not found"
        )));
    }
    if let Some(file) = files.iter().find(|f| !is_image_mime_type(&f.mime_type)) {
        return Err(validation(
            "imageFileIds",
            "Only images are allowed",
            Some(json!({ "fileName": file.name })),
        ));
    }
    Ok(())
}

fn find_in(conn: &Connection, workspace_id: i64, id: i64) -> Result<SavedReply> {
    repo::find_one::<SavedReply>(
        conn,
        r#""id" = ?1 AND "workspaceId" = ?2"#,
        &[&id, &workspace_id],
    )?
    .ok_or_else(|| BusinessError::not_found(NOT_FOUND))
}

fn attach_images(
    conn: &Connection,
    storage_url: &str,
    workspace_id: i64,
    replies: Vec<SavedReply>,
) -> Result<Vec<WithImages>> {
    let ids: Vec<i64> = replies.iter().map(|r| r.id).collect();
    let images = small_repos::saved_reply_image_list_by_saved_reply_ids(conn, workspace_id, &ids)?;
    Ok(replies
        .into_iter()
        .map(|reply| {
            let images = images
                .iter()
                .filter(|image| image.saved_reply_id == reply.id)
                .map(|image| Image {
                    id: image.id,
                    media_library_file_id: image.media_library_file_id,
                    name: image.file_name.clone(),
                    mime_type: image.file_mime_type.clone(),
                    size: image.file_size,
                    url: get_public_file_url(&image.file_path, storage_url)
                        .unwrap_or_else(|_| format!("{storage_url}{}", image.file_path)),
                })
                .collect();
            WithImages { reply, images }
        })
        .collect())
}

async fn with_images(
    db: &Db,
    storage_url: &str,
    workspace_id: i64,
    reply: SavedReply,
) -> Result<WithImages> {
    let storage_url = storage_url.to_string();
    let mut out = tx::read(db, move |c| {
        attach_images(c, &storage_url, workspace_id, vec![reply])
    })
    .await?;
    Ok(out.remove(0))
}

/// `savedReplyService.create`.
pub async fn create(
    db: &Db,
    storage_url: &str,
    workspace_id: i64,
    shortcut: String,
    text: Option<String>,
    image_file_ids: Vec<i64>,
) -> Result<WithImages> {
    let text = text.map(|t| t.trim().to_string()).unwrap_or_default();
    assert_text_or_images(&text, image_file_ids.len())?;
    let reply = tx::write(db, move |c| {
        assert_image_files(c, workspace_id, &image_file_ids)?;
        let (id, now) = (new_id(), now_ms());
        c.execute(
            r#"INSERT INTO "SavedReply" ("id", "createdAt", "updatedAt", "shortcut", "text", "workspaceId")
               VALUES (?1, ?2, ?2, ?3, ?4, ?5)"#,
            chatbotx_db::rusqlite::params![id, now, shortcut, text, workspace_id],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        small_repos::saved_reply_image_replace_for_saved_reply(c, workspace_id, id, &image_file_ids)?;
        find_in(c, workspace_id, id)
    })
    .await?;
    with_images(db, storage_url, workspace_id, reply).await
}

/// `savedReplyService.findByIdOrFail`.
pub async fn find_by_id_or_fail(
    db: &Db,
    storage_url: &str,
    workspace_id: i64,
    id: i64,
) -> Result<WithImages> {
    let storage_url = storage_url.to_string();
    let mut out = tx::read(db, move |c| {
        let reply = find_in(c, workspace_id, id)?;
        attach_images(c, &storage_url, workspace_id, vec![reply])
    })
    .await?;
    Ok(out.remove(0))
}

/// `savedReplyService.update`.
pub async fn update(
    db: &Db,
    storage_url: &str,
    workspace_id: i64,
    id: i64,
    data: Update,
) -> Result<WithImages> {
    let reply = tx::write(db, move |c| {
        if let Some(ids) = &data.image_file_ids {
            assert_image_files(c, workspace_id, ids)?;
        }
        let reply = find_in(c, workspace_id, id)?;
        let text = data
            .text
            .as_deref()
            .map_or_else(|| reply.text.clone(), |t| t.trim().to_string());
        let image_count = match &data.image_file_ids {
            Some(ids) => ids.len(),
            None => usize::try_from(small_repos::saved_reply_image_count_by_saved_reply_id(
                c,
                workspace_id,
                reply.id,
            )?)
            .unwrap_or(0),
        };
        assert_text_or_images(&text, image_count)?;
        c.execute(
            r#"UPDATE "SavedReply" SET "shortcut" = ?2, "text" = ?3, "updatedAt" = ?4 WHERE "id" = ?1"#,
            chatbotx_db::rusqlite::params![reply.id, data.shortcut, text, now_ms()],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        if let Some(ids) = &data.image_file_ids {
            small_repos::saved_reply_image_replace_for_saved_reply(c, workspace_id, reply.id, ids)?;
        }
        find_in(c, workspace_id, reply.id)
    })
    .await?;
    with_images(db, storage_url, workspace_id, reply).await
}

/// `savedReplyService.delete` (the image links cascade).
pub async fn delete(db: &Db, workspace_id: i64, id: i64) -> Result<()> {
    tx::write(db, move |c| {
        find_in(c, workspace_id, id)?;
        c.execute(
            r#"DELETE FROM "SavedReply" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
            chatbotx_db::rusqlite::params![id, workspace_id],
        )
        .map_err(|e| BusinessError::Repo(e.into()))?;
        Ok(())
    })
    .await
}

/// `savedReplyService.listByWorkspaceId` (oldest first).
pub async fn list_by_workspace_id(
    db: &Db,
    storage_url: &str,
    workspace_id: i64,
) -> Result<Vec<WithImages>> {
    let storage_url = storage_url.to_string();
    tx::read(db, move |c| {
        let replies = repo::find_many::<SavedReply>(
            c,
            r#"WHERE "workspaceId" = ?1 ORDER BY "createdAt" ASC, "id" ASC"#,
            &[&workspace_id],
        )?;
        attach_images(c, &storage_url, workspace_id, replies)
    })
    .await
}
