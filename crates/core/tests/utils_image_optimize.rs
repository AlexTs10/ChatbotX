//! Port of `packages/utils/__tests__/image-optimize.test.ts`. sharp-made
//! inputs come from `support/images.rs`; `vi.stubEnv` is an env map that
//! always carries `stubGenerousEncodeTimeout` (60 s); `toBe(input)` (same
//! Buffer) is the same `Bytes` buffer (pointer equality).

#[path = "support/images.rs"]
mod images;
mod support;

use bytes::Bytes;
use chatbotx_core::env::EmptyEnv;
use chatbotx_core::utils::image_optimize::{
    ImageOptimizeProfile, OptimizedImage, optimize_image_for_storage,
};
use chatbotx_core::utils::image_optimize_settings::{
    ImageOptimizationSettings, InlineMaxPixels, InlineSettings, JobSettings, ProfileSettings,
    get_image_optimization_settings,
};

const GENEROUS: (&str, &str) = ("STORED_IMAGE_ENCODE_TIMEOUT_SECONDS", "60");

async fn optimize_with(
    bytes: &Bytes,
    mime: &str,
    profile: ImageOptimizeProfile,
    extra: &[(&str, &str)],
) -> OptimizedImage {
    let mut env: Vec<(String, String)> = vec![(GENEROUS.0.to_owned(), GENEROUS.1.to_owned())];
    env.extend(
        extra
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned())),
    );
    optimize_image_for_storage(bytes.clone(), mime, profile, &env).await
}

async fn optimize(bytes: &Bytes, mime: &str, profile: ImageOptimizeProfile) -> OptimizedImage {
    optimize_with(bytes, mime, profile, &[]).await
}

fn same(a: &Bytes, b: &Bytes) -> bool {
    a.as_ptr() == b.as_ptr() && a.len() == b.len()
}

use ImageOptimizeProfile::{Avatar, Media, Original};

