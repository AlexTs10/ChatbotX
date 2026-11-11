//! MIME helpers (spec 1.2, 1.3; JS `mime-type.ts` and `helper.ts`).

use crate::FileType;

/// Non-standard MIME types some clients and hosts send, mapped to the
/// registered type. Meta (WhatsApp error 131053) rejects `image/jpg`.
fn alias(base: &str) -> Option<&'static str> {
    match base {
        "image/jpg" | "image/pjpeg" => Some("image/jpeg"),
        "image/x-png" => Some("image/png"),
        _ => None,
    }
}

/// JS `String.prototype.trim` white space (Unicode `White_Space` plus the
/// BOM, which Rust's `trim` keeps).
fn is_js_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{FEFF}'
}

/// Lower-cases and trims the base type and replaces a known alias with its
/// registered type. Parameters (from the first `;` on) are kept unchanged.
pub fn normalize_mime_type(mime: &str) -> String {
    let (base, params) = match mime.find(';') {
        Some(index) => (&mime[..index], &mime[index..]),
        None => (mime, ""),
    };
    let base = base.trim_matches(is_js_space).to_lowercase();
    let mut out = match alias(&base) {
        Some(registered) => registered.to_owned(),
        None => base,
    };
    out.push_str(params);
    out
}

/// First segment before `/`: `image`, `video`, `audio`, else `file`. Case
/// sensitive, like the JS.
pub fn guess_file_type_from_mime_type(mime: &str) -> FileType {
    match mime.split('/').next().unwrap_or_default() {
        "image" => FileType::Image,
        "video" => FileType::Video,
        "audio" => FileType::Audio,
        _ => FileType::File,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_keeps_params_untouched_and_empty_stays_empty() {
        assert_eq!(normalize_mime_type(""), "");
        assert_eq!(
            normalize_mime_type(" Image/X-PNG ; Name=A"),
            "image/png; Name=A"
        );
        assert_eq!(normalize_mime_type("\u{FEFF}TEXT/Plain"), "text/plain");
        assert_eq!(
            normalize_mime_type("application/x-foo"),
            "application/x-foo"
        );
    }

    #[test]
    fn guess_file_type_matches_the_js_switch() {
        assert_eq!(guess_file_type_from_mime_type("image/png"), FileType::Image);
        assert_eq!(guess_file_type_from_mime_type("video/mp4"), FileType::Video);
        assert_eq!(guess_file_type_from_mime_type("audio/ogg"), FileType::Audio);
        assert_eq!(guess_file_type_from_mime_type("image"), FileType::Image);
        assert_eq!(guess_file_type_from_mime_type("IMAGE/PNG"), FileType::File);
        assert_eq!(guess_file_type_from_mime_type(""), FileType::File);
        assert_eq!(
            guess_file_type_from_mime_type("application/pdf"),
            FileType::File
        );
        assert_eq!(FileType::Image.to_string(), "image");
    }
}
