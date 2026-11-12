//! `packages/utils/src/storage.ts`.

/// `getPublicFileUrl`: the object key joined under the public base URL
/// (its path, e.g. the bucket, kept); an absolute `http(s)` key is returned
/// as it is (normalized).
pub fn get_public_file_url(path: &str, base_url: &str) -> Result<String, url::ParseError> {
    let base = if base_url.ends_with('/') {
        base_url.to_owned()
    } else {
        format!("{base_url}/")
    };
    let base = url::Url::parse(&base)?;
    Ok(base.join(path.trim_start_matches('/'))?.to_string())
}
