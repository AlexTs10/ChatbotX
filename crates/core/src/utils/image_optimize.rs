//! `packages/utils/src/image-optimize.ts`: re-encodes images before they
//! are stored (sharp/libvips in the original; `image` decode + Lanczos3
//! resize, `jpeg-encoder` and libwebp here). Failure-safe: anything
//! unsupported, animated, transparent media, undecodable, too slow or not
//! smaller enough is returned untouched (the same `Bytes`).

use std::io::Cursor;
use std::time::Duration;

use bytes::Bytes;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};

use super::image_optimize_settings::{ProfileSettings, get_image_optimization_settings};
use crate::env::Env;

string_enum! {
    /// `ImageOptimizeProfile`.
    pub enum ImageOptimizeProfile {
        /// Our UI only: WebP, 256 px by default.
        Avatar = "avatar",
        /// May be re-sent to a channel: JPEG (4:4:4), 1600 px by default.
        Media = "media",
        /// Stored as sent (a document or file).
        Original = "original",
    }
}

/// `OptimizedImage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizedImage {
    pub bytes: Bytes,
    pub mime_type: String,
    /// The bytes are the re-encoded image.
    pub optimized: bool,
    /// Output size; set only when `optimized`.
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// File extension of the encoded format; set only when `optimized`.
    pub extension: Option<&'static str>,
}

const OPTIMIZABLE_MIME_TYPES: &[&str] = &[
    "image/jpeg",
    "image/jpg",
    "image/pjpeg",
    "image/png",
    "image/webp",
];

fn normalize_mime_type(mime_type: &str) -> String {
    crate::js::trim(mime_type.split(';').next().unwrap_or("")).to_lowercase()
}

/// `readImageDimensions`: width and height from the header (nothing is
/// decoded), or `None` when the bytes are not an image.
pub fn read_image_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

struct Encoded {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

fn is_animated_webp(bytes: &[u8]) -> bool {
    image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)).is_ok_and(|d| d.has_animation())
}

/// A JPEG's pixels through `jpeg-decoder` (RGB, grayscale or CMYK).
fn decode_jpeg(bytes: &[u8]) -> Result<DynamicImage, String> {
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(bytes));
    let pixels = decoder.decode().map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("missing JPEG header")?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    let image = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            image::RgbImage::from_raw(w, h, pixels).map(DynamicImage::ImageRgb8)
        }
        jpeg_decoder::PixelFormat::L8 => {
            image::GrayImage::from_raw(w, h, pixels).map(DynamicImage::ImageLuma8)
        }
        jpeg_decoder::PixelFormat::L16 => {
            let wide: Vec<u16> = pixels
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            image::ImageBuffer::from_raw(w, h, wide).map(DynamicImage::ImageLuma16)
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            let rgb: Vec<u8> = pixels
                .chunks_exact(4)
                .flat_map(|p| {
                    let k = 255 - u16::from(p[3]);
                    [0, 1, 2].map(|i| ((255 - u16::from(p[i])) * k / 255) as u8)
                })
                .collect();
            image::RgbImage::from_raw(w, h, rgb).map(DynamicImage::ImageRgb8)
        }
    };
    image.ok_or_else(|| "JPEG pixel buffer has the wrong size".to_owned())
}

fn resize_inside(image: DynamicImage, max: u32) -> DynamicImage {
    if image.width() <= max && image.height() <= max {
        return image;
    }
    image.resize(max, max, image::imageops::FilterType::Lanczos3)
}

