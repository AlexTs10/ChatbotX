//! Port of `packages/utils/__tests__/store-incoming-image.test.ts`. The
//! uploader mock records its calls; the logger mock is a `tracing`
//! recorder (fields in snake_case); each test has its own `EncodeSlots` and
//! an env map with `stubGenerousEncodeTimeout` (60 s).

#[path = "support/images.rs"]
mod images;
mod support;

use std::cell::RefCell;

use bytes::Bytes;
use chatbotx_core::utils::image_optimize::ImageOptimizeProfile;
use chatbotx_core::utils::image_optimize_settings::{EncodeSlots, get_image_optimization_settings};
use chatbotx_core::utils::store_incoming_image::{
    IncomingImageUploader, PutObjectOptions, StoreIncomingImageInput, StoredIncomingImage,
    StoredIncomingImageSkip, store_incoming_image,
};
use support::RecordingLog;
use tracing::Level;

#[derive(Default)]
struct Uploader {
    calls: RefCell<Vec<(String, Bytes, PutObjectOptions)>>,
}

impl IncomingImageUploader for Uploader {
    async fn put_object(
        &self,
        path: &str,
        body: Bytes,
        options: PutObjectOptions,
    ) -> Result<(), String> {
        self.calls
            .borrow_mut()
            .push((path.to_owned(), body, options));
        Ok(())
    }
}

fn env(extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut env = vec![(
        "STORED_IMAGE_ENCODE_TIMEOUT_SECONDS".to_owned(),
        "60".to_owned(),
    )];
    env.extend(
        extra
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    );
    env
}

fn inline_max_bytes() -> usize {
    get_image_optimization_settings(&env(&[]))
        .unwrap()
        .inline
        .max_bytes as usize
}

fn inline_max_concurrent_encodes() -> usize {
    get_image_optimization_settings(&env(&[]))
        .unwrap()
        .inline
        .max_concurrent_encodes
}

fn noisy_jpeg(width: u32, height: u32, quality: u8) -> Bytes {
    Bytes::from(images::noisy_jpeg(width, height, quality))
}

async fn store(
    uploader: Option<&Uploader>,
    key: &str,
    bytes: Bytes,
    mime_type: &str,
    profile: ImageOptimizeProfile,
    acl: Option<&'static str>,
    extra_env: &[(&str, &str)],
    slots: &EncodeSlots,
) -> StoredIncomingImage {
    let env = env(extra_env);
    store_incoming_image(
        StoreIncomingImageInput {
            uploader,
            key,
            bytes,
            mime_type,
            profile,
            acl,
        },
        &env,
        slots,
    )
    .await
    .unwrap()
}

fn same(a: &Bytes, b: &Bytes) -> bool {
    a.as_ptr() == b.as_ptr() && a.len() == b.len()
}

