# Q.A1 — auth rest from P3.5f (porter spec)

Unit (TRACKER Q.A1; P3.5f "left" list). Order: (1) public `inboxes.*`,
(2) MCP OAuth server, (3) mobile-token member filters, then mobile tokens on,
(4) rate limits.

## 1. Public `inboxes.*`

The original `features/inboxes/api/public.ts` has three routes only:
`inboxes.list`, `inboxes.update`, `inboxes.listChannels` (no `inboxes.get`).
All three are in 8f1c7fdae. Done.

## 2. MCP OAuth server

Source: better-auth `oauthProvider()` / `mcp()` as
`packages/auth/src/mcp-config.ts` configures them, the builder routes
`app/.well-known/*`, `app/oauth/{sign-in,consent}`,
`features/mcp-oauth/*` and `packages/business/src/mcp-oauth/service.ts`.

| TypeScript | Rust |
|---|---|
| `/.well-known/oauth-authorization-server`, `/.well-known/openid-configuration` | `session::mcp_oauth` `server_metadata` |
| `/.well-known/oauth-protected-resource/[...path]` | `protected_resource` |
| `POST /api/auth/oauth2/register` (unauthenticated DCR, `mcp-client-registration` 20 / window / IP) | `register` |
| `GET /api/auth/oauth2/authorize` (code, PKCE S256, signed login / consent query) | `authorize` |
| `POST /api/auth/oauth2/token` (`authorization_code`, `refresh_token`, 30 s rotation replay, family revoke) | `token` |
| `POST /api/auth/oauth2/consent` | 404 (consent only through the action, as the original hook) |
| `GET /oauth/sign-in` | `oauth_sign_in` |
| `submitOAuthConsentAction`, consent page data | `/rpc/authActions/submitOAuthConsent`, `/rpc/authActions/oauthConsentPage` |
| `revokeMcpOAuthGrantAction`, settings page `listGrantsForUser` | `/rpc/workspaceActions/revokeMcpOAuthGrant`, `/rpc/workspaceActions/listMcpOAuthGrants` |
| `mcpOAuthService` | `chatbotx_business::mcp_oauth` |
| `verifyMcpAccessToken` + `resolvePrincipal` in the `/v1` middleware | `LiveAuthDeps::{verify_mcp_access_token, resolve_oauth_principal}` |

Security rules (all tested in `crates/api/tests/session_mcp_oauth.rs`):
codes and refresh tokens and client secrets are stored as SHA-256
base64url hashes; a code is consumed in one write transaction on first use
(also on a failed PKCE check), and a replay deletes the tokens it issued;
PKCE S256 only, required for public clients and `require_pkce` (registration
with `require_pkce: false` is refused); a redirect URI must equal a registered
one (only the loopback port may differ, as the original); the token request
must repeat it; the signed login / consent query is HMAC-SHA256 with
`BETTER_AUTH_SECRET`, compared in constant time; consent is stored for the
session user and bound to a workspace the user administers
(`assertAuthorizableWorkspace` before the write, `bindConsentWorkspace`
after); a refresh token is bound to its client. Access tokens are EdDSA JWTs
(not stored), audience = the MCP resource URL.

Also: the session-cookie signature check (`verify_signed_cookie`) now
compares in constant time.

Not ported (ponytail, no MCP client needs them): request objects,
`claims`, `max_age`, DPoP, `private_key_jwt`, client credentials,
introspection / revocation / userinfo / end-session endpoints (the metadata
lists them as better-auth does; the MCP server app introspects through
`GET /v1/token`), Client ID Metadata Documents, per-client resources,
support-access sessions.

Tests: `crates/api/tests/session_mcp_oauth.rs` (8, Rust-native: the TS side
has no route test of the plugin), `crates/business/tests/mcp_oauth_service.rs`
(port of `packages/business/__tests__/mcp-oauth.service.test.ts`; not
ported: synthetic support membership, mocked-logger assertions),
`crates/api/tests/rpc_workspace.rs` (port of
`apps/builder/__tests__/mcp-oauth-revoke-action.test.ts`).

## 3. Mobile member filters (not done)

The original allows a mobile (member) principal only on the 45 routes of
`apps/builder/src/lib/mobile-auth/member-route-allowlist.ts`; every other
`/v1` route is 403 before any read. Each allowed handler applies
`lib/public-api/member-scope.ts` (conversation chat scope, contact
permission scope + team restriction, email / phone masking, sender
attribution, flow start check). No Rust `/v1` handler reads
`WorkspaceTokenContext.mobile` yet, so `LiveAuthDeps::mobile_api_audience`
stays `None` (mobile JWTs on `/v1` are refused as invalid tokens). Turning
them on needs: the allowlist gate in `resolve_mobile_principal`, member
scoping in each of the 45 handlers, `/v1/me*`, and the ports of
`mobile-member-allowlist`, `mobile-contacts-member-scope`,
`mobile-conversations-member-scope`, `mobile-jwt-middleware`,
`mobile-user-auth-middleware`, `mobile-me-api`.

## 4. Rate limits

`session::rate_limit::auth_rate_limit` (axum middleware on every
`/api/auth/*` route in `crates/server/src/main.rs`): better-auth's built-in
limiter (10 s window, key = client IP + path, 100 requests; 3 for
`/sign-in*`, `/sign-up*`, `/change-password*`, `/change-email*` and the
`mobileAuth` rule `/mobile/token`; 429 `{ message }` + `X-Retry-After`) and
the builder's `rateLimitMobilePasswordGrant` (5 password grants / 10 s per
lower-cased email, 429 + `Retry-After`; unknown grant 400). Always on
(better-auth enables it in production). Plus `mcp-client-registration`
(20 / 10 s per IP) on `/oauth2/register`. Tests:
`crates/api/tests/session_auth_rate_limit.rs` (port of the main cases of
`apps/builder/__tests__/mobile-password-grant-rate-limit.test.ts`) and the
unit tests of the module.

## State

- 2026-11-17: §1 done (8f1c7fdae); §2 and §4 done; §3 open (mobile tokens stay off).