fn encode_jpeg(image: &DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let rgb = image.to_rgb8();
    let mut out = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, quality);
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1);
    encoder.set_optimized_huffman_tables(true);
    let (w, h) = (
        u16::try_from(rgb.width()).map_err(|e| e.to_string())?,
        u16::try_from(rgb.height()).map_err(|e| e.to_string())?,
    );
    encoder
        .encode(rgb.as_raw(), w, h, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

fn encode_webp(image: &DynamicImage, quality: u8) -> Vec<u8> {
    if image.color().has_alpha() {
        let rgba = image.to_rgba8();
        webp::Encoder::from_rgba(rgba.as_raw(), rgba.width(), rgba.height())
            .encode(f32::from(quality))
            .to_vec()
    } else {
        let rgb = image.to_rgb8();
        webp::Encoder::from_rgb(rgb.as_raw(), rgb.width(), rgb.height())
            .encode(f32::from(quality))
            .to_vec()
    }
}

/// `Ok(None)` = keep the source (animated / transparent media).
fn encode(
    bytes: &[u8],
    profile: ImageOptimizeProfile,
    settings: ProfileSettings,
    max_input_pixels: u64,
) -> Result<Option<Encoded>, String> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = reader.format().ok_or("unsupported image format")?;
    let (w, h) = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?
        .into_dimensions()
        .map_err(|e| e.to_string())?;
    if u64::from(w) * u64::from(h) > max_input_pixels {
        return Err("Input image exceeds pixel limit".to_owned());
    }
    if format == ImageFormat::WebP && is_animated_webp(bytes) {
        return Ok(None);
    }
    if !matches!(
        format,
        ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP
    ) {
        return Err(format!("unsupported image format {format:?}"));
    }
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut image = if format == ImageFormat::Jpeg {
        // zune-jpeg (image's JPEG decoder) misdecodes some valid 4:2:0
        // files with optimized Huffman tables (libjpeg and jpeg-decoder
        // agree); the pixels come from jpeg-decoder, the orientation from
        // the EXIF read above.
        decode_jpeg(bytes)?
    } else {
        DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?
    };
    if profile == ImageOptimizeProfile::Media && image.color().has_alpha() {
        let opaque = image.to_rgba8().pixels().all(|p| p.0[3] == 255);
        if !opaque {
            return Ok(None);
        }
    }
    image.apply_orientation(orientation);
    let image = resize_inside(image, settings.max_dimension);
    let data = match profile {
        ImageOptimizeProfile::Avatar => encode_webp(&image, settings.quality),
        _ => encode_jpeg(&image, settings.quality)?,
    };
    Ok(Some(Encoded {
        data,
        width: image.width(),
        height: image.height(),
    }))
}

/// `optimizeImageForStorage`.
pub async fn optimize_image_for_storage(
    bytes: Bytes,
    mime_type: &str,
    profile: ImageOptimizeProfile,
    env: &(dyn Env + Sync),
) -> OptimizedImage {
    let unchanged = |bytes: Bytes| OptimizedImage {
        bytes,
        mime_type: mime_type.to_owned(),
        optimized: false,
        width: None,
        height: None,
        extension: None,
    };
    if profile == ImageOptimizeProfile::Original
        || !OPTIMIZABLE_MIME_TYPES.contains(&normalize_mime_type(mime_type).as_str())
    {
        return unchanged(bytes);
    }
    let settings = match get_image_optimization_settings(env) {
        Ok(settings) => settings,
        Err(err) => {
            tracing::warn!(module = "image-optimize", err = %err, profile = profile.as_str(), "Image re-encode failed; keeping the source");
            return unchanged(bytes);
        }
    };
    let profile_settings = if profile == ImageOptimizeProfile::Avatar {
        settings.avatar
    } else {
        settings.media
    };
    let source = bytes.clone();
    let task = tokio::task::spawn_blocking(move || {
        encode(
            &source,
            profile,
            profile_settings,
            settings.max_input_pixels,
        )
    });
    let outcome = match tokio::time::timeout(
        Duration::from_secs(settings.encode_timeout_seconds),
        task,
    )
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(join)) => Err(join.to_string()),
        Err(_) => Err("Image encode timed out".to_owned()),
    };
    let encoded = match outcome {
        Ok(Some(encoded)) => encoded,
        Ok(None) => return unchanged(bytes),
        Err(err) => {
            tracing::warn!(module = "image-optimize", err = %err, profile = profile.as_str(), "Image re-encode failed; keeping the source");
            return unchanged(bytes);
        }
    };
    let max_bytes =
        bytes.len() as f64 * (1.0 - f64::from(profile_settings.min_saving_percent) / 100.0);
    if encoded.data.len() as f64 > max_bytes {
        return unchanged(bytes);
    }
    let (mime, extension) = match profile {
        ImageOptimizeProfile::Avatar => ("image/webp", "webp"),
        _ => ("image/jpeg", "jpg"),
    };
    OptimizedImage {
        bytes: Bytes::from(encoded.data),
        mime_type: mime.to_owned(),
        optimized: true,
        width: Some(encoded.width),
        height: Some(encoded.height),
        extension: Some(extension),
    }
}
