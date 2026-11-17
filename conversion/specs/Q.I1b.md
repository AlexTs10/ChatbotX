# Q.I1b — Meta channels (Messenger, Instagram, Instagram-Facebook, Threads)

Branch `feat/conversion-meta`. The channels live in the worker crate next to
WhatsApp (`crates/worker/src/channels/`); the HTTP route is in the server
crate (`crates/server/src/meta_webhook.rs`). Inbound reuses the shared
pipeline of Q.I1a (`crates/worker/src/inbound.rs`, see specs/Q.I1a.md).

## Slices

| # | Slice | State |
|---|-------|-------|
| 1 | Messenger: shared Graph client (`channels/graph.rs`), shared Meta helpers (`channels/meta.rs`), `Channel` impl (`channels/messenger.rs`: send, flow steps, comments, private replies, typing, mark-as-read, inbound parser), webhook parser (`channels/messenger_webhook.rs`), webhook route with credential resolution (`channels/meta_webhook.rs` + `server/src/meta_webhook.rs`), worker jobs `incomingMessageEcho`, `contactMarkAsRead`, `messageReaction`, `deleteIncomingMessage`, `updateIncomingComment`, `deleteIncomingComment` (`worker/src/inbound_events.rs`), parity fixture `createMessengerPage` | in progress |
| 2 | Instagram (`integrations/instagram`, graph.instagram.com) | todo |
| 3 | Instagram-Facebook (`integrations/instagram-facebook`) | todo |
| 4 | Threads (`integrations/threads`, `?appId=` webhook) | todo |
| 5 | Messenger routes / steps: `messengerPersonas.*`, `userPersistentMenus.*`, `messengerTemplates.*`, menu / composer / persona steps, thread control (Messenger side, shared with Q.D12) | todo |

## Messenger

- Auth is the plain `IntegrationMessenger.auth` JSON (as the original: not
  encrypted): `tokens.accessToken` (page token), `metadata.{pageId, version}`.
- Every send carries `metadata: "SENT_FROM_CHATBOTX"`; the webhook drops
  echoes with it.
- Messaging policy (`resolveMessagingPolicy`): inside 24 h `RESPONSE`, inside
  7 d `MESSAGE_TAG` + `HUMAN_AGENT` when sent by a human, else the original
  error.
- Graph errors map through `map_error` (`error-mapper.ts`); network errors
  are `Unknown` (as the original); sends are not retried (`retry: 0`).
- Flow step media is uploaded as an attachment (`/me/message_attachments`)
  and sent by `attachment_id`; domains are whitelisted once per process
  (`ENSURED_DOMAINS`, sub code 2018062 retry).

## Webhook (P1.21)

`GET|POST /integrations/{type}/webhook` and
`/integrations/{type}/webhook/c/{credentialId}`, one static path pair per
Meta type (no overlap with the WhatsApp manual route).

Credential: `/c/{id}` (cloud only) = `findDecryptedForWebhook`, jobs only for
inboxes of its owner; cloud + broker host (`x-domain` = host of
`NEXT_PUBLIC_BROKER_URL ?? NEXT_PUBLIC_BUILDER_URL`) = platform row; cloud
custom domain = active tenant owner's own row; non-cloud = platform row.
None = 404 `{"message":"Integration is not configured"}`.

Messenger handler: handshake (`hub.mode=subscribe`, token, non-empty
challenge); POST: empty body, missing / invalid `X-Hub-Signature-256`
(HMAC-SHA256 hex of the raw body with `clientSecret`, exactly one `=`,
constant-time compare) = 400 with the nested original text; 16+ digit ids
quoted before parsing. Jobs: `incomingMessage` (+ postback `action`),
`incomingMessageEcho`, `contactMarkAsRead`, `messageReaction`,
`deleteIncomingMessage`, `incomingComment`, `updateIncomingComment`,
`deleteIncomingComment`, `channelLabelChange`, `processLeadgen`,
`processMarketingMessageWebhook`; each `messaging[]` event as a single-event
payload.

## Left

- Conversation routing (`threadControlEvent`: handover, requests, app roles,
  standby): dropped with a log until thread control (Q.D12 / slice 5).
- Worker jobs not ported yet (fail "IntegrationJobAction action is not
  defined"): `incomingComment` (receiveComment + comment automation),
  `channelLabelChange`, `processLeadgen`, `processMarketingMessageWebhook`.
- Inbound profile lookup (`getProfile` for new contacts), reaction
  leaderboard award, `low` queue routing of echoes (no low worker in Rust).
- Inbound attachments stay pending provider URLs (no download / re-host).
