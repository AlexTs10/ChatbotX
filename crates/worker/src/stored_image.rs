//! `optimizeStoredImage` (heavy queue,
//! `packages/business/src/media/optimize-stored-image.ts`): re-encodes a
//! stored attachment or avatar in place, then updates the attachment row.

use chatbotx_core::env::ProcessEnv;
use chatbotx_core::utils::image_optimize::{
    ImageOptimizeProfile, optimize_image_for_storage, read_image_dimensions,
};
use chatbotx_core::utils::image_optimize_settings::{
    get_image_optimization_settings, is_stored_image_optimization_enabled,
};
use chatbotx_db::repo::message::{
    self as repo_message, FindAttachmentByIdParams, UpdateAttachmentFields, UpdateAttachmentParams,
};
use chatbotx_db::rusqlite::OptionalExtension as _;
use chatbotx_files::{Acl, FilesError, PutOptions};
use chatbotx_jobs::JobError;
use chatbotx_jobs::queues::stored_image::{OptimizeStoredImagePayload, StoredImageJob};
use serde_json::{Value, json};

use crate::{Deps, db_fail, fail};

const PUBLIC_STORAGE_PREFIX: &str = "public/";
const AVATAR_SEGMENTS: [&str; 2] = ["avatars", "avatar"];

/// `ReferencedRow`: `keep_original` and the stored type / size (attachments).
struct Referenced {
    keep_original: Option<bool>,
    stored: Option<(String, i64)>,
}

fn ms(iso: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .ok()
        .map(|d| d.timestamp_millis())
}

fn id(text: &str) -> Result<i64, JobError> {
    text.parse()
        .map_err(|_| JobError::unrecoverable("Invalid optimizeStoredImage data"))
}

/// `isStoredImageOptimizationTarget`.
fn is_target(job: &StoredImageJob) -> bool {
    let (key, ws) = match job {
        StoredImageJob::Attachment {
            object_key,
            workspace_id,
            ..
        }
        | StoredImageJob::Avatar {
            object_key,
            workspace_id,
            ..
        } => (object_key, workspace_id),
    };
    // `is_optimizable_storage_key` also wants an extension-less file name;
    // the workspace-key part is the same check.
    chatbotx_jobs::queues::stored_image::is_workspace_storage_key(key, ws)
        && (!matches!(job, StoredImageJob::Avatar { .. })
            || key.split('/').any(|s| AVATAR_SEGMENTS.contains(&s)))
}

async fn find_referenced(
    deps: &Deps,
    job: &StoredImageJob,
) -> Result<Option<Referenced>, JobError> {
    match job.clone() {
        StoredImageJob::Avatar {
            workspace_id,
            object_key,
            contact_id,
        } => {
            let (ws, contact) = (id(&workspace_id)?, id(&contact_id)?);
            let avatar: Option<Option<String>> = deps
                .db
                .read(move |conn| {
                    Ok(conn
                        .query_row(
                            r#"SELECT "avatar" FROM "Contact" WHERE "id" = ?1 AND "workspaceId" = ?2"#,
                            [contact, ws],
                            |r| r.get(0),
                        )
                        .optional()?)
                })
                .await
                .map_err(db_fail)?;
            Ok(
                (avatar.flatten().as_deref() == Some(object_key.as_str())).then_some(Referenced {
                    keep_original: None,
                    stored: None,
                }),
            )
        }
        StoredImageJob::Attachment {
            workspace_id,
            object_key,
            attachment_id,
            message_created_at,
            ..
        } => {
            let params = FindAttachmentByIdParams {
                id: id(&attachment_id)?,
                workspace_id: id(&workspace_id)?,
                message_created_at: ms(&message_created_at),
            };
            let row = deps
                .db
                .read(move |conn| {
                    repo_message::find_attachment_by_id(conn, &params).map_err(crate::repo_err)
                })
                .await
                .map_err(db_fail)?;
            Ok(row
                .filter(|a| a.origin_path == object_key)
                .map(|a| Referenced {
                    keep_original: a.keep_original,
                    stored: Some((a.mime_type, a.size)),
                }))
        }
    }
}

/// `updateAttachmentOrThrow`.
async fn update_attachment(
    deps: &Deps,
    job: &StoredImageJob,
    fields: UpdateAttachmentFields,
) -> Result<(), JobError> {
    let StoredImageJob::Attachment {
        workspace_id,
        attachment_id,
        attachment_created_at,
        ..
    } = job
    else {
        return Ok(());
    };
    let params = UpdateAttachmentParams {
        id: id(attachment_id)?,
        workspace_id: id(workspace_id)?,
        created_at: ms(attachment_created_at).unwrap_or_default(),
        fields,
    };
    let id = params.id;
    let updated = deps
        .db
        .write(move |tx| repo_message::update_attachment(tx, &params).map_err(crate::repo_err))
        .await
        .map_err(db_fail)?;
    if updated == 0 {
        return Err(fail(format!("Attachment {id} row was not updated")));
    }
    Ok(())
}

