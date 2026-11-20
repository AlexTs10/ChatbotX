//! The public API document and its docs page (Q.P6-U5, COVERAGE C11, C12):
//! `GET|HEAD /api/public-spec.json` (`app/api/public-spec.json/route.ts`) and
//! `GET /api/scalar-ui` (`app/api/scalar-ui/route.ts`).
//!
//! The original generates the OpenAPI document from the zod schemas of the
//! public router at run time. Rust has no zod: `assets/public-spec.json.gz`
//! is that document, generated once from the original
//! (`conversion/tools/public-spec-gen/gen.sh`), with `__TENANT_NAME__` /
//! `__SERVER_URL__` placeholders that each request fills in. Regenerate it
//! when a public route or schema changes.

use std::collections::HashMap;
use std::io::Read as _;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use chatbotx_core::utils::request::{
    RequestParts, force_public_https, get_public_origin_from_request,
};
use chatbotx_db::rusqlite::OptionalExtension as _;
use serde_json::{Value, json};
use sha1::Digest as _;

use crate::handlers::ApiState;

/// `TTL_SECONDS`.
const TTL: Duration = Duration::from_secs(300);
const CACHE_CONTROL: &str = "public, max-age=300, stale-while-revalidate=3600";

/// The generated document (gzip).
static SPEC_GZ: &[u8] = include_bytes!("../assets/public-spec.json.gz");

/// One cached document (`CachedSpec`).
struct Cached {
    body: Arc<String>,
    etag: String,
    at: Instant,
}

/// The state of the routes: the db (tenant name) and the document cache,
/// keyed by (tenant name, origin, filter).
pub struct SpecState {
    api: Arc<ApiState>,
    cache: Mutex<HashMap<String, Arc<Cached>>>,
}

/// The two routes.
pub fn router(api: Arc<ApiState>) -> Router {
    Router::new()
        .route("/api/public-spec.json", get(spec))
        .route("/api/scalar-ui", get(scalar_ui))
        .with_state(Arc::new(SpecState {
            api,
            cache: Mutex::new(HashMap::new()),
        }))
}

/// The parsed generated document.
fn template() -> &'static Value {
    static DOC: OnceLock<Value> = OnceLock::new();
    DOC.get_or_init(|| {
        let mut text = String::new();
        flate2::read::GzDecoder::new(SPEC_GZ)
            .read_to_string(&mut text)
            .expect("assets/public-spec.json.gz is gzip");
        serde_json::from_str(&text).expect("assets/public-spec.json.gz is JSON")
    })
}

/// `buildSpecDocument`: the title, the server and the `filter` (operations
/// whose path starts with `/v1/<filter>`).
pub fn build_document(tenant_name: &str, origin: &str, filter: Option<&str>) -> String {
    let mut doc = template().clone();
    doc["info"]["title"] = json!(tenant_name);
    doc["servers"] = json!([{ "url": format!("{}/api", origin.trim_end_matches('/')) }]);
    if let Some(filter) = filter
        && let Some(paths) = doc.get_mut("paths").and_then(Value::as_object_mut)
    {
        let prefix = format!("/v1/{filter}");
        paths.retain(|path, _| path.starts_with(&prefix));
    }
    doc.to_string()
}

/// `computeEtag`: the quoted SHA-1 hex of the body.
fn etag_of(body: &str) -> String {
    let digest = sha1::Sha1::digest(body.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("\"{hex}\"")
}

/// `getTenantSettings().name` (one tenant: the root tenant's name).
async fn tenant_name(api: &ApiState) -> String {
    api.db
        .read(|conn| {
            Ok(conn
                .query_row(
                    r#"SELECT "name" FROM "Tenant" WHERE "id" = ?1"#,
                    [chatbotx_business::tenant::ROOT_TENANT_ID],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten())
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "ChatbotX".to_string())
}

async fn spec(
    State(state): State<Arc<SpecState>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let url = format!("http://{host}/api/public-spec.json");
    let origin = get_public_origin_from_request(
        &RequestParts {
            url: &url,
            headers: &headers,
        },
        force_public_https(&chatbotx_core::env::ProcessEnv),
    );
    let filter = url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(key, _)| key == "filter")
        .map(|(_, value)| value.into_owned())
        .filter(|f| !f.is_empty());
    let tenant = tenant_name(&state.api).await;
    let key = format!(
        "{tenant}::{origin}::{}",
        filter.as_deref().unwrap_or_default()
    );
    let fresh = state
        .cache
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key)
        .filter(|c| c.at.elapsed() < TTL)
        .cloned();
    let cached = match fresh {
        Some(cached) => cached,
        None => {
            let body = build_document(&tenant, &origin, filter.as_deref());
            let entry = Arc::new(Cached {
                etag: etag_of(&body),
                body: Arc::new(body),
                at: Instant::now(),
            });
            let mut cache = state.cache.lock().unwrap_or_else(PoisonError::into_inner);
            cache.retain(|_, c| c.at.elapsed() < TTL);
            cache.insert(key, Arc::clone(&entry));
            entry
        }
    };
    let etag = HeaderValue::from_str(&cached.etag).unwrap_or_else(|_| HeaderValue::from_static(""));
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(cached.etag.as_str())
    {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (
                    header::CACHE_CONTROL,
                    HeaderValue::from_static(CACHE_CONTROL),
                ),
            ],
        )
            .into_response();
    }
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            ),
            (header::ETAG, etag),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(CACHE_CONTROL),
            ),
        ],
        cached.body.as_str().to_owned(),
    )
        .into_response()
}

/// The Scalar API reference page over `/api/public-spec.json`, as the original.
async fn scalar_ui() -> Response {
    const PAGE: &str = r#"
    <!doctype html>
    <html>
      <head>
        <title>My Client</title>
        <meta charset="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <link rel="icon" type="image/svg+xml" href="https://orpc.dev/icon.svg" />
      </head>
      <body>
        <div id="app"></div>

        <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
        <script>
          Scalar.createApiReference('#app', {
            url: '/api/public-spec.json',
            authentication: {
              securitySchemes: {
                bearerAuth: {
                  token: 'default-token',
                },
                developerAccessToken: {
                  token: 'default-workspace-token',
                },
              },
            },
          })
        </script>
      </body>
    </html>
  "#;
    ([(header::CONTENT_TYPE, "text/html")], PAGE).into_response()
}
