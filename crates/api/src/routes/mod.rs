//! The public oRPC API under `/api` (`apps/builder/src/app/api/[[...rest]]/route.ts`):
//! the route table, path matching, auth per route, and the "not ported yet"
//! answer for an operation that has no Rust handler yet.

mod table;

use std::sync::{Arc, OnceLock};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

pub use table::PUBLIC_ROUTES;

use crate::auth::{TokenAuthRequest, WorkspaceTokenDeps, authorize_workspace_token_for_scope};
use crate::orpc::OrpcError;

/// The auth chain of an operation (the oRPC builder it is declared with).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteAuth {
    /// `workspaceTokenAuthAPIForScope(scope)`; any of the scopes opens the route.
    WorkspaceToken(&'static [&'static str]),
    /// `channelApiTokenAPI` (the API channel's own token).
    ChannelToken,
    /// `mobileUserAPI` (mobile access JWT, no workspace).
    MobileUser,
    /// `mobileUserPasswordChangeAPI` (same, also while `mustChangePassword` is set).
    MobileUserPasswordChange,
}

/// One operation of `publicRouter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicRoute {
    /// The OpenAPI `operationId` (router key path, e.g. `contacts.list`).
    pub id: &'static str,
    /// Upper-case HTTP method.
    pub method: &'static str,
    /// oRPC path template under `/api` (e.g. `/v1/contacts/{id}`).
    pub path: &'static str,
    pub auth: RouteAuth,
    /// The builder source module that declares the route (`features/.../api/public`).
    pub module: &'static str,
}

/// A matched route and its path parameters (percent-decoded).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteMatch {
    pub route: &'static PublicRoute,
    pub params: Vec<(&'static str, String)>,
}

/// The message of an operation that is in the table but has no Rust handler yet.
pub const NOT_PORTED_MESSAGE: &str = "Operation not ported yet";

/// The path segments of every route template, in table order.
fn split_templates() -> &'static [Vec<&'static str>] {
    static SPLIT: OnceLock<Vec<Vec<&'static str>>> = OnceLock::new();
    SPLIT.get_or_init(|| {
        PUBLIC_ROUTES
            .iter()
            .map(|route| route.path.split('/').collect())
            .collect()
    })
}

fn param_name(segment: &str) -> Option<&str> {
    segment.strip_prefix('{')?.strip_suffix('}')
}

/// Percent-decoding of one path segment (`decodeURIComponent`; `+` stays).
fn decode_segment(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = |at: usize| {
            bytes
                .get(at)
                .and_then(|b| char::from(*b).to_digit(16))
                .and_then(|d| u8::try_from(d).ok())
        };
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (hex(index + 1), hex(index + 2))
        {
            out.push(high * 16 + low);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The route of `method` + `path` (path under `/api`, no query). A literal
/// segment wins over a `{param}` segment at the same position.
pub fn match_route(method: &str, path: &str) -> Option<RouteMatch> {
    let trimmed = if path.len() > 1 {
        path.trim_end_matches('/')
    } else {
        path
    };
    let segments: Vec<&str> = trimmed.split('/').collect();
    let mut best: Option<(usize, Vec<bool>)> = None;
    for (index, template) in split_templates().iter().enumerate() {
        let route = &PUBLIC_ROUTES[index];
        if !route.method.eq_ignore_ascii_case(method) || template.len() != segments.len() {
            continue;
        }
        let mut literals = Vec::with_capacity(template.len());
        let fits = template.iter().zip(&segments).all(|(part, segment)| {
            if param_name(part).is_some() {
                literals.push(false);
                !segment.is_empty()
            } else {
                literals.push(true);
                part == segment
            }
        });
        if fits && best.as_ref().is_none_or(|(_, current)| literals > *current) {
            best = Some((index, literals));
        }
    }
    let (index, _) = best?;
    let route = &PUBLIC_ROUTES[index];
    let params = split_templates()[index]
        .iter()
        .zip(&segments)
        .filter_map(|(part, segment)| param_name(part).map(|name| (name, decode_segment(segment))))
        .collect();
    Some(RouteMatch { route, params })
}

fn module_matches(route_module: &str, module: &str) -> bool {
    route_module == module
        || route_module
            .strip_prefix(module)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// All routes of a source module (or of its sub-modules), in table order.
pub fn routes_of_module(module: &str) -> Vec<&'static PublicRoute> {
    PUBLIC_ROUTES
        .iter()
        .filter(|route| module_matches(route.module, module))
        .collect()
}

/// The distinct scopes of the workspace-token routes of a module, in first-use
/// order (a multi-scope route counts as each of its scopes).
pub fn scopes_of_module(module: &str) -> Vec<&'static str> {
    let mut scopes: Vec<&'static str> = Vec::new();
    for route in routes_of_module(module) {
        if let RouteAuth::WorkspaceToken(route_scopes) = route.auth {
            for scope in route_scopes {
                if !scopes.contains(scope) {
                    scopes.push(scope);
                }
            }
        }
    }
    scopes
}

/// The route with this operation id.
pub fn route_by_id(id: &str) -> Option<&'static PublicRoute> {
    PUBLIC_ROUTES.iter().find(|route| route.id == id)
}

