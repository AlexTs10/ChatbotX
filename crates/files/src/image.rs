//! Image dimensions (spec 1.4; JS `getImageDimensions` over `image-size`).
//!
//! Raster formats use the header-only `imagesize` crate. SVG (which
//! `imagesize` does not read) uses a std-only port of the `image-size` SVG
//! handler: `width` / `height` / `viewBox` of the root `<svg>` element.

use crate::mime::guess_file_type_from_mime_type;
use crate::{DEFAULT_MIME_TYPE, FileType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageDimensions {
    pub mime_type: String,
    pub file_type: FileType,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// `{ mime_type, file_type }`, plus `width` / `height` for `image/*` types.
/// When an image cannot be measured, the result is forced to
/// `file` / `application/octet-stream` (the stored object keeps its type).
pub fn get_image_dimensions(mime_type: &str, bytes: &[u8]) -> ImageDimensions {
    let mut result = ImageDimensions {
        mime_type: mime_type.to_owned(),
        file_type: guess_file_type_from_mime_type(mime_type),
        width: None,
        height: None,
    };
    if mime_type.starts_with("image/") {
        match measure(bytes) {
            Ok((width, height)) => {
                result.width = Some(width);
                result.height = Some(height);
            }
            Err(error) => {
                tracing::warn!(target: "uploader", %error, "Unable to retrieve image dimensions");
                result.file_type = FileType::File;
                result.mime_type = DEFAULT_MIME_TYPE.to_owned();
            }
        }
    }
    result
}

/// `(width, height)` of a raster image or an SVG document.
fn measure(bytes: &[u8]) -> Result<(u32, u32), String> {
    match imagesize::blob_size(bytes) {
        Ok(size) => {
            let width = u32::try_from(size.width).map_err(|e| e.to_string())?;
            let height = u32::try_from(size.height).map_err(|e| e.to_string())?;
            Ok((width, height))
        }
        Err(raster_error) => svg_size(bytes).ok_or_else(|| raster_error.to_string()),
    }
}

// ---------------------------------------------------------------------------
// SVG (port of image-size `types/svg.ts`)
// ---------------------------------------------------------------------------

/// `(width, height)` from the root `<svg ...>` tag, or `None` ("Invalid SVG").
fn svg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    // validate: the root tag must start in the first 1000 bytes.
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1000)]);
    find_svg_root(&head)?;
    let text = String::from_utf8_lossy(bytes);
    let root = find_svg_root(&text)?;

    let width = attribute(root, "width", false)
        .filter(|v| !v.contains('%'))
        .and_then(parse_length);
    let height = attribute(root, "height", false)
        .filter(|v| !v.contains('%'))
        .and_then(parse_length);
    // JS truthiness: 0 and NaN count as missing.
    let width = width.filter(|v| *v > 0.0);
    let height = height.filter(|v| *v > 0.0);

    let (width, height) = match (width, height) {
        (Some(w), Some(h)) => (w, h),
        _ => {
            let viewbox = attribute(root, "viewBox", true)?;
            let bounds: Vec<&str> = viewbox.split(' ').collect();
            let vb_width = bounds.get(2).and_then(|v| parse_length(v))?;
            let vb_height = bounds.get(3).and_then(|v| parse_length(v))?;
            let ratio = vb_width / vb_height;
            match (width, height) {
                (Some(w), _) => (w, (w / ratio).floor()),
                (None, Some(h)) => ((h * ratio).floor(), h),
                (None, None) => (vb_width, vb_height),
            }
        }
    };
    Some((to_u32(width)?, to_u32(height)?))
}

fn to_u32(value: f64) -> Option<u32> {
    (value.is_finite() && value >= 0.0 && value <= f64::from(u32::MAX)).then_some(value as u32)
}

/// The text of the first `<svg` + white space tag, up to its `>` (quoted
/// attribute values may contain `>`). JS regex `/<svg\s([^>"']|"[^"]*"|'[^']*')*>/`.
fn find_svg_root(text: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(offset) = text[from..].find("<svg") {
        let start = from + offset;
        let after = start + "<svg".len();
        if text[after..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
        {
            if let Some(end) = tag_end(&text[after..]) {
                return Some(&text[start..after + end + 1]);
            }
        }
        from = after;
    }
    None
}

/// Index of the closing `>` outside quotes, or `None` when a quote is open
/// at the end of the text.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (index, c) in text.char_indices() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '>' => return Some(index),
            None => {}
        }
    }
    None
}

