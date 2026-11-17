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
| 4 | `/whatsapp-template` (`messages.sendWhatsappTemplate` + worker `sendWhatsappTemplateToConversation` / `processWhatsappTemplate`, `send-wa-template.ts` components, payment-request parameters, template Flow tokens), `/whatsapp-call-permission` (Meta `call_permissions` check), message `/flow-response` view, Rust parity fixtures `createWhatsappChannel` / `createWhatsappTemplate` | done |
| 5 | Media: inbound copy (`fetchMedia` + `storeIncomingImage`, SSRF-checked hops), outbound `prepareOutboundMedia` / `resolveOutboundMediaUrl` (HEAD probe, re-host to `outbound-media/<sha256>`) | done |
| 6 | Flow steps: carousel (native 2..10 chunks + legacy cards), view catalog, option list, call button, location, WhatsApp Flow, products (step + product rows), `sendWaTemplateMessage` through `processWhatsappTemplate` | done |
| 7 | Platform webhook route `GET|POST /integrations/whatsapp/webhook` (platform credential decrypt, handshake, enforced signature, no pin) | done |
| 8 | Webhook events: coexist history, identity changes, account updates | todo |

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
- Inbound media is stored under `public/whatsapp/{ws}/{integration}/{date}/{id}`
  (`storeIncomingImage`, profile `media` for photos, else `original`). The
  download follows redirects by hand with an SSRF check per hop, sends the
  token only to the first URL's origin and stops at 100 MB. Without a file
  store (unit tests) the Graph media id stays the `originPath`.
- Outbound media links are probed and re-hosted as in the original; the cache
  is in process (24 h / 5 min), with no single flight.
- The platform route reads the platform WhatsApp credential directly (the
  minimum of Q.D11's `platform_credential`, which is not merged: its branch
  conflicts and its gates are pending). In cloud only the broker host is
  served; tenant custom domains and `/c/{credentialId}` answer 404.
- The Flow response view always answers `labelSource: "fallback"`.
- `continuesFlow` ingress stamp, coexist, calls, identity change, account
  update, automatic events and thread-control routing are not queued.

## Left

- Slice 8 webhook events: coexist (`coexistWhatsappBuffer` + flush, about
  2.3k TS lines), identity change (`whatsappIdentityChange` +
  `rotateScopedUserId` / `changePrimaryPhone` / `adoptPhoneNumberIfSafe`,
  about 1.1k), account update (`providerAccountEvent` + connection state).
- Q.D12: calls (`whatsappCallEvent`, VoIP signaling, native capture) and
  thread control (`threadControlEvent`, standby echoes).
- Tenant custom-domain and credential-scoped platform webhook routes (need
  Q.D11), the `ActionFlow` emoji CTA check, Flow JSON labels in the Flow
  response view, the 60 s call-permission cache, Google click capture,
  commerce events, automatic events.
- `api/*` management calls (WABA setup, phone numbers, Flows builder,
  calling settings, conversions, subscriptions).
- AI agent / product search / Global Router after a keyword miss (Q.D6).

## Parity

- p1-10 `41-keywords-trigger`: 8/8 on the Rust stack (2026-11-17, after c1925f064; was 0/8).
- p1-11 `42-messages-whatsapp`: 0/7, waits for slice 4 (template / call permission / flow-response routes and the `createWhatsappChannel` / `createWhatsappTemplate` Rust fixtures).
