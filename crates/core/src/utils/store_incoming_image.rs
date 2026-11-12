//! `packages/utils/src/store-incoming-image.ts`: writes an image a channel
//! delivered, encoding it inline when that is cheap and safe (small input,
//! a free encode slot); everything else is stored untouched for the job.

use std::future::Future;

use bytes::Bytes;

use super::image_optimize::{
    ImageOptimizeProfile, optimize_image_for_storage, read_image_dimensions,
};
use super::image_optimize_settings::{
    EncodeSlots, InlineSettings, get_image_optimization_settings,
    is_stored_image_optimization_enabled,
};
use crate::env::Env;

/// `putObject` options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PutObjectOptions {
    /// `Some("public-read")` or none.
    pub acl: Option<&'static str>,
    pub content_length: usize,
    pub content_type: String,
}

/// `IncomingImageUploader`.
pub trait IncomingImageUploader {
    fn put_object(
        &self,
        path: &str,
        body: Bytes,
        options: PutObjectOptions,
    ) -> impl Future<Output = Result<(), String>>;
}

string_enum! {
    /// `StoredIncomingImageSkip`.
    pub enum StoredIncomingImageSkip {
        Disabled = "disabled",
        Profile = "profile",
        Mime = "mime",
        Bytes = "bytes",
        Pixels = "pixels",
        Busy = "busy",
        Unchanged = "unchanged",
        Undecodable = "undecodable",
    }
}

/// `StoredIncomingImage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredIncomingImage {
    pub origin_path: String,
    pub mime_type: String,
    pub size: usize,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub optimized: bool,
    pub skipped: Option<StoredIncomingImageSkip>,
}

/// `StoredMediaFields`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMediaFields {
    pub origin_path: String,
    pub mime_type: String,
    pub size: usize,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub optimized: Option<bool>,
}

/// `toStoredMediaFields`.
pub fn to_stored_media_fields(stored: &StoredIncomingImage) -> StoredMediaFields {
    let both = matches!((stored.width, stored.height), (Some(w), Some(h)) if w > 0 && h > 0);
    StoredMediaFields {
        origin_path: stored.origin_path.clone(),
        mime_type: stored.mime_type.clone(),
        size: stored.size,
        width: if both { stored.width } else { None },
        height: if both { stored.height } else { None },
        optimized: stored.optimized.then_some(true),
    }
}

/// `StoreIncomingImageInput`.
#[derive(Debug, Clone)]
pub struct StoreIncomingImageInput<'a, U> {
    pub uploader: Option<&'a U>,
    pub key: &'a str,
    pub bytes: Bytes,
    pub mime_type: &'a str,
    pub profile: ImageOptimizeProfile,
    /// `Some("public-read")`.
    pub acl: Option<&'static str>,
}

fn read_inline_limits(env: &dyn Env) -> Option<InlineSettings> {
    match get_image_optimization_settings(env) {
        Ok(settings) => Some(settings.inline),
        Err(err) => {
            tracing::error!(
                module = "store-incoming-image",
                err = %err,
                "Invalid STORED_IMAGE_* setting; storing images without inline encoding"
            );
            None
        }
    }
}

fn gate_reason(
    env: &dyn Env,
    profile: ImageOptimizeProfile,
    size: usize,
    pixels: Option<u64>,
    limits: &InlineSettings,
) -> Option<StoredIncomingImageSkip> {
    if !is_stored_image_optimization_enabled(env) || limits.max_concurrent_encodes == 0 {
        return Some(StoredIncomingImageSkip::Disabled);
    }
    if profile == ImageOptimizeProfile::Original {
        return Some(StoredIncomingImageSkip::Profile);
    }
    if size as u64 > limits.max_bytes {
        return Some(StoredIncomingImageSkip::Bytes);
    }
    let Some(pixels) = pixels else {
        return Some(StoredIncomingImageSkip::Undecodable);
    };
    let max = if profile == ImageOptimizeProfile::Avatar {
        limits.max_pixels.avatar
    } else {
        limits.max_pixels.media
    };
    (pixels > max).then_some(StoredIncomingImageSkip::Pixels)
}