use ImageOptimizeProfile::{Avatar, Media, Original};

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:50 "storeIncomingImage > encodes a small photo inline and writes it once"
#[tokio::test]
async fn storeincomingimage_encodes_a_small_photo_inline_and_writes_it_once() {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let source = noisy_jpeg(1200, 900, 95);
    let result = store(
        Some(&uploader),
        "ws/a",
        source.clone(),
        "image/jpeg",
        Media,
        None,
        &[],
        &slots,
    )
    .await;

    assert!(source.len() < inline_max_bytes());
    assert!(result.optimized);
    assert_eq!(result.origin_path, "ws/a.jpg");
    let calls = uploader.calls.borrow();
    assert_eq!(calls[0].0, "ws/a.jpg");
    assert_eq!(result.mime_type, "image/jpeg");
    assert!(result.size < source.len());
    assert_eq!(result.width, Some(1200));
    assert_eq!(result.height, Some(900));
    assert_eq!(calls.len(), 1);
    let (_, body, options) = &calls[0];
    assert_eq!(body.len(), result.size);
    assert_eq!(options.content_length, result.size);
    assert_eq!(options.content_type, "image/jpeg");
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:83 "storeIncomingImage > names an encoded avatar's format in its key, and leaves an unencoded image's key as given"
#[tokio::test]
async fn storeincomingimage_names_an_encoded_avatar_s_format_in_its_key_and_leaves_an_unencoded_image_s_key_as_given()
 {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let avatar = store(
        Some(&uploader),
        "ws/avatars/a",
        noisy_jpeg(480, 480, 95),
        "image/jpeg",
        Avatar,
        None,
        &[],
        &slots,
    )
    .await;
    let original = store(
        Some(&uploader),
        "ws/doc",
        noisy_jpeg(480, 480, 95),
        "image/jpeg",
        Original,
        None,
        &[],
        &slots,
    )
    .await;

    assert_eq!(avatar.origin_path, "ws/avatars/a.webp");
    assert_eq!(original.origin_path, "ws/doc");
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:104 "storeIncomingImage > logs why an image was or was not encoded, with the bytes saved"
#[tokio::test]
async fn storeincomingimage_logs_why_an_image_was_or_was_not_encoded_with_the_bytes_saved() {
    let log = RecordingLog::install();
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let source = noisy_jpeg(1200, 900, 95);
    let result = store(
        Some(&uploader),
        "ws/log",
        source.clone(),
        "image/jpeg",
        Media,
        None,
        &[],
        &slots,
    )
    .await;

    let entry = log
        .at(Level::INFO)
        .into_iter()
        .find(|e| e.field("bytes_before").is_some())
        .expect("an info line with the bytes");
    assert_eq!(entry.field("profile"), Some("media"));
    assert_eq!(entry.field("optimized"), Some("true"));
    assert_eq!(
        entry.field("bytes_before"),
        Some(source.len().to_string().as_str())
    );
    assert_eq!(
        entry.field("bytes_after"),
        Some(result.size.to_string().as_str())
    );
    assert!(!entry.message.is_empty());
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:127 "storeIncomingImage > passes the acl through to the write"
#[tokio::test]
async fn storeincomingimage_passes_the_acl_through_to_the_write() {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    store(
        Some(&uploader),
        "ws/a",
        noisy_jpeg(800, 600, 95),
        "image/jpeg",
        Media,
        Some("public-read"),
        &[],
        &slots,
    )
    .await;

    assert_eq!(uploader.calls.borrow()[0].2.acl, Some("public-read"));
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:142 "storeIncomingImage > stores the original, with its dimensions, when it is over the byte gate"
#[tokio::test]
async fn storeincomingimage_stores_the_original_with_its_dimensions_when_it_is_over_the_byte_gate()
{
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let source = noisy_jpeg(2000, 1500, 100);
    assert!(source.len() > inline_max_bytes());

    let result = store(
        Some(&uploader),
        "ws/big",
        source.clone(),
        "image/jpeg",
        Media,
        None,
        &[],
        &slots,
    )
    .await;

    assert!(!result.optimized);
    assert_eq!(result.skipped, Some(StoredIncomingImageSkip::Bytes));
    assert_eq!(result.size, source.len());
    assert_eq!(result.width, Some(2000));
    assert_eq!(result.height, Some(1500));
    assert_eq!(result.mime_type, "image/jpeg");
    assert!(same(&uploader.calls.borrow()[0].1, &source));
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:166 "storeIncomingImage > stores the original when it is over the pixel gate"
#[tokio::test]
async fn storeincomingimage_stores_the_original_when_it_is_over_the_pixel_gate() {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let source = Bytes::from(images::flat_jpeg(3200, 2600, [0x33, 0x66, 0x99]));
    let result = store(
        Some(&uploader),
        "ws/mp",
        source.clone(),
        "image/jpeg",
        Media,
        None,
        &[],
        &slots,
    )
    .await;

    assert!(source.len() < inline_max_bytes());
    assert!(!result.optimized);
    assert_eq!(result.skipped, Some(StoredIncomingImageSkip::Pixels));
    assert_eq!(result.width, Some(3200));
    assert_eq!(result.height, Some(2600));
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:197 "storeIncomingImage > applies the tighter pixel gate to avatars"
#[tokio::test]
async fn storeincomingimage_applies_the_tighter_pixel_gate_to_avatars() {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let source = Bytes::from(images::flat_jpeg(2200, 2000, [0x33, 0x66, 0x99]));
    let result = store(
        Some(&uploader),
        "ws/avatar",
        source,
        "image/jpeg",
        Avatar,
        None,
        &[],
        &slots,
    )
    .await;

    assert!(!result.optimized);
    assert_eq!(result.skipped, Some(StoredIncomingImageSkip::Pixels));
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:221 "storeIncomingImage > never encodes more than the concurrency limit at once"
#[tokio::test]
async fn storeincomingimage_never_encodes_more_than_the_concurrency_limit_at_once() {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let source = noisy_jpeg(1200, 900, 95);
    let keys: Vec<String> = (0..inline_max_concurrent_encodes() + 3)
        .map(|index| format!("ws/{index}"))
        .collect();
    let results = futures::future::join_all(keys.iter().map(|key| {
        store(
            Some(&uploader),
            key,
            source.clone(),
            "image/jpeg",
            Media,
            None,
            &[],
            &slots,
        )
    }))
    .await;

    let busy: Vec<&StoredIncomingImage> = results
        .iter()
        .filter(|r| r.skipped == Some(StoredIncomingImageSkip::Busy))
        .collect();
    let optimized = results.iter().filter(|r| r.optimized).count();
    assert!(!busy.is_empty());
    assert!(optimized <= inline_max_concurrent_encodes());
    for result in busy {
        assert_eq!(result.size, source.len());
    }
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:246 "storeIncomingImage > releases the encode slot after a failure so later images still encode"
#[tokio::test]
async fn storeincomingimage_releases_the_encode_slot_after_a_failure_so_later_images_still_encode()
{
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let garbage = Bytes::from_static(b"not an image at all");
    for attempt in 0..inline_max_concurrent_encodes() + 2 {
        store(
            Some(&uploader),
            &format!("ws/bad-{attempt}"),
            garbage.clone(),
            "image/jpeg",
            Media,
            None,
            &[],
            &slots,
        )
        .await;
    }

    let result = store(
        Some(&uploader),
        "ws/good",
        noisy_jpeg(1200, 900, 95),
        "image/jpeg",
        Media,
        None,
        &[],
        &slots,
    )
    .await;
    assert!(result.optimized);
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:274 "storeIncomingImage > stores an undecodable image as-is"
#[tokio::test]
async fn storeincomingimage_stores_an_undecodable_image_as_is() {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let garbage = Bytes::from_static(b"not an image at all");
    let result = store(
        Some(&uploader),
        "ws/bad",
        garbage.clone(),
        "image/jpeg",
        Media,
        None,
        &[],
        &slots,
    )
    .await;

    assert!(!result.optimized);
    assert_eq!(result.size, garbage.len());
    assert_eq!(result.mime_type, "image/jpeg");
    assert_eq!(result.width, None);
    assert!(same(&uploader.calls.borrow()[0].1, &garbage));
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:295 "storeIncomingImage > keeps the original for the original profile"
#[tokio::test]
async fn storeincomingimage_keeps_the_original_for_the_original_profile() {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let source = noisy_jpeg(1200, 900, 95);
    let result = store(
        Some(&uploader),
        "ws/doc",
        source.clone(),
        "image/jpeg",
        Original,
        None,
        &[],
        &slots,
    )
    .await;

    assert!(!result.optimized);
    assert_eq!(result.skipped, Some(StoredIncomingImageSkip::Profile));
    assert_eq!(result.size, source.len());
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:314 "storeIncomingImage > keeps non-image bytes untouched"
#[tokio::test]
async fn storeincomingimage_keeps_non_image_bytes_untouched() {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let pdf = Bytes::from_static(b"%PDF-1.4");
    let result = store(
        Some(&uploader),
        "ws/doc",
        pdf.clone(),
        "application/pdf",
        Media,
        None,
        &[],
        &slots,
    )
    .await;

    assert!(!result.optimized);
    assert_eq!(result.skipped, Some(StoredIncomingImageSkip::Mime));
    assert_eq!(result.mime_type, "application/pdf");
    assert_eq!(result.size, pdf.len());
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:334 "storeIncomingImage > uses the configured byte gate"
#[tokio::test]
async fn storeincomingimage_uses_the_configured_byte_gate() {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let result = store(
        Some(&uploader),
        "ws/gate",
        noisy_jpeg(800, 600, 95),
        "image/jpeg",
        Media,
        None,
        &[("STORED_IMAGE_INLINE_MAX_MB", "0.01")],
        &slots,
    )
    .await;

    assert!(!result.optimized);
    assert_eq!(result.skipped, Some(StoredIncomingImageSkip::Bytes));
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:349 "storeIncomingImage > never encodes inline when its concurrency is 0, leaving the image to the job"
#[tokio::test]
async fn storeincomingimage_never_encodes_inline_when_its_concurrency_is_0_leaving_the_image_to_the_job()
 {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let source = noisy_jpeg(800, 600, 95);
    let result = store(
        Some(&uploader),
        "ws/no-inline",
        source.clone(),
        "image/jpeg",
        Media,
        None,
        &[("STORED_IMAGE_INLINE_CONCURRENCY", "0")],
        &slots,
    )
    .await;

    assert!(!result.optimized);
    assert_eq!(result.skipped, Some(StoredIncomingImageSkip::Disabled));
    assert_eq!(result.size, source.len());
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:369 "storeIncomingImage > still stores the image, untouched, when the tuning is misconfigured"
#[tokio::test]
async fn storeincomingimage_still_stores_the_image_untouched_when_the_tuning_is_misconfigured() {
    let log = RecordingLog::install();
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let source = noisy_jpeg(800, 600, 95);
    let result = store(
        Some(&uploader),
        "ws/bad-config",
        source.clone(),
        "image/jpeg",
        Media,
        None,
        &[("STORED_IMAGE_INLINE_MAX_MB", "not-a-number")],
        &slots,
    )
    .await;

    assert!(!result.optimized);
    assert_eq!(result.size, source.len());
    assert!(same(&uploader.calls.borrow()[0].1, &source));
    let errors = log.at(Level::ERROR);
    assert!(
        errors
            .iter()
            .any(|e| e.field("err").is_some() && !e.message.is_empty())
    );
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:390 "storeIncomingImage > stores the original when the switch is off"
#[tokio::test]
async fn storeincomingimage_stores_the_original_when_the_switch_is_off() {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let source = noisy_jpeg(1200, 900, 95);
    let result = store(
        Some(&uploader),
        "ws/off",
        source.clone(),
        "image/jpeg",
        Media,
        None,
        &[("STORED_IMAGE_OPTIMIZATION", "off")],
        &slots,
    )
    .await;

    assert!(!result.optimized);
    assert_eq!(result.skipped, Some(StoredIncomingImageSkip::Disabled));
    assert_eq!(result.size, source.len());
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:407 "storeIncomingImage > stores the original when the encode saves too little"
#[tokio::test]
async fn storeincomingimage_stores_the_original_when_the_encode_saves_too_little() {
    let uploader = Uploader::default();
    let slots = EncodeSlots::new();
    let source = noisy_jpeg(1200, 900, 50);
    let result = store(
        Some(&uploader),
        "ws/ratio",
        source.clone(),
        "image/jpeg",
        Media,
        None,
        &[],
        &slots,
    )
    .await;

    assert!(!result.optimized);
    assert_eq!(result.size, source.len());
}

/// Port of: packages/utils/__tests__/store-incoming-image.test.ts:422 "storeIncomingImage > does not write when there is no uploader"
#[tokio::test]
async fn storeincomingimage_does_not_write_when_there_is_no_uploader() {
    let slots = EncodeSlots::new();
    let result = store(
        None,
        "ws/none",
        noisy_jpeg(800, 600, 95),
        "image/jpeg",
        Media,
        None,
        &[],
        &slots,
    )
    .await;

    assert_eq!(result.origin_path, "ws/none.jpg");
}
