# Q.I1b — Meta channels (Messenger, Instagram, Instagram-Facebook, Threads)

Branch `feat/conversion-meta`. The channels live in the worker crate next to
WhatsApp (`crates/worker/src/channels/`); the HTTP route is in the server
crate (`crates/server/src/meta_webhook.rs`). Inbound reuses the shared
pipeline of Q.I1a (`crates/worker/src/inbound.rs`, see specs/Q.I1a.md).

## Slices

| # | Slice | State |
|---|-------|-------|
| 1 | Messenger: shared Graph client (`channels/graph.rs`), shared Meta helpers (`channels/meta.rs`), `Channel` impl (`channels/messenger.rs`: send, flow steps, comments, private replies, typing, mark-as-read, inbound parser), webhook parser (`channels/messenger_webhook.rs`), webhook route with credential resolution (`channels/meta_webhook.rs` + `server/src/meta_webhook.rs`), worker jobs `incomingMessageEcho`, `contactMarkAsRead`, `messageReaction`, `deleteIncomingMessage`, `updateIncomingComment`, `deleteIncomingComment` (`worker/src/inbound_events.rs`), parity fixture `createMessengerPage` | done (60ec6aaea, dd4749b47) |
| 2 | Instagram (`integrations/instagram`, graph.instagram.com): `Channel` impl (`channels/instagram.rs`: send with the 24 h / 7 d policy, flow steps, comment reply / hide / delete, private replies, typing, mark-as-read, inbound parser with location / story reply / story mention), webhook jobs (comments, live comments, messaging) on the shared Meta route | done (49cc18b9f) |
| 3 | Instagram-Facebook (`integrations/instagram-facebook`): `Instagram` variant `Facebook` (graph.facebook.com, page-scoped sends, thread-control take-over on 2534037, `{igId}/likes`), `type = 'facebook'` rows resolved centrally in `Channels::resolve`, `instagramFacebook` webhook jobs | done |
| 4 | Threads (`integrations/threads`): `Channel` impl (`channels/threads.rs`: comment reply = create, poll status, publish; hide / unhide), own HTTP client (form bodies, GET retries), error mapper + sanitizer, `?appId=` webhook with `sha256=` signature | done |
| 5 | Messenger routes / steps: `messengerPersonas.*`, `userPersistentMenus.*`, `messengerTemplates.*`, menu / composer / persona steps, thread control (Messenger side, shared with Q.D12) | in progress: 5a steps `setMessengerUserPersistentMenu`, `enable/disableMessengerComposer`, `setMessengerPersona` + channel `custom_user_settings` calls; 5b `messengerPersonas.list`, `userPersistentMenus.*` + both `/rpc` lists |

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

## Instagram

- Auth: `tokens.accessToken`, `metadata.{igId, pageId, version}`
  (default `v22.0`), on `graph.instagram.com`.
- Send policy (`resolveMessagingPolicy`): automated sends only inside 24 h
  (`instagram_response_window_expired`); inbox sends `RESPONSE` inside 24 h,
  `MESSAGE_TAG` + `HUMAN_AGENT` inside 7 d, else
  `instagram_human_agent_window_expired`.
- Comment replies are text only (media = `PayloadInvalid`); edit and like
  are no-ops with a log (no API); hide is `POST {commentId}?hide=`.
- Webhook: one malformed entry or event skips the whole delivery (the
  original parses the body with one schema); comments come from a top-level
  `field` / `value` or from every `changes[]` item (`live_comments` adds
  `isLive`).
- Outbound to an `IntegrationInstagram` row with `type = 'facebook'` uses
  the Instagram-Facebook channel (`isInstagramViaFacebook`).

## Instagram-Facebook

- Same row and auth as Instagram; sends use the top-level `auth.version`
  (default `v23.0`) on graph.facebook.com and `{pageId}/messages`.
- No messaging policy; error 2534037 (not thread owner) = take thread
  control, then one retry.
- Flow step quick replies go on the last message; uploads retry 2 times.
- Webhook: comments only from `changes[]`, tagged `instagramFacebook`;
  `read.watermark` is required.

## Threads

- `?appId=` = client id of the Threads credential; `/c/{id}` is not used.
- Signature `sha256=<hex>`, compared as decoded bytes (upper-case hex
  matches). Errors: 400 for known bad requests, 405, else 500, with a
  public message only.
- Own HTTP client: form-urlencoded, 30 s; GET retried 2 times on
  408/413/429/5xx and transport errors, POST never.

## Messenger steps and menus

- Steps resolve the Messenger user from the run contact inbox, else the
  contact's latest Messenger inbox of the workspace; no user = success.
- `updateMessengerContactData` is not ported (needs the profile lookup).
- `userPersistentMenus.*`: `paginateInMemory`, item union `flow` / `url`,
  max 20 items, single-id delete 404s.

## Parity

- p1-11 `41-messages-messenger`: 4/5 on the Rust stack (2026-11-17, after dd4749b47; was 0/5). The left case (two `mediaFileIds` on a comment reply -> 422 `imagesNotSupportedOnChannel`) needs the media-library outgoing path (`mediaFileIds`, now 501 `notPorted` in `contact_messages.rs`), owned by the messages unit.

## Left

- `messengerTemplates.*` and `messengerChannels.*` routes (parity p1-11
  50 / 51 / 60).
- Conversation routing (`threadControlEvent`: handover, requests, app roles,
  standby): dropped with a log until thread control (Q.D12 / slice 5).
- Worker jobs not ported yet (fail "IntegrationJobAction action is not
  defined"): `incomingComment` (receiveComment + comment automation),
  `channelLabelChange`, `processLeadgen`, `processMarketingMessageWebhook`.
- Inbound profile lookup (`getProfile` for new contacts), reaction
  leaderboard award, `low` queue routing of echoes (no low worker in Rust).
- Inbound attachments stay pending provider URLs (no download / re-host).