/// Port of: packages/utils/__tests__/image-optimize.test.ts:33 "optimizeImageForStorage > re-encodes an avatar to WebP no larger than the avatar cap"
#[tokio::test]
async fn optimizeimageforstorage_re_encodes_an_avatar_to_webp_no_larger_than_the_avatar_cap() {
    let input = Bytes::from(images::noisy_jpeg(480, 480, 95));
    let result = optimize(&input, "image/jpeg", Avatar).await;
    assert!(result.optimized);
    assert_eq!(result.mime_type, "image/webp");
    assert!(result.bytes.len() < input.len());
    assert_eq!(result.width, Some(256));
    assert_eq!(result.height, Some(256));
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:51 "optimizeImageForStorage > keeps channel media as JPEG and caps its longest side"
#[tokio::test]
async fn optimizeimageforstorage_keeps_channel_media_as_jpeg_and_caps_its_longest_side() {
    let input = Bytes::from(images::noisy_jpeg(1800, 900, 98));
    let result = optimize(&input, "image/jpeg", Media).await;
    assert!(result.optimized);
    assert_eq!(result.mime_type, "image/jpeg");
    assert!(result.bytes.len() < input.len());
    assert_eq!(result.width, Some(1600));
    assert_eq!(result.height, Some(800));
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:73 "optimizeImageForStorage > never enlarges a small image"
#[tokio::test]
async fn optimizeimageforstorage_never_enlarges_a_small_image() {
    let input = Bytes::from(images::noisy_jpeg(100, 80, 100));
    let result = optimize(&input, "image/jpeg", Avatar).await;
    assert_eq!(result.width, Some(100));
    assert_eq!(result.height, Some(80));
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:88 "optimizeImageForStorage > applies EXIF orientation before re-encoding"
#[tokio::test]
async fn optimizeimageforstorage_applies_exif_orientation_before_re_encoding() {
    let input = Bytes::from(images::with_exif_orientation(
        &images::noisy_jpeg(400, 200, 98),
        6,
    ));
    let result = optimize(&input, "image/jpeg", Media).await;
    assert!(result.optimized);
    assert_eq!(result.width, Some(200));
    assert_eq!(result.height, Some(400));
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:105 "optimizeImageForStorage > leaves transparent media untouched so it is not flattened onto a background"
#[tokio::test]
async fn optimizeimageforstorage_leaves_transparent_media_untouched_so_it_is_not_flattened_onto_a_background()
 {
    let input = Bytes::from(images::png(&images::noise(600, 600, 4), 600, 600, 4));
    let result = optimize(&input, "image/png", Media).await;
    assert!(!result.optimized);
    assert!(same(&result.bytes, &input));
    assert_eq!(result.mime_type, "image/png");
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:119 "optimizeImageForStorage > re-encodes an alpha-channel screenshot that is fully opaque"
#[tokio::test]
async fn optimizeimageforstorage_re_encodes_an_alpha_channel_screenshot_that_is_fully_opaque() {
    let rgb = images::noise(900, 600, 3);
    let rgba: Vec<u8> = rgb
        .chunks(3)
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect();
    let input = Bytes::from(images::png(&rgba, 900, 600, 4));
    let result = optimize(&input, "image/png", Media).await;
    assert!(result.optimized);
    assert_eq!(result.mime_type, "image/jpeg");
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:140 "optimizeImageForStorage > does not decode an image whose declared size exceeds the pixel cap"
#[tokio::test]
async fn optimizeimageforstorage_does_not_decode_an_image_whose_declared_size_exceeds_the_pixel_cap()
 {
    // A valid 1x1 PNG whose header claims 20000x20000 pixels.
    let mut png = images::png(&[0, 0, 0], 1, 1, 3);
    png[16..20].copy_from_slice(&20_000u32.to_be_bytes());
    png[20..24].copy_from_slice(&20_000u32.to_be_bytes());
    let crc = images::crc32(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    let input = Bytes::from(png);
    let result = optimize(&input, "image/png", Media).await;
    assert!(!result.optimized);
    assert!(same(&result.bytes, &input));
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:163 "optimizeImageForStorage > leaves animated images untouched"
#[tokio::test]
async fn optimizeimageforstorage_leaves_animated_images_untouched() {
    let frame = images::noise(64, 64, 4);
    let input = Bytes::from(images::animated_gif(&frame, 64, 64));
    let result = optimize(&input, "image/gif", Media).await;
    assert!(!result.optimized);
    assert!(same(&result.bytes, &input));
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:181 "optimizeImageForStorage > stores an image sent as a document in its original form"
#[tokio::test]
async fn optimizeimageforstorage_stores_an_image_sent_as_a_document_in_its_original_form() {
    let input = Bytes::from(images::noisy_jpeg(600, 300, 98));
    let result = optimize(&input, "image/jpeg", Original).await;
    assert!(!result.optimized);
    assert!(same(&result.bytes, &input));
    assert_eq!(result.mime_type, "image/jpeg");
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:199 "optimizeImageForStorage > leaves non-image payloads untouched"
#[tokio::test]
async fn optimizeimageforstorage_leaves_non_image_payloads_untouched() {
    let input = Bytes::from_static(b"not an image");
    let result = optimize(&input, "video/mp4", Media).await;
    assert!(!result.optimized);
    assert!(same(&result.bytes, &input));
    assert_eq!(result.mime_type, "video/mp4");
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:215 "optimizeImageForStorage > returns the original bytes when the image cannot be decoded"
#[tokio::test]
async fn optimizeimageforstorage_returns_the_original_bytes_when_the_image_cannot_be_decoded() {
    let input = Bytes::from_static(b"corrupt jpeg bytes");
    let result = optimize(&input, "image/jpeg", Media).await;
    assert!(!result.optimized);
    assert!(same(&result.bytes, &input));
    assert_eq!(result.mime_type, "image/jpeg");
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:229 "optimizeImageForStorage > keeps the original when re-encoding does not save enough"
#[tokio::test]
async fn optimizeimageforstorage_keeps_the_original_when_re_encoding_does_not_save_enough() {
    let white: Vec<u8> = vec![255; 32 * 32 * 3];
    let input = Bytes::from(images::jpeg(&white, 32, 32, 50));
    let result = optimize(&input, "image/jpeg", Media).await;
    assert!(!result.optimized);
    assert!(same(&result.bytes, &input));
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:246 "optimizeImageForStorage > requires a 20% saving for media and 10% for avatars"
#[tokio::test]
async fn optimizeimageforstorage_requires_a_20_saving_for_media_and_10_for_avatars() {
    let settings = get_image_optimization_settings(&EmptyEnv).unwrap();
    assert_eq!(settings.media.min_saving_percent, 20);
    assert_eq!(settings.avatar.min_saving_percent, 10);

    let input = Bytes::from(images::noisy_jpeg(600, 400, 80));
    let result = optimize(&input, "image/jpeg", Media).await;
    assert!(!result.optimized);
    assert!(same(&result.bytes, &input));
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:268 "optimizeImageForStorage > accepts an ArrayBuffer input"
#[tokio::test]
async fn optimizeimageforstorage_accepts_an_arraybuffer_input() {
    // ArrayBuffer and Buffer are both `Bytes` here.
    let input = Bytes::from(images::noisy_jpeg(480, 480, 95).into_boxed_slice());
    let result = optimize(&input, "image/jpeg", Avatar).await;
    assert!(result.optimized);
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:287 "optimizeImageForStorage > uses the configured media size cap"
#[tokio::test]
async fn optimizeimageforstorage_uses_the_configured_media_size_cap() {
    let input = Bytes::from(images::noisy_jpeg(1800, 900, 98));
    let result = optimize_with(
        &input,
        "image/jpeg",
        Media,
        &[("STORED_IMAGE_MEDIA_MAX_DIMENSION", "800")],
    )
    .await;
    assert_eq!(result.width, Some(800));
    assert_eq!(result.height, Some(400));
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:303 "optimizeImageForStorage > uses the configured quality and avatar size"
#[tokio::test]
async fn optimizeimageforstorage_uses_the_configured_quality_and_avatar_size() {
    let env = [
        ("STORED_IMAGE_MEDIA_QUALITY", "40"),
        ("STORED_IMAGE_AVATAR_MAX_DIMENSION", "128"),
    ];
    let photo = Bytes::from(images::noisy_jpeg(600, 400, 80));
    let avatar = Bytes::from(images::noisy_jpeg(480, 480, 95));
    let media = optimize_with(&photo, "image/jpeg", Media, &env).await;
    let resized_avatar = optimize_with(&avatar, "image/jpeg", Avatar, &env).await;
    assert!(media.optimized);
    assert_eq!(resized_avatar.width, Some(128));
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:329 "optimizeImageForStorage > keeps the source when the configured saving is not reached"
#[tokio::test]
async fn optimizeimageforstorage_keeps_the_source_when_the_configured_saving_is_not_reached() {
    let input = Bytes::from(images::noisy_jpeg(1000, 800, 95));
    let by_default = optimize(&input, "image/jpeg", Media).await;
    assert!(by_default.optimized);
    let result = optimize_with(
        &input,
        "image/jpeg",
        Media,
        &[("STORED_IMAGE_MEDIA_MIN_SAVING_PERCENT", "90")],
    )
    .await;
    assert!(!result.optimized);
    assert!(same(&result.bytes, &input));
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:361 "optimizeImageForStorage with a misconfigured environment > returns the source untouched instead of failing the upload"
#[tokio::test]
async fn optimizeimageforstorage_with_a_misconfigured_environment_returns_the_source_untouched_instead_of_failing_the_upload()
 {
    let input = Bytes::from(images::noisy_jpeg(1000, 800, 95));
    let result = optimize_with(
        &input,
        "image/jpeg",
        Media,
        &[("STORED_IMAGE_MEDIA_QUALITY", "0")],
    )
    .await;
    assert!(!result.optimized);
    assert!(same(&result.bytes, &input));
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:382 "getImageOptimizationSettings > defaults to the documented policy"
#[test]
fn getimageoptimizationsettings_defaults_to_the_documented_policy() {
    assert_eq!(
        get_image_optimization_settings(&EmptyEnv).unwrap(),
        ImageOptimizationSettings {
            media: ProfileSettings {
                max_dimension: 1600,
                quality: 80,
                min_saving_percent: 20
            },
            avatar: ProfileSettings {
                max_dimension: 256,
                quality: 80,
                min_saving_percent: 10
            },
            max_input_pixels: 16_000_000,
            encode_timeout_seconds: 2,
            inline: InlineSettings {
                max_bytes: 2 * 1024 * 1024,
                max_pixels: InlineMaxPixels {
                    media: 8_000_000,
                    avatar: 4_000_000
                },
                max_concurrent_encodes: 2,
            },
            job: JobSettings {
                min_source_bytes: 32 * 1024,
                max_source_bytes: 10 * 1024 * 1024
            },
        }
    );
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:397 "getImageOptimizationSettings > reads every value from the environment"
#[test]
fn getimageoptimizationsettings_reads_every_value_from_the_environment() {
    let settings = get_image_optimization_settings(&[
        ("STORED_IMAGE_INLINE_MAX_MB", "0.5"),
        ("STORED_IMAGE_INLINE_MAX_MEGAPIXELS", "12"),
        ("STORED_IMAGE_INLINE_AVATAR_MAX_MEGAPIXELS", "2"),
        ("STORED_IMAGE_INLINE_CONCURRENCY", "0"),
        ("STORED_IMAGE_JOB_MIN_SOURCE_KB", "64"),
        ("STORED_IMAGE_JOB_MAX_SOURCE_MB", "20"),
        ("STORED_IMAGE_MAX_INPUT_MEGAPIXELS", "24"),
        ("STORED_IMAGE_ENCODE_TIMEOUT_SECONDS", "5"),
    ])
    .unwrap();
    assert_eq!(settings.max_input_pixels, 24_000_000);
    assert_eq!(settings.encode_timeout_seconds, 5);
    assert_eq!(
        settings.inline,
        InlineSettings {
            max_bytes: 512 * 1024,
            max_pixels: InlineMaxPixels {
                media: 12_000_000,
                avatar: 2_000_000
            },
            max_concurrent_encodes: 0,
        }
    );
    assert_eq!(
        settings.job,
        JobSettings {
            min_source_bytes: 64 * 1024,
            max_source_bytes: 20 * 1024 * 1024
        }
    );
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:419 "getImageOptimizationSettings > rejects an invalid STORED_IMAGE_MEDIA_QUALITY=0 instead of guessing"
#[test]
fn getimageoptimizationsettings_rejects_an_invalid_stored_image_media_quality_0_instead_of_guessing()
 {
    assert!(get_image_optimization_settings(&[("STORED_IMAGE_MEDIA_QUALITY", "0")]).is_err());
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:419 "getImageOptimizationSettings > rejects an invalid STORED_IMAGE_MEDIA_QUALITY=101 instead of guessing"
#[test]
fn getimageoptimizationsettings_rejects_an_invalid_stored_image_media_quality_101_instead_of_guessing()
 {
    assert!(get_image_optimization_settings(&[("STORED_IMAGE_MEDIA_QUALITY", "101")]).is_err());
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:419 "getImageOptimizationSettings > rejects an invalid STORED_IMAGE_AVATAR_MAX_DIMENSION=abc instead of guessing"
#[test]
fn getimageoptimizationsettings_rejects_an_invalid_stored_image_avatar_max_dimension_abc_instead_of_guessing()
 {
    assert!(
        get_image_optimization_settings(&[("STORED_IMAGE_AVATAR_MAX_DIMENSION", "abc")]).is_err()
    );
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:419 "getImageOptimizationSettings > rejects an invalid STORED_IMAGE_MEDIA_MIN_SAVING_PERCENT=100 instead of guessing"
#[test]
fn getimageoptimizationsettings_rejects_an_invalid_stored_image_media_min_saving_percent_100_instead_of_guessing()
 {
    assert!(
        get_image_optimization_settings(&[("STORED_IMAGE_MEDIA_MIN_SAVING_PERCENT", "100")])
            .is_err()
    );
}

/// Port of: packages/utils/__tests__/image-optimize.test.ts:419 "getImageOptimizationSettings > rejects an invalid STORED_IMAGE_INLINE_CONCURRENCY=-1 instead of guessing"
#[test]
fn getimageoptimizationsettings_rejects_an_invalid_stored_image_inline_concurrency_1_instead_of_guessing()
 {
    assert!(get_image_optimization_settings(&[("STORED_IMAGE_INLINE_CONCURRENCY", "-1")]).is_err());
}
