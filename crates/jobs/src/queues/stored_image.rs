//! Stored image optimization (`queues/heavy`: `isOptimizableStorageKey`,
//! `enqueueStoredImageOptimizations`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::enqueue::Enqueuer;
use crate::options::{JobOptions, NewJob};
use crate::queue::QueueName;
use crate::queues::heavy::{HeavyJobAction, heavy_job_options};

/// `optimizeStoredImage` priority (low).
pub const OPTIMIZE_STORED_IMAGE_PRIORITY: u32 = 1000;
/// Delay of each job, so a caller-owned transaction can commit first.
pub const OPTIMIZE_STORED_IMAGE_DELAY_MS: u64 = 5000;

/// One object to optimize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StoredImageJob {
    #[serde(rename_all = "camelCase")]
    Attachment {
        workspace_id: String,
        object_key: String,
        attachment_id: String,
        attachment_created_at: String,
        message_created_at: String,
    },
    #[serde(rename_all = "camelCase")]
    Avatar {
        workspace_id: String,
        object_key: String,
        contact_id: String,
    },
}

/// The heavy job payload of `optimizeStoredImage`: `{ type, data }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum OptimizeStoredImagePayload {
    #[serde(rename = "optimizeStoredImage")]
    OptimizeStoredImage(StoredImageJob),
}

/// `STORED_IMAGE_OPTIMIZATION` (`off` disables the enqueue).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredImageOptimizationConfig {
    pub enabled: bool,
}

impl StoredImageOptimizationConfig {
    /// Unset or `on` = enabled; `off` or any other (invalid) value = off.
    pub fn from_env_map(env: &BTreeMap<String, String>) -> StoredImageOptimizationConfig {
        let enabled = match env.get("STORED_IMAGE_OPTIMIZATION").map(|v| v.trim()) {
            None | Some("") | Some("on") => true,
            Some("off") => false,
            Some(other) => {
                tracing::error!(
                    value = other,
                    "Invalid STORED_IMAGE_* setting; image optimization is off"
                );
                false
            }
        };
        StoredImageOptimizationConfig { enabled }
    }
}

/// `isOptimizableStorageKey(key, workspaceId)`: an extension-less key of
/// literal `[A-Za-z0-9._-]` segments with the workspace id in a known
/// layout position.
pub fn is_optimizable_storage_key(key: Option<&str>, workspace_id: &str) -> bool {
    let Some(key) = key.filter(|k| !k.is_empty()) else {
        return false;
    };
    if has_uri_scheme(key) || key.contains('?') {
        return false;
    }
    let file_name = key.rsplit('/').next().unwrap_or("");
    !file_name.is_empty() && !file_name.contains('.') && is_workspace_storage_key(key, workspace_id)
}

/// `/^[a-z][a-z0-9+.-]*:/i`: a reference (`https:`, `wa-media:`, `failed:`), not a key.
fn has_uri_scheme(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    for c in chars {
        if c == ':' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
            return false;
        }
    }
    false
}

/// `public/space/public/ws/<ws>/...` (legacy Messenger avatars).
const LEGACY_AVATAR_PREFIX: [&str; 4] = ["public", "space", "public", "ws"];

fn is_literal_segment(segment: &str) -> bool {
    segment != "."
        && segment != ".."
        && !segment.is_empty()
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The workspace id sits where the key layouts put it: segment 2, or
/// segment 4 under [`LEGACY_AVATAR_PREFIX`].
fn is_workspace_storage_key(key: &str, workspace_id: &str) -> bool {
    if workspace_id.is_empty() {
        return false;
    }
    let segments: Vec<&str> = key.split('/').collect();
    if !segments.iter().all(|s| is_literal_segment(s)) {
        return false;
    }
    if segments.get(2) == Some(&workspace_id) {
        return true;
    }
    LEGACY_AVATAR_PREFIX
        .iter()
        .enumerate()
        .all(|(i, part)| segments.get(i) == Some(part))
        && segments.get(4) == Some(&workspace_id)
}

/// `isStoredImageOptimizationTarget`: an avatar job only rewrites an object
/// stored as an avatar (`.../avatars/<id>` or `.../avatar/<id>`).
fn is_optimization_target(job: &StoredImageJob) -> bool {
    match job {
        StoredImageJob::Attachment {
            workspace_id,
            object_key,
            ..
        } => is_workspace_storage_key(object_key, workspace_id),
        StoredImageJob::Avatar {
            workspace_id,
            object_key,
            ..
        } => {
            is_workspace_storage_key(object_key, workspace_id)
                && object_key
                    .split('/')
                    .any(|s| s == "avatars" || s == "avatar")
        }
    }
}

fn job_key(job: &StoredImageJob) -> (&str, &str) {
    match job {
        StoredImageJob::Attachment {
            workspace_id,
            object_key,
            ..
        }
        | StoredImageJob::Avatar {
            workspace_id,
            object_key,
            ..
        } => (workspace_id, object_key),
    }
}

fn stored_image_job_id(job: &StoredImageJob) -> String {
    match job {
        StoredImageJob::Attachment { attachment_id, .. } => {
            format!("optimize-image-attachment-{attachment_id}")
        }
        StoredImageJob::Avatar {
            contact_id,
            object_key,
            ..
        } => {
            let last = object_key.rsplit('/').next().unwrap_or("");
            format!("optimize-image-avatar-{contact_id}-{last}")
        }
    }
}

/// `enqueueStoredImageOptimizations(jobs)`: one `add_bulk` on `heavy` of
/// the optimizable jobs (ids `optimize-image-attachment-<id>` /
/// `optimize-image-avatar-<contactId>-<last key segment>`, priority
/// [`OPTIMIZE_STORED_IMAGE_PRIORITY`], delay [`OPTIMIZE_STORED_IMAGE_DELAY_MS`]).
/// Never fails: an enqueue error is logged.
pub async fn enqueue_stored_image_optimizations(
    enqueuer: &dyn Enqueuer,
    config: &StoredImageOptimizationConfig,
    jobs: Vec<StoredImageJob>,
) {
    if !config.enabled {
        return;
    }
    let action = HeavyJobAction::OptimizeStoredImage;
    let eligible: Vec<NewJob> = jobs
        .into_iter()
        .filter(|job| {
            let (workspace_id, key) = job_key(job);
            is_optimizable_storage_key(Some(key), workspace_id) && is_optimization_target(job)
        })
        .map(|job| {
            let opts = JobOptions {
                job_id: Some(stored_image_job_id(&job)),
                ..heavy_job_options(action)
            };
            let data = serde_json::to_value(OptimizeStoredImagePayload::OptimizeStoredImage(job))
                .unwrap_or(serde_json::Value::Null);
            NewJob {
                name: action.as_str().to_owned(),
                data,
                opts,
            }
        })
        .collect();
    if eligible.is_empty() {
        return;
    }
    let count = eligible.len();
    if let Err(err) = enqueuer.add_bulk(QueueName::Heavy, eligible).await {
        // Best effort: the image stays stored at its original size.
        tracing::warn!(err = %err, count, "Failed to enqueue stored image optimization");
    }
}
