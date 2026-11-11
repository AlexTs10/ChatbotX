//! Port of `packages/filesystem/__tests__/mime-type.test.ts` (P2.9).
//!
//! Each `test.each` row is one Rust test. The line in each doc comment is
//! the line of the row in the original table.

use chatbotx_files::normalize_mime_type;

// test.each(...)("maps the non-standard %s to %s") at line 5.

/// Port of: packages/filesystem/__tests__/mime-type.test.ts:6 "normalizeMimeType > maps the non-standard image/jpg to image/jpeg"
#[test]
fn normalize_mime_type_maps_the_non_standard_image_jpg_to_image_jpeg() {
    assert_eq!(normalize_mime_type("image/jpg"), "image/jpeg");
}

/// Port of: packages/filesystem/__tests__/mime-type.test.ts:7 "normalizeMimeType > maps the non-standard image/pjpeg to image/jpeg"
#[test]
fn normalize_mime_type_maps_the_non_standard_image_pjpeg_to_image_jpeg() {
    assert_eq!(normalize_mime_type("image/pjpeg"), "image/jpeg");
}

/// Port of: packages/filesystem/__tests__/mime-type.test.ts:8 "normalizeMimeType > maps the non-standard image/x-png to image/png"
#[test]
fn normalize_mime_type_maps_the_non_standard_image_x_png_to_image_png() {
    assert_eq!(normalize_mime_type("image/x-png"), "image/png");
}

/// Port of: packages/filesystem/__tests__/mime-type.test.ts:9 "normalizeMimeType > maps the non-standard IMAGE/JPG to image/jpeg"
#[test]
fn normalize_mime_type_maps_the_non_standard_upper_case_image_jpg_to_image_jpeg() {
    assert_eq!(normalize_mime_type("IMAGE/JPG"), "image/jpeg");
}

/// Port of: packages/filesystem/__tests__/mime-type.test.ts:10 "normalizeMimeType > maps the non-standard  image/jpg  to image/jpeg"
#[test]
fn normalize_mime_type_maps_the_non_standard_padded_image_jpg_to_image_jpeg() {
    assert_eq!(normalize_mime_type(" image/jpg "), "image/jpeg");
}

// test.each(...)("leaves the registered type %s unchanged") at line 15.

/// Port of: packages/filesystem/__tests__/mime-type.test.ts:16 "normalizeMimeType > leaves the registered type image/jpeg unchanged"
#[test]
fn normalize_mime_type_leaves_the_registered_type_image_jpeg_unchanged() {
    assert_eq!(normalize_mime_type("image/jpeg"), "image/jpeg");
}

/// Port of: packages/filesystem/__tests__/mime-type.test.ts:17 "normalizeMimeType > leaves the registered type image/png unchanged"
#[test]
fn normalize_mime_type_leaves_the_registered_type_image_png_unchanged() {
    assert_eq!(normalize_mime_type("image/png"), "image/png");
}

/// Port of: packages/filesystem/__tests__/mime-type.test.ts:18 "normalizeMimeType > leaves the registered type image/webp unchanged"
#[test]
fn normalize_mime_type_leaves_the_registered_type_image_webp_unchanged() {
    assert_eq!(normalize_mime_type("image/webp"), "image/webp");
}

/// Port of: packages/filesystem/__tests__/mime-type.test.ts:19 "normalizeMimeType > leaves the registered type video/mp4 unchanged"
#[test]
fn normalize_mime_type_leaves_the_registered_type_video_mp4_unchanged() {
    assert_eq!(normalize_mime_type("video/mp4"), "video/mp4");
}

/// Port of: packages/filesystem/__tests__/mime-type.test.ts:20 "normalizeMimeType > leaves the registered type application/octet-stream unchanged"
#[test]
fn normalize_mime_type_leaves_the_registered_type_application_octet_stream_unchanged() {
    assert_eq!(
        normalize_mime_type("application/octet-stream"),
        "application/octet-stream",
    );
}

/// Port of: packages/filesystem/__tests__/mime-type.test.ts:25 "normalizeMimeType > keeps parameters after the base type"
#[test]
fn normalize_mime_type_keeps_parameters_after_the_base_type() {
    assert_eq!(
        normalize_mime_type("text/plain; charset=utf-8"),
        "text/plain; charset=utf-8",
    );
    assert_eq!(normalize_mime_type("image/jpg;q=1"), "image/jpeg;q=1");
}
