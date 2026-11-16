//! The SPA (`apps/web/dist`, P4): static files, and `index.html` for every
//! other GET that no route matched (client-side routing). API paths and
//! other methods go on to the public API fallback, as before.
//!
//! `index.html` gets the public client env (`NEXT_PUBLIC_*`) as
//! `window.__ENV` in place of the `<!--chatbotx-env-->` marker.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::http::{Method, header};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt as _;

/// Paths that belong to the API: never the SPA.
const API_PREFIXES: &[&str] = &["/api/", "/rpc/", "/files/", "/realtime"];

struct Web {
    dir: PathBuf,
    index: String,
    /// The public API router (itself a fallback for every `/api/...` path).
    api: Router,
}

/// The fallback router of the SPA in `dir` in front of `api`, or `None`
/// when `dir` has no `index.html` (API-only server).
pub fn router(dir: PathBuf, api: Router) -> Option<Router> {
    let index = std::fs::read_to_string(dir.join("index.html")).ok()?;
    let index = index.replace("<!--chatbotx-env-->", &env_script());
    let web = Arc::new(Web { dir, index, api });
    Some(Router::new().fallback(move |request: Request| serve(Arc::clone(&web), request)))
}

/// `<script>window.__ENV={...}</script>` of the `NEXT_PUBLIC_*` variables.
fn env_script() -> String {
    let env: serde_json::Map<String, serde_json::Value> = std::env::vars()
        .filter(|(key, _)| key.starts_with("NEXT_PUBLIC_"))
        .map(|(key, value)| (key, serde_json::Value::String(value)))
        .collect();
    // `<` escaped: a value cannot close the script element.
    let json = serde_json::Value::Object(env)
        .to_string()
        .replace('<', "\\u003c");
    format!("<script>window.__ENV={json}</script>")
}

async fn serve(web: Arc<Web>, request: Request) -> Response {
    let path = request.uri().path().to_string();
    let path = path.as_str();
    if !matches!(*request.method(), Method::GET | Method::HEAD)
        || API_PREFIXES.iter().any(|prefix| path.starts_with(prefix))
    {
        return match web.api.clone().oneshot(request).await {
            Ok(response) => response,
            Err(never) => match never {},
        };
    }
    let file = safe_path(&web.dir, path);
    let bytes = match &file {
        Some(file) => tokio::fs::read(file).await.ok(),
        None => None,
    };
    if let (Some(file), Some(bytes)) = (file, bytes) {
        let cache = if path.starts_with("/assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "public, max-age=3600"
        };
        return (
            [
                (header::CONTENT_TYPE, content_type(&file)),
                (header::CACHE_CONTROL, cache),
            ],
            bytes,
        )
            .into_response();
    }
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        web.index.clone(),
    )
        .into_response()
}

/// `dir` + the URL path, when the path has only plain segments.
fn safe_path(dir: &Path, url_path: &str) -> Option<PathBuf> {
    let relative = Path::new(url_path.trim_start_matches('/'));
    if url_path.contains('\\')
        || relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return None;
    }
    Some(dir.join(relative))
}

fn content_type(file: &Path) -> &'static str {
    match file.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "txt" => "text/plain; charset=utf-8",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_keeps_plain_segments_only() {
        let dir = Path::new("/srv/web");
        assert_eq!(
            safe_path(dir, "/assets/app.js"),
            Some(PathBuf::from("/srv/web/assets/app.js"))
        );
        assert_eq!(safe_path(dir, "/"), None);
        assert_eq!(safe_path(dir, "/../etc/passwd"), None);
        assert_eq!(safe_path(dir, "/assets/../../x"), None);
        assert_eq!(safe_path(dir, "/a\\..\\b"), None);
    }

    #[test]
    fn env_script_escapes_lt() {
        assert!(!env_script().contains("</script><"));
    }
}
