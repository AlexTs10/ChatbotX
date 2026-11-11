//! Storage key and prefix helpers (spec 1.2, 1.5; JS `helper.ts` and
//! `copy-source.ts`), plus JS `encodeURIComponent` / `decodeURIComponent`.

use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, NaiveDate, Utc};

use crate::{ChannelMediaLocation, FilesError};

/// JS `pathJoin(...parts)`: join with `/`, then collapse every run of `/`
/// into one. Leading and trailing `/` stay (one of them). Note: this also
/// collapses the `//` of `https://`, like the JS.
pub fn path_join(parts: &[&str]) -> String {
    let joined = parts.join("/");
    let mut out = String::with_capacity(joined.len());
    for c in joined.chars() {
        if c == '/' && out.ends_with('/') {
            continue;
        }
        out.push(c);
    }
    out
}

/// The UTC date of "now" (system clock).
fn today_utc() -> NaiveDate {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    i64::try_from(secs)
        .ok()
        .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0))
        .map(|dt| dt.date_naive())
        .unwrap_or_default()
}

/// JS `getDatePrefix()`: `YYYY/MM/DD`.
fn date_prefix(date: NaiveDate) -> String {
    date.format("%Y/%m/%d").to_string()
}

/// `public/ws/{workspaceId}/{YYYY/MM/DD}` for the given date.
pub fn get_storage_prefix_at(workspace_id: &str, date: NaiveDate) -> String {
    format!("public/ws/{workspace_id}/{}", date_prefix(date))
}

/// `public/ws/{workspaceId}/{YYYY/MM/DD}` for today (UTC).
pub fn get_storage_prefix(workspace_id: &str) -> String {
    get_storage_prefix_at(workspace_id, today_utc())
}

/// Channel first, so one lifecycle rule can target one channel.
fn channel_media_path(loc: &ChannelMediaLocation, date: NaiveDate) -> String {
    format!(
        "{}/{}/{}/{}",
        loc.channel,
        loc.workspace_id,
        loc.integration_id,
        date_prefix(date)
    )
}

/// Media received from a channel, uploaded `public-read`:
/// `public/{channel}/{workspaceId}/{integrationId}/{YYYY/MM/DD}`.
pub fn get_channel_media_prefix_at(loc: &ChannelMediaLocation, date: NaiveDate) -> String {
    format!("public/{}", channel_media_path(loc, date))
}

/// [`get_channel_media_prefix_at`] for today (UTC).
pub fn get_channel_media_prefix(loc: &ChannelMediaLocation) -> String {
    get_channel_media_prefix_at(loc, today_utc())
}

/// Private mirrored copies of channel media, same layout under `workspace/`.
pub fn get_mirrored_channel_media_prefix_at(loc: &ChannelMediaLocation, date: NaiveDate) -> String {
    format!("workspace/{}", channel_media_path(loc, date))
}

/// [`get_mirrored_channel_media_prefix_at`] for today (UTC).
pub fn get_mirrored_channel_media_prefix(loc: &ChannelMediaLocation) -> String {
    get_mirrored_channel_media_prefix_at(loc, today_utc())
}

/// Builds the `x-amz-copy-source` value for CopyObject:
/// `{endpointPath/}{bucket}/{encoded key}`. Each `/` segment of the key is
/// `encodeURIComponent`-ed. The endpoint path (an R2 endpoint that already
/// contains the bucket) is replicated because the copy source is a header.
pub fn build_copy_source(
    source_path: &str,
    bucket: &str,
    endpoint: Option<&str>,
) -> Result<String, FilesError> {
    let encoded_source = source_path
        .split('/')
        .map(encode_uri_component)
        .collect::<Vec<_>>()
        .join("/");

    let endpoint_path = match endpoint {
        // JS `endpoint ? ... : ""`: the empty string is falsy.
        Some(endpoint) if !endpoint.is_empty() => {
            let url = url::Url::parse(endpoint)
                .map_err(|error| FilesError::InvalidEndpoint(format!("{endpoint}: {error}")))?;
            url.path().trim_matches('/').to_owned()
        }
        _ => String::new(),
    };
    let prefix = if endpoint_path.is_empty() {
        String::new()
    } else {
        format!("{endpoint_path}/")
    };

    Ok(format!("{prefix}{bucket}/{encoded_source}"))
}

/// JS `encodeURIComponent`: keep `A-Z a-z 0-9 - _ . ! ~ * ' ( )`, percent-
/// encode every other UTF-8 byte with upper-case hex.
pub(crate) fn encode_uri_component(input: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(input.len());
    for &byte in input.as_bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            );
        if keep {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    out
}

/// JS `decodeURIComponent`: decodes every `%XX`; a `%` without two hex
/// digits, or bytes that are not UTF-8, is an error (JS `URIError`).
pub(crate) fn decode_uri_component(input: &str) -> Result<String, String> {
    fn hex(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let high = bytes.get(i + 1).copied().and_then(hex);
            let low = bytes.get(i + 2).copied().and_then(hex);
            match (high, low) {
                (Some(high), Some(low)) => {
                    out.push(high << 4 | low);
                    i += 3;
                }
                _ => return Err(format!("URI malformed: {input}")),
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| format!("URI malformed: {input}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location() -> ChannelMediaLocation {
        ChannelMediaLocation {
            channel: "zalo".to_owned(),
            workspace_id: "1".to_owned(),
            integration_id: "2".to_owned(),
        }
    }

    #[test]
    fn path_join_collapses_every_run_of_slashes() {
        assert_eq!(path_join(&["a/", "/b", "c"]), "a/b/c");
        assert_eq!(path_join(&["/a//", "b/"]), "/a/b/");
        assert_eq!(path_join(&["https://x.y", "z"]), "https:/x.y/z");
        assert_eq!(path_join(&[]), "");
    }

    #[test]
    fn storage_prefix_uses_the_given_utc_date() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 9).expect("date");
        assert_eq!(get_storage_prefix_at("42", date), "public/ws/42/2026/01/09");
    }

    #[test]
    fn prefixes_without_a_date_use_today() {
        let today = date_prefix(today_utc());
        assert_eq!(get_storage_prefix("7"), format!("public/ws/7/{today}"));
        assert_eq!(
            get_channel_media_prefix(&location()),
            format!("public/zalo/1/2/{today}")
        );
        assert_eq!(
            get_mirrored_channel_media_prefix(&location()),
            format!("workspace/zalo/1/2/{today}")
        );
    }

    #[test]
    fn encode_uri_component_matches_js() {
        assert_eq!(encode_uri_component("a b#c"), "a%20b%23c");
        assert_eq!(encode_uri_component("-_.!~*'()"), "-_.!~*'()");
        assert_eq!(encode_uri_component("ễ/?&="), "%E1%BB%85%2F%3F%26%3D");
    }

    #[test]
    fn decode_uri_component_rejects_malformed_input() {
        assert_eq!(decode_uri_component("a%20b%E1%BB%85").unwrap(), "a bễ");
        assert!(decode_uri_component("100%").is_err());
        assert!(decode_uri_component("%zz").is_err());
        assert!(decode_uri_component("%FF").is_err());
    }

    #[test]
    fn build_copy_source_rejects_an_invalid_endpoint() {
        assert!(matches!(
            build_copy_source("a", "b", Some("not a url")),
            Err(FilesError::InvalidEndpoint(_))
        ));
        assert_eq!(build_copy_source("a", "b", Some("")).unwrap(), "b/a");
    }
}