/// `404 { defined: false, code: "NOT_FOUND", message: "Operation not ported yet" }`.
pub fn not_ported_error() -> OrpcError {
    let mut error = OrpcError::new("NOT_FOUND", Some(404), Some(NOT_PORTED_MESSAGE));
    error.defined = false;
    error
}

/// The original handler's answer to a path no procedure serves.
fn plain_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/plain;charset=UTF-8")],
        "Not found",
    )
        .into_response()
}

/// Answer one request under `/api`: match the route, run its auth, then the
/// handler (none are ported yet: [`not_ported_error`]). An unknown route is a
/// plain-text `404 Not found`, like the original handler.
pub async fn handle_public_request<D: WorkspaceTokenDeps>(
    deps: &D,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
) -> Response {
    let Some(path) = uri
        .path()
        .strip_prefix("/api")
        .filter(|p| p.starts_with('/'))
    else {
        return plain_not_found();
    };
    let Some(matched) = match_route(method.as_str(), path) else {
        return plain_not_found();
    };
    let route = matched.route;
    let mut res_headers = HeaderMap::new();
    let outcome = match route.auth {
        RouteAuth::WorkspaceToken(scopes) => {
            let url = uri.to_string();
            authorize_workspace_token_for_scope(
                deps,
                TokenAuthRequest {
                    headers,
                    url: Some(&url),
                    method: Some(route.method),
                    path: Some(route.path),
                },
                scopes,
                &mut res_headers,
            )
            .await
            .map(|_| ())
        }
        // The channel token (`channelApiTokenAPI`) and the mobile user JWT
        // (`mobileUserAPI`) are ported with their domains (P3.5e / P3.6+).
        RouteAuth::ChannelToken | RouteAuth::MobileUser | RouteAuth::MobileUserPasswordChange => {
            Ok(())
        }
    };
    let mut response = match outcome {
        Ok(()) => not_ported_error().into_response(),
        Err(error) => error.into_response(),
    };
    response.headers_mut().extend(res_headers);
    response
}

async fn public_api_fallback<D: WorkspaceTokenDeps + Send + Sync + 'static>(
    State(deps): State<Arc<D>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Response {
    handle_public_request(deps.as_ref(), &method, &uri, &headers).await
}

/// The router of the public API: a fallback that serves every `/api/...` path
/// no other route took.
pub fn public_api_router<D: WorkspaceTokenDeps + Send + Sync + 'static>(deps: Arc<D>) -> Router {
    Router::new()
        .fallback(public_api_fallback::<D>)
        .with_state(deps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_literal_segment_wins_over_a_param_segment() {
        let matched = match_route("GET", "/v1/contacts/count").expect("route");
        assert_eq!(matched.route.id, "contacts.count");
        let matched = match_route("GET", "/v1/contacts/id%3A1").expect("route");
        assert_eq!(matched.route.id, "contacts.get");
        assert_eq!(matched.params, vec![("identifier", "id:1".to_string())]);
    }

    #[test]
    fn an_unknown_path_or_method_matches_nothing() {
        assert!(match_route("GET", "/v1/no-such-thing").is_none());
        assert!(match_route("PATCH", "/v1/ads/referral-stats").is_none());
    }

    #[test]
    fn every_route_matches_its_own_template_and_ids_are_unique() {
        let mut ids: Vec<&str> = PUBLIC_ROUTES.iter().map(|route| route.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), PUBLIC_ROUTES.len());
        for route in PUBLIC_ROUTES {
            let concrete: Vec<&str> = route
                .path
                .split('/')
                .map(|s| if param_name(s).is_some() { "x" } else { s })
                .collect();
            let matched = match_route(route.method, &concrete.join("/")).expect(route.id);
            assert_eq!(matched.route.method, route.method);
        }
    }
}
