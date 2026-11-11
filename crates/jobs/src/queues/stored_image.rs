//! Stored image optimization (`queues/heavy`: `isOptimizableStorageKey`,
//! `enqueueStoredImageOptimizations`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::enqueue::Enqueuer;

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
    pub fn from_env_map(env: &BTreeMap<String, String>) -> StoredImageOptimizationConfig {
        todo!()
    }
}

/// `isOptimizableStorageKey(key, workspaceId)`: an extension-less key of
/// literal `[A-Za-z0-9._-]` segments with the workspace id in a known
/// layout position.
pub fn is_optimizable_storage_key(key: Option<&str>, workspace_id: &str) -> bool {
    todo!()
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
    todo!()
}