/// `optimizeStoredImage`: `{ outcome, bytesBefore?, bytesAfter? }`.
pub(crate) async fn optimize(deps: &Deps, data: &Value) -> Result<Value, JobError> {
    let payload = json!({ "type": "optimizeStoredImage", "data": data });
    let OptimizeStoredImagePayload::OptimizeStoredImage(job) = serde_json::from_value(payload)
        .map_err(|_| JobError::unrecoverable("Invalid optimizeStoredImage data"))?;
    let outcome = |o: &str| Ok(json!({ "outcome": o }));
    let env = ProcessEnv;
    if !is_stored_image_optimization_enabled(&env) {
        return outcome("disabled");
    }
    if !is_target(&job) {
        return outcome("skipped-foreign");
    }
    let Some(referenced) = find_referenced(deps, &job).await? else {
        return outcome("skipped-stale");
    };
    if referenced.keep_original == Some(true) {
        return outcome("skipped-original");
    }
    let Some(files) = &deps.files else {
        return Err(fail("File storage is not configured"));
    };
    let (key, avatar) = match &job {
        StoredImageJob::Attachment { object_key, .. } => (object_key.clone(), false),
        StoredImageJob::Avatar { object_key, .. } => (object_key.clone(), true),
    };
    let head = match files.head_object(&key).await {
        Ok(head) => head,
        Err(FilesError::NotFound(_)) => return outcome("skipped-stale"),
        Err(e) => return Err(fail(e.to_string())),
    };
    let settings = get_image_optimization_settings(&env).map_err(|e| fail(e.to_string()))?;
    if head.content_length > settings.job.max_source_bytes {
        return outcome("skipped-large");
    }
    if !avatar
        && head.content_length < settings.job.min_source_bytes
        && referenced
            .stored
            .as_ref()
            .is_none_or(|(_, size)| *size as u64 == head.content_length)
    {
        return outcome("skipped-small");
    }
    let source = files
        .get_object(&key)
        .await
        .map_err(|e| fail(e.to_string()))?;
    let bytes_before = source.len();
    let content_type = head
        .content_type
        .clone()
        .unwrap_or_else(|| "application/octet-stream".into());
    let profile = if avatar {
        ImageOptimizeProfile::Avatar
    } else {
        ImageOptimizeProfile::Media
    };
    let result = optimize_image_for_storage(source.clone(), &content_type, profile, &env).await;
    let sizes = |after: usize| json!({ "bytesBefore": bytes_before, "bytesAfter": after });
    if !result.optimized {
        // A previous attempt may have overwritten the object and failed to
        // update the row: repair the row from the stored object.
        let stored_type = head
            .content_type
            .clone()
            .or_else(|| referenced.stored.as_ref().map(|(t, _)| t.clone()));
        if let (Some((mime, size)), Some(stored_type)) = (&referenced.stored, stored_type)
            && (*mime != stored_type || *size as usize != bytes_before)
        {
            let dims = read_image_dimensions(&source).filter(|(w, h)| *w > 0 && *h > 0);
            update_attachment(
                deps,
                &job,
                UpdateAttachmentFields {
                    origin_path: key,
                    mime_type: stored_type,
                    size: bytes_before as i64,
                    width: dims.map(|(w, _)| w.into()),
                    height: dims.map(|(_, h)| h.into()),
                },
            )
            .await?;
            let mut out = sizes(bytes_before);
            out["outcome"] = json!("reconciled");
            return Ok(out);
        }
        let mut out = sizes(bytes_before);
        out["outcome"] = json!("unchanged");
        return Ok(out);
    }
    // The encode takes time: never write over a key nothing points at.
    if find_referenced(deps, &job).await?.is_none() {
        return outcome("skipped-stale");
    }
    let after = result.bytes.len();
    files
        .put_object(
            &key,
            result.bytes,
            PutOptions {
                acl: key
                    .starts_with(PUBLIC_STORAGE_PREFIX)
                    .then_some(Acl::PublicRead),
                content_type: Some(result.mime_type.clone()),
                content_length: Some(after as u64),
                ..PutOptions::default()
            },
            None,
        )
        .await
        .map_err(|e| fail(e.to_string()))?;
    update_attachment(
        deps,
        &job,
        UpdateAttachmentFields {
            origin_path: key,
            mime_type: result.mime_type,
            size: after as i64,
            width: result.width.map(Into::into),
            height: result.height.map(Into::into),
        },
    )
    .await?;
    let mut out = sizes(after);
    out["outcome"] = json!("optimized");
    Ok(out)
}
