# Q.D13a — inbound webhooks worker, tool screens, Google Sheets, response SLA

Final-round unit after Q.D13 (part 1) and Q.D9. Branch
`feat/conversion-r2-d13a`. Not in scope (the d13b unit): dynamic images,
decisions, leaderboards, the flow-click / dashboard listeners.

## Done

| # | Item | Rust | Web |
|---|------|------|-----|
| 1 | Inbound webhooks: `previewMapping` (`POST /v1/inbound-webhooks/preview-mapping`), the worker job `processInboundWebhookRequest` (claim lease, fenced checkpoints, contact resolve / create, mapped fields, the allowlisted actions), the blocked-owner skip, the 5-minute re-dispatch / 24-hour abandon cron | `business::inbound_webhook_apply` (extract, resolve inbox / contact, apply mapped fields), `business::inbound_webhook` (claim / checkpoint / complete / release / stale / abandon), `worker::inbound_webhook`, `worker::trigger::run_inbound_webhook_action` | |
| 1 | Inbound webhook screens: list, editor, analytics; the 10 queries / actions | `api::rpc_procedures::screens` | `routes/webhook-tools.tsx` (`inbound-webhooks`, `$id`, `$id/analytics`) |
| 2 | Outbound webhook screens: list (folder layout), edit page; `getWebhooks`, `findWebhook`, `webhookStoreSource` | `api::rpc_procedures::screens` | `routes/webhook-tools.tsx` (`webhooks`, `webhooks/$id/edit`) |
| 3 | Google Sheets: the 5 flow steps (`spreadsheetGetRow`, `…GetRandomRow`, `…SendData`, `…UpdateRow`, `…ClearRow`), the trigger action `runGoogleSheet`, the sheet-name check of create / update, `listWorksheets`, `listWorksheetHeaders` | `worker::spreadsheet`, `business::spreadsheet`, `api::handlers::spreadsheets`, `rest.rs` (`spreadsheetsAPI.*`) | |
| 3 | Google Sheets screens: `google-sheets` list, the Sheets settings page; `listSpreadsheets`, `findIntegrationGoogleSheet` | `api::rpc_procedures::screens` | `routes/webhook-tools.tsx` (`google-sheets`), `routes/settings.tsx` (`settings/integrations/google-sheets`) |
| 4 | Response SLA: `computeSnapshot` (working time, overrides, holidays, workspace zone), `trackWindow` on the window open (inbound message), the handoff reopen and the assignment; the SLA card on the inbox-teams page | `business::response_sla` (`add_working_seconds`, `compute_snapshot`, `track_window`), `conversation_actions::assign_one`, `worker::inbound`, `worker::steps::conversation`; `workingHoursListPage` carries `responseSla` | `routes/settings.tsx` (`SettingsInboxTeamsPage`) |
| 5 | Magic link screens: list, analytics; `listMagicLinks`, `findMagicLink` | `api::rpc_procedures::screens` | `routes/webhook-tools.tsx` (`magic-links`, `magic-links/$magicLinkId/analytics`) |

## Security

- Every inbound webhook query and action checks `superAdmin`
  (`assertCanManageInboundWebhooks`; the pages' `requireWorkspacePermission`
  guard, so the SPA reads carry it too). The writes run the public `/v1`
  routes, so the input schemas and the audit records are the same.
- The token is returned only to a super admin (`getInboundWebhookTokenAction`,
  the editor read); the public `get` still hides `endpointUrl` from
  read-only API tokens.
- The webhook reads (`getWebhooks`, `findWebhook`, `webhookStoreSource`)
  answer 404 to a member without `superAdmin` (the pages' `notFound()`);
  the magic link and Google Sheets reads need a member only, as their
  pages. Every read is bound to the request workspace.
- The Google Sheets calls go through `SafeHttp` (SSRF check at each
  redirect hop); the OAuth tokens stay in the encrypted integration row and
  never reach the SPA (`findIntegrationGoogleSheet` returns ids only).
- `runGoogleSheet` is trigger-only: an inbound webhook run is
  `unsupportedSource` (as `source.kind !== "trigger"`).
- The worker never runs an action type outside `inboundWebhookActionTypes`;
  a failed action stores a generic message, a failed request a sanitised
  one (max 1024 chars, `sanitize_public_text`).

## Decisions

See `DECISIONS.md` (2026-11-19 Q.D13a): the one-transaction contact
find-or-create, the page guard on the screen queries, the generic action
failure message, the injected Sheets transport, and `trackWindow` inside the
caller's write transaction.

## Tests

- `crates/business/tests/inbound_webhook_apply_payload.rs`, unit tests in
  `business::inbound_webhook_apply`.
- `crates/worker/tests/inbound_webhook_worker.rs`.
- `crates/api/tests/inbound_webhooks_public_api.rs` (`previewMapping`),
  `crates/api/tests/rpc_screen_actions.rs` (inbound actions; webhook, magic
  link and Sheets reads; the `responseSla` of `workingHoursListPage`).
- `crates/api/tests/webhooks_public_api.rs`,
  `crates/api/tests/magic_links_public_api.rs`,
  `crates/api/tests/spreadsheets_public_api.rs` (worksheet reads, URL check).
- `crates/worker/src/spreadsheet_tests.rs` and the unit tests in
  `worker::spreadsheet` (steps, lookup, write values, operators).
- `crates/business/tests/response_sla_window.rs` and the unit tests in
  `business::response_sla` (`addWorkingSeconds`, `computeSnapshot`,
  `trackWindow`, reassignment).

## Parity (not run here)

p1-09 `20`–`24` (inbound webhooks, `22` previewMapping); p1-08
`06-external-webhooks` and the webhooks suite; p1-11 `10-magic-links`; the
spreadsheets / Google Sheets suite; the response-SLA and working-hours
suites (conversation SLA columns after inbound messages and assignment).

## Not done

- `workingSecondsBetween` (the agent reply metric of
  `working-hours-duration.test.ts`): not in this unit (analytics); the
  `addWorkingSeconds` part of that test is ported.
- The Rust bulk assignment / call-agent claim paths do not exist, so
  `trackWindow` runs on the three Rust writes that open a window or change
  the assignee (inbound message, handoff reopen, `assign_one`).

- The cloud-only `ownerBlocked` answer of the intake (`403
  WORKSPACE_BLOCKED`): the Rust port has no owner quota state (same as the
  broadcast send). The worker does skip a blocked owner's queued jobs.
