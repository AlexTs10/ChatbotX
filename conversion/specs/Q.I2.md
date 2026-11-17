# Q.I2: Telegram, TikTok, Zalo, webchat, API channel

Each channel implements `chatbotx_worker::channel::Channel` and goes through the
shared inbound pipeline (`crates/worker/src/inbound.rs`). Inbound media reuses
`channels::whatsapp_media::copy_to_storage`. Webhook routes are in
`crates/server` (the API crate does not depend on the worker).

## Slice 1: webchat guest round trip + Telegram

- `crates/business/src/integration_webchat.rs`: guest id, origin checks
  (embed allowlist or first-party host), HMAC access token (30 min, bound to
  workspace, webchat, origin host), `create_guest_conversation`
  (contact + contact inbox + conversation inside the MAC quota).
- `crates/worker/src/webchat_guest.rs`: `createWebchatMessage` (init, initRef,
  flow, text, postback, skip, challenge), `shouldRunWebchatChallenge`.
- `crates/server/src/webchat.rs`: `/api/guest/messages` (OPTIONS, GET, POST) and
  `/api/guest/realtime-token` (POST), CORS, rate limits, validation errors.
- `crates/worker/src/channels/telegram.rs`: Bot API client (ky retry policy),
  error categories, inline keyboards, flow steps, message send, typing, update
  parsing, media (getFile + shared copy), audience events, webhook routing.
- `/integrations/telegram/webhook?botId=` (GET, POST) in
  `crates/server/src/whatsapp.rs`; secret header compared with
  `auth.metadata.webhookSecretToken`.
- `connectTelegramAction` calls `setWebhook` (allowed updates + secret token)
  and stores the secret.

Not ported (ponytail): guest file uploads (JSON route), `enqueueHandoffReentry`,
`emitContactCreated`, `finalizeContactProfile` phone hint / region timezone,
`cancelByInboxSource`.

## Slice 2: TikTok

- `crates/worker/src/channels/tiktok.rs`: Business API client (`Access-Token`,
  ky retry on 408/429/5xx, `code != 0` rejected), error mapper, QA button /
  link cards (3 per card, title 40 chars), text / image / multiple images,
  image upload (hand-built multipart), DM parse (echo, share_post, postback
  from `reply_source_unique_id`).
- The DM `conversation_id` is stored on
  `Conversation.additionalAttributes.channelConversationId`
  (`IncomingContact.channel_conversation_id`); sends read it through
  `OutgoingContact::new` (`resolveChannelConversationId`).
- `/integrations/tiktok/webhook`: `TikTok-Signature` (`t=,s=` HMAC, 300 s
  window, 2 s skew); `authorization.removed` marks the Connection
  `needs_reauth` (and the inbox) before the signature check, as the original;
  comment / high-intent / DM jobs (DM with a 2 s delay).
- `tiktokChannels.{list,get,getCommentToMessage,updateCommentToMessage}` and
  `zaloChannels.{list,get,updateTagSync}` in
  `crates/api/src/handlers/channel_integrations.rs` over
  `crates/business/src/channel_integration.rs`.

Not ported: comment writes (Q.D10), the comment / high-intent / audience job
handlers (other units), TikTok / Zalo OAuth connect + callback, the
credential-scoped route, quota release on revoke.

## Slice 3: Zalo + zalo-verifier

- `crates/worker/src/channels/zalo.rs`: OA client (`access_token` header,
  `error != 0` / `error_name` rejected, no POST retry), error-code
  categories, revoked = `-124`, buttons (`oa.open.url`, `oa.open.phone`
  without `+`, `oa.query.hide` with `postback_`), text (5 buttons incl.
  quick replies), image / gif / multiple images / file (upload first), send
  message (all uploads before the first send), receive (attachments copied
  with the bearer token, a failed copy drops only that file; location; postback
  prefix; `msg_id` required).
- `/integrations/zalo/webhook` (platform `zalo` credential): tag events →
  `channelLabelChange`; `app_id` must equal `clientId`; signed follow /
  unfollow → `channelAudienceEvent` (raw-body `mac=sha256(...)`); delivery
  receipts and events without a message are dropped; `user_seen_message` →
  `contactMarkAsRead`; `oa_send*` → `incomingMessageEcho`. Errors answer 400
  `Webhook processing failed: ...`.
- `/api/zalo-verifier/{verifier}` and the `/zalo_verifier{v}.html` rewrite
  (an app-wide layer); the verifier is HTML-escaped.

## Tests

Ported: `webchat-guest-session.test.ts` (server cases),
`should-run-webchat-challenge.test.ts`, `integrations/telegram/__tests__`
(group routing, copy text, call phone, quick replies, multiple images, error
mapper, connection describe, incoming message parse + media requests). The
Telegram download mocks are n-a: the copy code is WhatsApp's, tested there.
`integrations/tiktok/__tests__` (webhook signature, comment and high-intent
routing, incoming message, quick replies, error mapper; the HTTP-mocked API
tests are asserted on the payload builders). `integrations/zalo/__tests__`
(error mapper, call button, quick replies, multiple images, message ids,
text with images, incoming gif / skip postback, webhook routing, follow
events) with a local HTTP stub for the send path. n-a: Zalo OAuth, token
refresh, profile, tags, image compression (other units).
`webchat-message-schema.test.ts` (guest id formats).
