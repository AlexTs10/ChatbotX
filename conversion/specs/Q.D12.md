# Q.D12 — WhatsApp domain

Branch `feat/conversion-d12` (from `conversion` at `e2d16a491`). Thread control
is one joint slice with the Meta side (Q.I1b, DECISIONS 2026-11-17 "Q.I1b thread
control").

## Slices

| # | Slice | Rust | Original |
|---|-------|------|----------|
| 1 | Thread-control service: record an event (guarded transitions, divider + context card, redelivery / stale), request an action (take / pass / release, expected version), sync the owner, inbound delivery + standby promote, archive release | `business::thread_control` (`mod.rs`, `channel.rs`) | `packages/business/src/thread-control/*`, `integrations/{whatsapp,messenger}/src/apis/thread-control.ts` |
| 2 | `POST /v1/conversations/{id}/thread-control` (take / pass / release / sync) | `api::handlers::conversations` | `features/conversations/api/public.ts` |
| 3 | `/rpc` actions `threadControlAction`, `syncThreadOwnerAction` (contact scope like `requireContactAccessForMember`) | `api::rpc_procedures::conversations` | `features/conversations/actions/thread-control*.ts` |
| 4 | Archive auto-release: `set_archived` enqueues `threadControlAction` (job id `thread-release-{ci}-{version}-{archivedAt}`); worker re-reads the row and releases | `business::conversation_actions`, `worker::thread_control::release_owned_thread` | `conversation-actions.ts`, `apps/worker/src/integration/handlers/thread-control.ts` |
| 5 | Handover webhooks → `threadControlEvent` → recorded event: WhatsApp `messaging_handovers` (`wa-tc-` id) and Messenger `pass_thread_control` / `take_thread_control` / Business-AI hand-back notice (`fb-tc-` id, direction from app ids, AI app = `ai_agent`) | `worker::channels::{whatsapp_webhook,messenger_webhook}`, `worker::thread_control::receive_event` | `integrations/*/src/lib/conversation-routing.ts`, `handlers/webhook.ts`, `handlers/conversation.ts` |
| 6 | `/v1 whatsappTemplates.list` / `templateMessages.list` (alias, from Q.D4) / `get` (parameters) / `sync` (full sync, 70 pages); `searchCatalogProducts` = `products.searchMetaCatalogItems` | `api::handlers::whatsapp_templates`, `meta_catalog` | `features/integration-whatsapp/message-templates/api/public.ts`, `lib/sync-whatsapp-templates.ts` |
| 7 | `/v1 whatsappFlows.list` / `getScreens` / `sync`; `parseFlowScreens` | `api::handlers::whatsapp_flows`, `business::whatsapp_flow` | `features/integration-whatsapp/flows/*`, `integrations/whatsapp/src/api/flow.ts` |
| 8 | `/rpc whatsappMessageTemplateAPIs.*`, `whatsappFlowAPIs.*`, actions `syncMessageTemplateAction`, `syncWhatsappFlowsAction` | `api::rpc_procedures::whatsapp` | `*/api/private.ts`, `*/actions/sync-*.ts` |

## Security

- Every route reads the conversation / contact inbox / integration inside the
  token's workspace first; a row of another workspace reads as missing (404).
  `/rpc` thread control applies the member's contact scope.
- Webhooks: the handover jobs are built only after the existing signature
  check of the WhatsApp / Messenger webhook routes. A WhatsApp handover for a
  number other than the route-pinned one is dropped.
- Outbound: Graph calls go to the fixed Graph URL with the stored token;
  redirects are off. The Flow asset `download_url` comes from Meta, so it is
  checked with `assert_public_url` (SSRF guard) before the fetch; a private
  host reads as "no screens".
- The original has no WhatsApp Flows data-exchange endpoint (no
  `encrypted_flow_data` decrypt), so there is nothing to port there.

## Decisions

See `conversion/DECISIONS.md` (2026-11-17 "Q.D12" lines).

## Tests

- `crates/business/tests/thread_control_service.rs` (record, inbound, request,
  refresh, archive release, owner classify).
- `crates/api/tests/conversations_thread_control_public_api.rs` (port of
  `conversations-thread-control-public-api.test.ts`, local mock Graph).
- `crates/api/tests/whatsapp_templates_public_api.rs` (ports of
  `whatsapp-templates-more-public-api.test.ts`,
  `whatsapp-template-status-forwarding.test.ts`).
- `crates/api/tests/whatsapp_flows_public_api.rs` (port of
  `whatsapp-flows-public-api.test.ts`).
- Unit tests: `business::thread_control::channel`, `business::whatsapp_flow`,
  `worker::thread_control` (ports of the WhatsApp / Messenger
  `conversation-routing-parsers.test.ts` handover cases), `messenger_webhook`.

## Parity suites to run

p1-10 / p1-11 WhatsApp and Meta folders (thread control, templates, flows) and
the conversations thread-control cases. Not run in this unit.

## Left

- WhatsApp calls: the call-event worker, native recording / transcript
  fetch, `whatsappCallRecordingReady`, `transcribeCall` and
  `whatsappCalls.generateSummary` are ported (follow-up 2). Left: the
  `whatsappCalling.*` routes, the VoIP actions and signalling (WebRTC SDP,
  `whatsappVoipSignaling` queue, Redis control record, agent realtime hints)
  and the inbox summary server action in `apps/web`.
- Coexist: `coexistAttachmentDownload` (WhatsApp media only) and
  `coexistMessengerSync` are ported. Left: `coexistInstagramSync` (the
  scheduler queues it; the job fails as an undefined action) and the
  Messenger BUC throttle / parallel threads.
- Thread control: standby messages / echoes, Messenger requests and app roles,
  the hand-back response (`startHandoverResponse` resume flow, AI settings),
  AI take-back, the `isOwnRetry` check, and the inbound-pipeline hooks
  (`record_inbound_delivery` / promote are in the service but not called from
  `inbound.rs`).
- Web: the WhatsApp channel pages (`/space/$id/whatsapps/$id/*`,
  `settings/channels/whatsapp`) are not routed in `apps/web` (they need the
  WhatsApp channel list / connect screens). The inbox thread-control buttons
  and the flow-editor template / Flow option lists already exist and now get
  real data.
