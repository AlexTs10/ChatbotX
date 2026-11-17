//! `lib/mobile-auth/member-route-allowlist.ts` (Q.A1 §3): the only `/v1`
//! operations a mobile (member) principal may call. Every other route fails
//! closed (403) before any read; each route here applies member scoping in
//! its handler (`handlers::member_scope`).

/// `MOBILE_MEMBER_ROUTES`, as `"METHOD /v1/path"` exactly as declared.
pub const MOBILE_MEMBER_ROUTES: &[&str] = &[
    "GET /v1/conversations",
    "GET /v1/conversations/{id}",
    "POST /v1/conversations/{id}/assign",
    "POST /v1/conversations/{id}/archive",
    "POST /v1/conversations/{id}/unarchive",
    "POST /v1/conversations/{id}/read",
    "POST /v1/conversations/{id}/unread",
    "POST /v1/conversations/{id}/follow",
    "POST /v1/conversations/{id}/unfollow",
    "POST /v1/conversations/{id}/enable-bot",
    "POST /v1/conversations/{id}/disable-bot",
    "GET /v1/conversations/{conversationId}/messages",
    "GET /v1/conversations/{conversationId}/messages/{messageId}",
    "POST /v1/conversations/{conversationId}/messages",
    "PUT /v1/conversations/{conversationId}/messages/{messageId}",
    "DELETE /v1/conversations/{conversationId}/messages/{messageId}",
    "POST /v1/conversations/{conversationId}/messages/{messageId}/attributes",
    "GET /v1/contacts",
    "GET /v1/contacts/{identifier}",
    "PATCH /v1/contacts/{identifier}",
    "DELETE /v1/contacts/{identifier}",
    "POST /v1/contacts/{identifier}/block",
    "POST /v1/contacts/{identifier}/unblock",
    "GET /v1/contacts/{identifier}/tags",
    "POST /v1/contacts/{identifier}/tags",
    "DELETE /v1/contacts/{identifier}/tags",
    "GET /v1/contacts/{identifier}/custom-fields",
    "PUT /v1/contacts/{identifier}/custom-fields/{idOrName}",
    "POST /v1/contacts/{identifier}/custom-fields/{customFieldId}",
    "DELETE /v1/contacts/{identifier}/custom-fields/{idOrName}",
    "GET /v1/contacts/{identifier}/sequences",
    "POST /v1/contacts/{identifier}/sequences",
    "DELETE /v1/contacts/{identifier}/sequences",
    "POST /v1/contacts/{identifier}/flows",
    "GET /v1/members",
    "GET /v1/teams",
    "GET /v1/flows",
    "GET /v1/sequences",
    "GET /v1/tags",
    "GET /v1/custom-fields",
    "GET /v1/saved-replies",
    "POST /v1/media-library/files/upload-url",
    "POST /v1/media-library/files",
    "GET /v1/realtime/connect-token",
];

/// `isMobileMemberRoute(method, path)` (a missing method is `POST`).
pub fn is_mobile_member_route(method: Option<&str>, path: Option<&str>) -> bool {
    let key = format!("{} {}", method.unwrap_or("POST"), path.unwrap_or(""));
    MOBILE_MEMBER_ROUTES.contains(&key.as_str())
}

/// [`is_mobile_member_route`] for a route id of the table.
pub fn is_mobile_member_route_id(route_id: &str) -> bool {
    crate::routes::route_by_id(route_id)
        .is_some_and(|route| is_mobile_member_route(Some(route.method), Some(route.path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_allowlisted_operation_is_a_declared_route() {
        for entry in MOBILE_MEMBER_ROUTES {
            let (method, path) = entry.split_once(' ').unwrap();
            assert!(
                crate::routes::PUBLIC_ROUTES
                    .iter()
                    .any(|r| r.method == method && r.path == path),
                "{entry} is not a route"
            );
        }
        assert_eq!(MOBILE_MEMBER_ROUTES.len(), 44);
    }

    #[test]
    fn unlisted_routes_and_a_missing_method_fail_closed() {
        assert!(is_mobile_member_route(Some("GET"), Some("/v1/contacts")));
        assert!(!is_mobile_member_route(Some("POST"), Some("/v1/contacts")));
        assert!(!is_mobile_member_route(None, Some("/v1/contacts")));
        assert!(!is_mobile_member_route(Some("GET"), None));
        assert!(!is_mobile_member_route_id("workspaceTokens.list"));
        assert!(is_mobile_member_route_id("conversations.list"));
    }
}
