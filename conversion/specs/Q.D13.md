# Q.D13 — rest (first part)

Branch `feat/conversion-d13` (from `conversion` at `e2d16a491`, merged again
for Q.F2). Follows P3.6-D2.

## Slices

| # | Slice | Rust | Original |
|---|-------|------|----------|
| 1 | Saved replies (`/v1 savedReplies.*`, actions, `listSavedRepliesAuthorizedAPI`) | `business::saved_reply`, `api::handlers::saved_replies` | `features/saved-replies/*`, `packages/business/src/saved-reply` |
| 2 | Response SLA (`/v1 responseSla.*`, `updateResponseSlaAction`, `getResponseSlaStatusWorkspaceAuthAPI`) | `business::response_sla`, `api::handlers::response_sla` | `features/response-sla/*` |
| 3 | Webhook (conditions) CRUD actions | `business::webhook`, `api::handlers::{webhooks,conditions}` | `features/webhooks/*` |
| 4 | Working hours and holidays (`/v1 workingHours.*`, 7 actions, options procedure) | `business::working_hours`, `api::handlers::working_hours`, `db::repo::partials::working_hours` (date ranges, local date) | `packages/business/src/working-hours/*`, `features/working-hours/*` |
| 5 | Spreadsheets CRUD (`/v1 spreadsheets.*`, 3 actions, list procedure) | `business::spreadsheet`, `api::handlers::spreadsheets` | `packages/business/src/spreadsheet/service.ts`, `features/spreadsheets/*` |
| 6 | Magic links CRUD (`/v1 magicLinks.*`, 3 actions) and `GET /r/{workspaceId}/{name}` | `business::magic_link`, `api::handlers::magic_links`, `api::magic_link_redirect` | `packages/business/src/magic-link/service.ts`, `app/r/[workspaceId]/[name]/route.ts` |
| 7 | External webhooks (`/v1 externalWebhooks.list/create/delete`) | `business::external_webhook`, `api::handlers::external_webhooks` | `features/external-webhooks/*` |
| 8 | Inbound webhooks: `/v1 inboundWebhooks.*` (list, create, get, update, updateSettings, regenerateToken, listRequests, getRequest, delete, deleteMany), `inboundWebhookChannels.list`, `POST /api/hooks/{token}` intake | `business::inbound_webhook`, `api::handlers::inbound_webhooks`, `api::inbound_webhook_intake` | `features/inbound-webhooks/{api,lib,schema}`, `packages/business/src/inbound-webhook/service.ts`, `app/api/hooks/[token]/route.ts` |
| 9 | Flow-editor reads: `decisionProfilesAPI.listActiveForFlow`, `dynamicImagesAPI.getDynamicImageAPI`, `leaderboardsAPI.listLeaderboardOptionsAPI` | `api::rpc_procedures::rest::flow_reads` | `features/{decisions,dynamic-images,leaderboards}/api/*` |

Persistent menus and `schemas.flowSpec` were already ported
(`messenger_menus.rs`, `capabilities.rs`, Q.F2 `user_persistent_menus.rs`).

## Security

- `/v1` routes keep the original token scopes; `/rpc` actions use the shared
  membership gate; `listActiveForFlow` needs the `flows` permission (403).
- External webhook register: `ssrf_guard::assert_public_url` on the URL
  (422 `invalidRequestData`), max 50 per workspace, idempotent per
  (event, url). No outbound fetch in this unit.
- Spreadsheet URLs: only `https://docs.google.com/spreadsheets/d/<id>/…` is
  accepted (no other host, scheme or port); no fetch (Q.I4).
- Inbound webhook intake: the token shape is checked before any lookup;
  per-IP (300 / 10 s, unknown IP 1500) and per-webhook (600 / 10 s) limits
  on the kv store; a 256 KiB body cap (declared or streamed) before the
  token lookup; the token, URL and payload are never logged. The endpoint
  URL is not returned to a read-only token (`get`). Config saves check that
  inboxes, custom fields and action references belong to the workspace.
- `/r`: the workspace must be servable (410 otherwise); a button `code` is
  attributed only when the contact inbox's conversation is in the link's
  workspace.

## Decisions

See `conversion/DECISIONS.md` (2026-11-17 "Q.D13" lines).

## Tests

- New: `crates/api/tests/{inbound_webhooks_public_api,saved_replies_sla_public_api,working_hours_public_api,spreadsheets_public_api,magic_link_redirect,external_webhooks_public_api}.rs`.
- `crates/api/tests/rpc_spa_procedures.rs`: `saved_reply_sla_and_rest_actions_are_wired`.
- Unit: `business::spreadsheet` (URL parse), `api::inbound_webhook_intake` (token shape).

## Parity (not run here)

p1-11 `10-magic-links`, p1-08 `06-external-webhooks`, p1-09 `20`, `21`,
`23`, `24` (inbound webhooks; `22` previewMapping and the worker parts will
fail), plus the saved-reply,
response-SLA and working-hours suites.

## Not done

- Inbound webhooks: `previewMapping`, the worker job and the re-dispatch
  cron are done in Q.D13a (`specs/Q.D13a.md`).
- Dynamic images CRUD and rendering (needs a canvas / font renderer that
  the Rust workspace does not have); decisions CRUD; leaderboards (~3.3k TS)
  and the reaction leaderboard award hook.
- `analytics:dashboard` and `flow:clicked` listeners (no flow / dashboard bus
  listener in Rust yet, Q.W2).
- Google Sheets reads (`listWorksheets`, headers, `listSheetNames` check),
  the Sheets flow steps and `runGoogleSheet`: done in Q.D13a.
- Web: the webhook, magic link, Google Sheets and inbox-teams (response
  SLA) screens are done in Q.D13a; the dynamic image, decision and
  leaderboard screens are the d13b unit.
