//! `apps/worker/src/heavy/handlers/channel-safe-image.ts` and the image
//! work of `segment-image.ts` (sharp in the original): format sniffing,
//! EXIF orientation, real-transparency check, the channel-safe re-encode
//! (opaque -> JPEG q90 4:4:4, transparent -> PNG) and the crop.

use std::io::Cursor;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};

use super::image_optimize::{decode_jpeg, encode_jpeg};

const JPEG_QUALITY: u8 = 90;

/// `ChannelSafeImage`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSafeImage {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

/// The `sharp(...).metadata().format` of the formats the image APIs take
/// (`png`, `jpeg`, `webp`, `gif`), from the magic bytes.
pub fn sniff_format(bytes: &[u8]) -> Option<&'static str> {
    match image::guess_format(bytes).ok()? {
        ImageFormat::Png => Some("png"),
        ImageFormat::Jpeg => Some("jpeg"),
        ImageFormat::WebP => Some("webp"),
        ImageFormat::Gif => Some("gif"),
        _ => None,
    }
}

/// The decoded pixels with the EXIF orientation applied (`.rotate()`).
fn decode_oriented(bytes: &[u8]) -> Result<DynamicImage, String> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let format = reader.format().ok_or("unsupported image format")?;
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut image = if format == ImageFormat::Jpeg {
        decode_jpeg(bytes)?
    } else {
        DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?
    };
    image.apply_orientation(orientation);
    Ok(image)
}

/// `hasTransparency`: some pixel is see-through (an opaque alpha channel
/// does not count).
fn has_transparency(image: &DynamicImage) -> bool {
    image.color().has_alpha() && image.to_rgba8().pixels().any(|p| p.0[3] != 255)
}

/// `encodeChannelSafeImage`.
fn encode_channel_safe(
    image: &DynamicImage,
    transparent: bool,
) -> Result<ChannelSafeImage, String> {
    if transparent {
        let mut out = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        return Ok(ChannelSafeImage {
            bytes: out,
            content_type: "image/png".into(),
        });
    }
    Ok(ChannelSafeImage {
        bytes: encode_jpeg(image, JPEG_QUALITY)?,
        content_type: "image/jpeg".into(),
    })
}

/// `toChannelSafeImage`: a WebP is re-encoded (WhatsApp treats WebP as a
/// sticker); any other format is returned untouched.
pub fn to_channel_safe_image(
    bytes: Vec<u8>,
    content_type: &str,
) -> Result<ChannelSafeImage, String> {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mime != "image/webp" {
        return Ok(ChannelSafeImage {
            bytes,
            content_type: content_type.to_owned(),
        });
    }
    let image = decode_oriented(&bytes)?;
    encode_channel_safe(&image, has_transparency(&image))
}

/// Why an image cannot be segmented (the expected step errors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizeError {
    NotAnImage,
    TooManyPixels(u64),
    Other(String),
}

/// `NormalizedImage`: the bytes SAM sees, their size, and the oriented
/// source the crop is cut from.
pub struct NormalizedImage {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub transparent: bool,
    source: DynamicImage,
}

/// `normalizeImage`: orientation applied, one channel-safe re-encode.
/// The pixel ceiling is checked from the header before any decode.
pub fn normalize_for_segmentation(
    bytes: &[u8],
    max_pixels: u64,
) -> Result<NormalizedImage, NormalizeError> {
    if !matches!(sniff_format(bytes), Some("png" | "jpeg" | "webp" | "gif")) {
        return Err(NormalizeError::NotAnImage);
    }
    let (w, h) =
        super::image_optimize::read_image_dimensions(bytes).ok_or(NormalizeError::NotAnImage)?;
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > max_pixels {
        return Err(NormalizeError::TooManyPixels(max_pixels));
    }
    // ponytail: GIF is not a decoder feature of the `image` build; it fails
    // here as an unsupported image (sharp decodes the first frame).
    let source = decode_oriented(bytes).map_err(|_| NormalizeError::NotAnImage)?;
    let transparent = has_transparency(&source);
    let encoded = encode_channel_safe(&source, transparent).map_err(NormalizeError::Other)?;
    Ok(NormalizedImage {
        bytes: encoded.bytes,
        mime_type: encoded.content_type,
        width: source.width(),
        height: source.height(),
        transparent,
        source,
    })
}

/// `renderCrop`: the half-open box cut from the oriented source.
pub fn render_crop(
    image: &NormalizedImage,
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
) -> Result<ChannelSafeImage, String> {
    if right <= left || bottom <= top || right > image.width || bottom > image.height {
        return Err("Crop box is outside the image".into());
    }
    let crop = image.source.crop_imm(left, top, right - left, bottom - top);
    encode_channel_safe(&crop, image.transparent)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32, alpha: u8) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, alpha]));
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn normalizes_crops_and_keeps_transparency() {
        let opaque = normalize_for_segmentation(&png(8, 6, 255), 1000).unwrap();
        assert_eq!((opaque.width, opaque.height), (8, 6));
        assert_eq!(opaque.mime_type, "image/jpeg");
        let crop = render_crop(&opaque, 1, 1, 5, 4).unwrap();
        assert_eq!(sniff_format(&crop.bytes), Some("jpeg"));
        assert_eq!(
            super::super::image_optimize::read_image_dimensions(&crop.bytes),
            Some((4, 3))
        );
        assert!(render_crop(&opaque, 1, 1, 9, 4).is_err());

        let clear = normalize_for_segmentation(&png(4, 4, 0), 1000).unwrap();
        assert_eq!(clear.mime_type, "image/png");
        assert_eq!(
            normalize_for_segmentation(&png(40, 40, 255), 1000).err(),
            Some(NormalizeError::TooManyPixels(1000))
        );
        assert_eq!(
            normalize_for_segmentation(b"not an image", 1000).err(),
            Some(NormalizeError::NotAnImage)
        );
    }

    #[test]
    fn only_webp_is_re_encoded() {
        let png = png(2, 2, 255);
        let same = to_channel_safe_image(png.clone(), "image/png").unwrap();
        assert_eq!(same.bytes, png);
        let rgb = image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]));
        let webp = webp::Encoder::from_rgb(rgb.as_raw(), 2, 2)
            .encode(90.0)
            .to_vec();
        let safe = to_channel_safe_image(webp, "image/webp").unwrap();
        assert_eq!(safe.content_type, "image/jpeg");
    }
}
