# Q.I1a — WhatsApp channel

Branch `feat/conversion-wa`. WhatsApp lives in the worker crate next to the
other channel senders (`crates/worker/src/channels/whatsapp.rs`,
`channels/whatsapp_webhook.rs`); the HTTP route is in the server crate
(`crates/server/src/whatsapp.rs`).

## Slices

| # | Slice | State |
|---|-------|-------|
| 1 | Shared inbound pipeline (`worker/src/inbound.rs`), `ai-agent` queue with the keyword step of `processAutomatedResponse` (`worker/src/ai_agent.rs`), API channel inbound (`channels.sendMessage` + `ApiChannel::receive_message`), ports of `keyword-match.ts`, `extract-contact.ts`, `workspace.isActiveNow` | done |
| 2 | WhatsApp `Channel`: Graph send (lib / raw body, BSUID recipient), error mapper, `sendMessage` (text, media, location request, call permission request), `sendFlowStep` (`sendText`, `sendImage`, `sendMultipleImages` with reply buttons, lists, CTA links), `receiveMessage` parsers (text, `/ref-`, location, contacts, order, interactive button / list / Flow / call permission replies, template button, media, referral), `handleMessageStatus`, `sendTyping` | done |
| 3 | Manual webhook route `GET|POST /integrations/whatsapp/webhook/{integrationId}` (handshake, `markWebhookVerified`, verified gate, `X-Hub-Signature-256` policy, per-item split, `wa-msg-` / `wa-status-` / `wa-reaction-` job ids) | done |
| 4 | Templates (`whatsappTemplates.*`, `/whatsapp-template`), call permission route, Flow response view, parity fixtures | todo |

## Inbound pipeline (reuse for every channel)

A channel webhook queues `incomingMessage` (or `messageStatus`) on the
`integration` queue with `{ type, data: { integrationType,
integrationIdentifier, payload } }`. The worker:

1. `Channels::resolve_by_identifier(type, identifier)`: the integration row by
   its identifier column (`identifier_column` in `channel.rs`) joined to its
   inbox. Unknown = unrecoverable failure.
2. `Channel::receive_message(ctx, payload) -> Option<Received>` (the SDK
   `ReceivedMessageResult`); a channel implements only this parser.
3. `inbound::receive_message`: contact inbox by `sourceId` → `sourceUserId` →
   `sourceParentUserId`, else new Contact + ContactInbox + Conversation in one
   transaction; contact phone / email from the text; auto-unblock; the message
   via `create_or_update(_with_attachments)` (dedup by `sourceId`); on a new
   row `recordInboundActivity` and realtime `messageCreated`.
4. `inbound::route_incoming_message`: postback / quick reply → `runFlowPostback`
   / `runFlowQuickReply`; `ref` → `runRef`; text / keyword media →
   `resolve_incoming_text_routing` → `processAutomatedResponse` on `ai-agent`
   (job id `automated-response-<messageId>`, dedup
   `automated-response:<conv>-<ci>:messages`, 2 s delay / ttl, extend +
   replace) plus the pending-message list in the durable kv.
5. `ai_agent::process_automated_response`: workspace active + bot on, the
   pending messages, inbound rules (`status = 1`, `createdAt` asc), first
   match: flow (published) → `sendFlow`, text → spintax, variables →
   `sendChatMessage`; `automatedResponseMatched` activity + keyword message.

## Decisions

- WhatsApp is a module of `chatbotx-worker`, not a new crate: the channel
  trait, `Channels::builtin()` and the API / webchat senders already live there.
- Media of inbound messages is not copied to storage yet; the attachment keeps
  the Graph media id as `originPath` (ponytail, see "Left").
- Only the manual per-integration webhook route is ported; the platform
  credential route `/integrations/whatsapp/webhook` needs the platform
  credential service (not ported).
- `continuesFlow` ingress stamp, coexist, calls, identity change, account
  update, automatic events and thread-control routing are not queued.

## Left

- Flow steps: carousel, view catalog, `sendWaTemplateMessage`, option list,
  call button, location, WhatsApp Flow, products.
- Template routes (`whatsappTemplates.*`, `/v1/conversations/{id}/whatsapp-template`),
  `/whatsapp-call-permission`, message `/flow-response`, thread control.
- Inbound media download (`fetchMedia` + `storeIncomingImage`), outbound media
  re-hosting (`prepareOutboundMedia`), Google click capture, standby echoes,
  commerce events, template Flow tokens.
- `api/*` management calls (WABA setup, phone numbers, Flows builder,
  calling settings, conversions, subscriptions).
- AI agent / product search / Global Router after a keyword miss (Q.D6).
