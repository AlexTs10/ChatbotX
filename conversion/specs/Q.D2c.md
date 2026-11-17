# Q.D2c — connections rest

Branch `feat/conversion-d2c` (from `conversion` at `d78d8db3b`). Follows
P3.6-D2 (`specs/P3.6-D2.md`).

## Slices

| # | Slice | Rust | Original |
|---|-------|------|----------|
| 1 | Public `integrations.*` (`/v1/integrations`, `/v1/integrations/ai/{provider}` get / connect / update / disconnect) | `business::integration`, `integration_ai_provider::disconnect_legacy`, `api::handlers::integrations_ai` | `features/integrations/api/public/*.ts` |
| 2 | `connectSessions.*` (get / cancel / connectTargets), OAuth start on `connections.create` / `reconnect`, `GET /integrations/{provider}/callback` | `business::connect_session`, `business::connection::oauth`, `api::handlers::connect_sessions`, `api::connect_callback` | `packages/connections/src/connect-session*.ts`, `connect-targets.ts`, `app/integrations/[provider]/callback` |
| 3 | `channels.typing` / `markRead` / `deliveryStatus` (send was P3.6-D2); worker `messageStatus` | `api::handlers::channels`, `worker::channels::api` | `features/channels/api/public.ts` |
| 4 | `commentAutomations.*` (listAccounts, processMissedComments, getMissedCommentRun, listModerationLogs, getModerationSummary) | `business::comment_automation`, `api::handlers::comment_automations` | `features/shared/comment-automation/*` |
| 5 | `contactScans.*` (create, getStatus, list) | `business::contact_scan`, `api::handlers::contact_scans` | `packages/business/src/contact-scan/service.ts` |
| 6 | Email-marketing API-key connects + verify (activeCampaign, drip, getResponse, klaviyo, mailchimp, mailerLite, moosend, sendGrid) | `business::connection::api_key` (shares `credentials::persist_key_connection`) | `integrations/<name>/src/integration.ts` `apiKeyConnection` |
| 7 | `connection/state.ts` state machine + its tests | `business::connection::state` | `packages/business/src/connection/state.ts` |

## Security

- OAuth `state` = `{sessionId}.{nonce}`; only the SHA-256 of the 32-byte
  random nonce is stored. The callback finds the session by the nonce hash
  and then checks the session id and the provider; any mismatch is 404.
- The session is bound to the workspace and the actor (user or token);
  `connectSessions.*` checks the token scope after the workspace check.
- `returnUrl` / `redirect`: only the app origin is kept (relative path +
  search + hash); any other origin is dropped. No open redirect.
- The session's provider auth (`encryptedAuth`) is encrypted with the
  existing `Encryption` (`ENCRYPTION_KEY`) and the original AAD
  (`connect-session:{id}` / `connect-session:{id}:authorization`); the
  platform OAuth app credential is decrypted with its owner-bound AAD.

## Decisions

See `conversion/DECISIONS.md` (2026-11-17 "Q.D2c" lines): messenger is the
only OAuth adapter; no Graph retry; no `originHost` relay; no legacy
JSON-state callback; the deliveryStatus `messageId` / `messageSourceId`
quirk is kept; no low-queue worker (contactMarkAsRead, replayMissedComment)
and no contact-scan worker (Q.W2); messenger-only missed-comment scan;
contactScans `sort` ignored; email-marketing probes check only the fields
they decide on.

## Tests

- `crates/api/tests/integrations_public_api.rs` (new), `connections_public_api.rs`,
  `integrations_ai_public_api.rs`, `routes_channel_and_mobile_auth.rs`.
- `crates/business/tests/connection_credentials.rs` (+ `allow_update`).
- Unit tests: `connect_callback` (state split, error query), `comment_automation`
  (Graph time), `contact_scan` (availability), `connection::state` (port of
  `state.test.ts`), `connection::api_key` (auth, probe requests, failure map).
- Parity fixtures: `parity/stacks/rust/ext/p1-05.ts` (place id 10),
  `p1-10.ts` (place id 13).

## Parity (Rust stack, Dell, 2026-11-17)

p1-05 `00`, `05`–`08` + p1-10 `00`, `30`, `31`: 452 / 454 pass. The 2 failures
need workers that are not ported (Q.W2): `07` "queues unhandled recent
comments; the worker posts the public reply" (low-queue
`replayMissedComment`) and `08` "schedules a run (202); the worker walks the
conversations" (contact-scan worker; the API part passes: the run stays
`init`). p1-02 `ai-files`: the setup (AI key connect) now passes; the 7 tests
that wait for chunks / embeddings time out (heavy worker, Q.W2), the other
tests pass.

## Not done

- `connection/state-service.ts` as one DB service (the Rust writes keep their
  inline status SQL).
- OAuth adapters other than messenger; the low-queue and contact-scan
  workers; missed-comment scans for instagram / threads / tiktok.
