//! The public oRPC API under `/api` (`apps/builder/src/app/api/[[...rest]]/route.ts`):
//! the route table, path matching, auth per route, and the "not ported yet"
//! answer for an operation that has no Rust handler yet.

mod table;

use std::sync::Arc;

use axum::Router;
use axum::http::{HeaderMap, Method, Uri};
use axum::response::Response;

pub use table::PUBLIC_ROUTES;

use crate::auth::WorkspaceTokenDeps;
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

/// The route of `method` + `path` (path under `/api`, no query). A literal
/// segment wins over a `{param}` segment at the same position.
pub fn match_route(method: &str, path: &str) -> Option<RouteMatch> {
    let _ = (method, path);
    todo!("P3.5b")
}

/// All routes of a source module, in table order.
pub fn routes_of_module(module: &str) -> Vec<&'static PublicRoute> {
    let _ = module;
    todo!("P3.5b")
}

/// The distinct scopes of the workspace-token routes of a module, in first-use
/// order (a multi-scope route counts as each of its scopes).
pub fn scopes_of_module(module: &str) -> Vec<&'static str> {
    let _ = module;
    todo!("P3.5b")
}

/// The route with this operation id.
pub fn route_by_id(id: &str) -> Option<&'static PublicRoute> {
    let _ = id;
    todo!("P3.5b")
}

/// `404 { defined: false, code: "NOT_FOUND", message: "Operation not ported yet" }`.
pub fn not_ported_error() -> OrpcError {
    todo!("P3.5b")
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
    let _ = (deps, method, uri, headers);
    todo!("P3.5b")
}

/// The router of the public API: a fallback that serves every `/api/...` path
/// no other route took.
pub fn public_api_router<D: WorkspaceTokenDeps + Send + Sync + 'static>(deps: Arc<D>) -> Router {
    let _ = deps;
    todo!("P3.5b")
}