/// Value of the first `\s{name}=(['"])(.+?)\1` match in the root tag. Width
/// and height values with `%` are skipped by the caller (the JS regex
/// `[^%]+?` would move on to the next match; a second `width` is rare).
fn attribute<'a>(root: &'a str, name: &str, ignore_case: bool) -> Option<&'a str> {
    let haystack = if ignore_case {
        root.to_ascii_lowercase()
    } else {
        root.to_owned()
    };
    let needle = if ignore_case {
        format!("{}=", name.to_ascii_lowercase())
    } else {
        format!("{name}=")
    };
    let mut from = 0;
    while let Some(offset) = haystack[from..].find(&needle) {
        let start = from + offset;
        from = start + needle.len();
        let preceded_by_space = root[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
        if !preceded_by_space {
            continue;
        }
        let value_start = start + needle.len();
        let quote = root[value_start..].chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let inner = &root[value_start + 1..];
        match inner.find(quote) {
            Some(close) if close > 0 => return Some(&inner[..close]),
            _ => {}
        }
    }
    None
}

/// JS `parseLength`: the first `([0-9.]+)([a-z]*)` match, converted to
/// pixels and rounded (`Math.round`).
fn parse_length(value: &str) -> Option<f64> {
    let start = value.find(|c: char| c.is_ascii_digit() || c == '.')?;
    let rest = &value[start..];
    let number_end = rest
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(rest.len());
    let number: f64 = rest[..number_end].parse().ok()?;
    let unit_rest = &rest[number_end..];
    let unit_end = unit_rest
        .find(|c: char| !c.is_ascii_lowercase())
        .unwrap_or(unit_rest.len());
    let factor = match &unit_rest[..unit_end] {
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "em" => 16.0,
        "ex" => 8.0,
        "m" => 96.0 / 2.54 * 100.0,
        "mm" => 96.0 / 2.54 / 10.0,
        "pc" => 96.0 / 72.0 / 12.0,
        "pt" => 96.0 / 72.0,
        _ => 1.0,
    };
    // Math.round: halves round up.
    Some((number * factor + 0.5).floor())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1x1 PNG.
    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89,
    ];

    #[test]
    fn measures_a_png() {
        let dims = get_image_dimensions("image/png", PNG_1X1);
        assert_eq!(dims.width, Some(1));
        assert_eq!(dims.height, Some(1));
        assert_eq!(dims.file_type, FileType::Image);
        assert_eq!(dims.mime_type, "image/png");
    }

    #[test]
    fn a_broken_image_becomes_a_plain_file() {
        let dims = get_image_dimensions("image/png", b"not an image");
        assert_eq!(dims.file_type, FileType::File);
        assert_eq!(dims.mime_type, DEFAULT_MIME_TYPE);
        assert_eq!(dims.width, None);
    }

    #[test]
    fn non_images_are_not_probed() {
        let dims = get_image_dimensions("video/mp4", PNG_1X1);
        assert_eq!(dims.file_type, FileType::Video);
        assert_eq!(dims.mime_type, "video/mp4");
        assert_eq!(dims.width, None);
        let dims = get_image_dimensions("", b"");
        assert_eq!(dims.file_type, FileType::File);
        assert_eq!(dims.mime_type, "");
    }

    #[test]
    fn svg_width_and_height_with_units() {
        let svg = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="2in" height='30.4px' stroke-width="9"></svg>"#;
        let dims = get_image_dimensions("image/svg+xml", svg);
        assert_eq!((dims.width, dims.height), (Some(192), Some(30)));
        assert_eq!(dims.file_type, FileType::Image);
    }

    #[test]
    fn svg_view_box_alone_and_with_one_side() {
        let svg = br#"<svg viewBox="0 0 200 100" data-x="a>b"><g/></svg>"#;
        assert_eq!(svg_size(svg), Some((200, 100)));
        let svg = br#"<svg width="50" viewbox="0 0 200 100"></svg>"#;
        assert_eq!(svg_size(svg), Some((50, 25)));
        let svg = br#"<svg height="50" viewBox="0 0 200 100"></svg>"#;
        assert_eq!(svg_size(svg), Some((100, 50)));
        let svg = br#"<svg width="100%" height="100%" viewBox="0 0 30 40"></svg>"#;
        assert_eq!(svg_size(svg), Some((30, 40)));
    }

    #[test]
    fn svg_without_sizes_is_invalid() {
        assert_eq!(svg_size(br#"<svg xmlns="x"></svg>"#), None);
        assert_eq!(svg_size(b"<svgfoo width=\"1\" height=\"1\">"), None);
        let dims = get_image_dimensions("image/svg+xml", br#"<svg></svg>"#);
        assert_eq!(dims.file_type, FileType::File);
    }

    #[test]
    fn parse_length_matches_js() {
        assert_eq!(parse_length("10"), Some(10.0));
        assert_eq!(parse_length("2.5"), Some(3.0));
        assert_eq!(parse_length("1cm"), Some(38.0));
        assert_eq!(parse_length("12pt"), Some(16.0));
        assert_eq!(parse_length("1.2.3"), None);
        assert_eq!(parse_length("auto"), None);
    }
}
