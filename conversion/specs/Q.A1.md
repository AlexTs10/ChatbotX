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

## 3. Mobile member filters, then mobile tokens on

| TypeScript | Rust |
|---|---|
| `member-route-allowlist.ts` (`MOBILE_MEMBER_ROUTES`, 44) | `auth::mobile_member` (`MOBILE_MEMBER_ROUTES`, `is_mobile_member_route{,_id}`) |
| `resolve-mobile-principal.ts`, `resolve-mobile-workspace-principal.ts` | `LiveAuthDeps::{authenticate_mobile_bearer, resolve_mobile_principal}` |
| `middlewares/mobile-user-auth.ts` | `auth::authenticate_mobile_user` (`RouteAuth::MobileUser*`) |
| `conversation/member-scope.ts`, `contact/permission-scope.ts`, `flowService.assertEnabledInInbox` | `chatbotx_business::member_scope` |
| `lib/public-api/member-scope.ts` | `handlers::member_scope` (task-local `handlers::mobile_member()`) |
| `features/me/api/public.ts` + `device-tokens.ts` | `handlers::me` |

Fail closed twice: `resolve_mobile_principal` refuses a route outside the
allowlist (403 before the membership read, as the original), and
`dispatch_as` refuses a mobile principal on any other route id. The member
(user id, stored permissions, resolved conversation scope) rides on
`Principal.mobile`; every helper is a no-op without it (tokens, MCP, `/rpc`).

Where the scope applies (one place each, every caller routes through it):
`conversation_list::ListInput.member_scope` (list SQL, `none` = 404),
`conversations::conversation_json` (masking), the `find_or_fail` of each
conversation action, `messages::conversation` (every `messages.*` route),
`contacts::resolve` (`contactIdFor`: every `contacts.*/{identifier}` route
and sub-resource), `contacts::public_contact_json` (masking),
`contacts.list` (`ListScope::Member`), `contact_messages::send` and
`messages.create` (`assertMemberMayStartFlow`, sender attribution),
`flows.list` (`enableInInbox` without `flows`). `workspaceMembers.list`,
`inboxTeams.list`, `sequences.list`, `tags.list`, `customFields.list`,
`savedReplies.list`, `mediaLibrary.*`: workspace-level in the original too.
`realtime.getConnectToken` is not ported (501 for every caller).

Not ported (ponytail, attribution only, no data exposure): the `userId` /
`assignedBy` actor of the archive / follow / read / bot / assign actions.
Order: a member's out-of-scope conversation is 404 before body validation.

Mobile tokens are on: `crates/server` sets `LiveAuthDeps.mobile_audience`
(`<builder origin>/api`). Tests: `crates/api/tests/mobile_member_scope.rs`
(handler scope over a real DB + a signed JWT end to end through the
`/api/v1` router: allowlist, workspace header, membership, password change,
audience, `/v1/me*`), unit tests of `auth::mobile_member`.

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

## 5. Invitation start and user persistent menus

`GET /api/invitations/{code}/start` (`session::invitation_start`): per-IP
rate limit (`invitation-start-rate-limit`, 20 / 10 s, 100 for `unknown`),
302 to `/auth/sign-up?callbackURL=/invitations/<code>` with the
`chatbotx_invitation` cookie (HttpOnly, SameSite=lax, 10 min, `Secure` when
`BETTER_AUTH_URL` is https), else 302 to `/auth/sign-in?error=invalid_invitation`.
Test: `crates/api/tests/session_invitation_start.rs`. The public
`/v1/user-persistent-menus` routes were already ported
(`handlers::messenger_menus`, Q.I1b slice 5).

## State

- 2026-11-17: §1 done (8f1c7fdae); §2 and §4 done; §3 open (mobile tokens stay off).
- 2026-11-17: §3 done (member filters, `/v1/me*`, mobile tokens on); §5 done.