async fn write<U: IncomingImageUploader>(
    uploader: Option<&U>,
    key: &str,
    acl: Option<&'static str>,
    bytes: Bytes,
    mime_type: &str,
    extra: (bool, Option<StoredIncomingImageSkip>, Option<(u32, u32)>),
) -> Result<StoredIncomingImage, String> {
    let size = bytes.len();
    if let Some(uploader) = uploader {
        uploader
            .put_object(
                key,
                bytes,
                PutObjectOptions {
                    acl,
                    content_length: size,
                    content_type: mime_type.to_owned(),
                },
            )
            .await?;
    }
    let (optimized, skipped, dims) = extra;
    Ok(StoredIncomingImage {
        origin_path: key.to_owned(),
        mime_type: mime_type.to_owned(),
        size,
        width: dims.map(|d| d.0),
        height: dims.map(|d| d.1),
        optimized,
        skipped,
    })
}

async fn encode_and_store<U: IncomingImageUploader>(
    input: StoreIncomingImageInput<'_, U>,
    env: &(dyn Env + Sync),
    slots: &EncodeSlots,
) -> Result<StoredIncomingImage, String> {
    let is_image = input.mime_type.to_lowercase().starts_with("image/");
    if !is_image {
        return write(
            input.uploader,
            input.key,
            input.acl,
            input.bytes,
            input.mime_type,
            (false, Some(StoredIncomingImageSkip::Mime), None),
        )
        .await;
    }
    let dims = read_image_dimensions(&input.bytes).filter(|(w, h)| *w > 0 && *h > 0);
    let pixels = dims.map(|(w, h)| u64::from(w) * u64::from(h));
    let unchanged_with = |reason| (false, Some(reason), dims);
    let Some(limits) = read_inline_limits(env) else {
        return write(
            input.uploader,
            input.key,
            input.acl,
            input.bytes,
            input.mime_type,
            unchanged_with(StoredIncomingImageSkip::Disabled),
        )
        .await;
    };
    if let Some(reason) = gate_reason(env, input.profile, input.bytes.len(), pixels, &limits) {
        return write(
            input.uploader,
            input.key,
            input.acl,
            input.bytes,
            input.mime_type,
            unchanged_with(reason),
        )
        .await;
    }
    let source = input.bytes.clone();
    let mime_type = input.mime_type;
    let profile = input.profile;
    let slot = slots
        .run_in_encode_slot(limits.max_concurrent_encodes, false, || async move {
            Ok::<_, String>(optimize_image_for_storage(source, mime_type, profile, env).await)
        })
        .await?;
    let Some(encoded) = slot else {
        return write(
            input.uploader,
            input.key,
            input.acl,
            input.bytes,
            input.mime_type,
            unchanged_with(StoredIncomingImageSkip::Busy),
        )
        .await;
    };
    if !encoded.optimized {
        return write(
            input.uploader,
            input.key,
            input.acl,
            input.bytes,
            input.mime_type,
            unchanged_with(StoredIncomingImageSkip::Unchanged),
        )
        .await;
    }
    let key = format!("{}.{}", input.key, encoded.extension.unwrap_or("bin"));
    let dims = encoded.width.zip(encoded.height);
    write(
        input.uploader,
        &key,
        input.acl,
        encoded.bytes,
        &encoded.mime_type,
        (true, None, dims),
    )
    .await
}

/// `storeIncomingImage`: writes the image once (encoded inline when the
/// gates allow) and logs the outcome for image MIME types.
pub async fn store_incoming_image<U: IncomingImageUploader>(
    input: StoreIncomingImageInput<'_, U>,
    env: &(dyn Env + Sync),
    slots: &EncodeSlots,
) -> Result<StoredIncomingImage, String> {
    let started = std::time::Instant::now();
    let bytes_before = input.bytes.len();
    let profile = input.profile;
    let is_image = input.mime_type.to_lowercase().starts_with("image/");
    let result = encode_and_store(input, env, slots).await?;
    if is_image {
        tracing::info!(
            module = "store-incoming-image",
            profile = profile.as_str(),
            optimized = result.optimized,
            skipped = result.skipped.map(|s| s.as_str()),
            bytes_before = bytes_before,
            bytes_after = result.size,
            duration_ms = started.elapsed().as_millis() as u64,
            "Stored incoming image"
        );
    }
    Ok(result)
}
