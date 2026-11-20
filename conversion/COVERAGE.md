# Coverage: what is not ported yet (Q.P6-audit)

Measured on branch `feat/conversion-audit` (from `conversion` 7365e3381),
2026-11-20. "Not ported" = the Rust server has no handler, or the handler
answers "not ported" / skips the work. The dropped **Execute JavaScript**
feature (user decision, DECISIONS.md 2026-11-11) is **dropped, not missing**,
and is not counted.

How each list was made:

| Group | Method | Guard |
|---|---|---|
| A. Public API (`/api/v1`) | `crates/api/tests/route_coverage.rs` calls all 811 `PUBLIC_ROUTES` through `public_api_router_with` + `LiveAuthDeps` with a full owner workspace token, path params `1`, body `{}`, a fresh database per call (channel / mobile routes: their handler modules directly). | `tests/fixtures/not_ported_routes.txt` |
| B. `/rpc` | The same test calls all 274 `RPC_PROCEDURES` and every `serverAction` / `serverFunction` / `serverQuery` name in `apps/web/src` with the owner session (fresh database per call). Actions with their own server route (`/rpc/actions/<name>` in `crates/api` / `crates/server`) count as ported. The SPA page loaders (`loadPage`, 19 names) were checked by hand: all 19 are served. | `tests/fixtures/not_ported_rpc.txt` |
| C. Other HTTP routes | The 48 `apps/builder/src/app/**/route.ts` files (and `apps/realtime`) against the `.route(..)` paths of `crates/api` / `crates/server` / `crates/rt`. | this file |
| D. Worker | The `case` labels of `apps/worker/src/*/worker.ts`, the schedulers of `crates/jobs/src/schedules.rs` and the flow step types of `step_action.rs` against the Rust dispatchers (`crates/worker/src/{integration,chat,default_queue,ai_agent,ai_file,low}.rs`, `schedule/mod.rs` `PORTED`, `flow.rs` + `steps/mod.rs`). | this file |
| E. Gaps inside ported code | `grep -rni "not ported\|501\|NOT_IMPLEMENTED"` in `crates/*/src` (107 lines), sorted by the feature each blocks. | this file |

The test prints the full lists (`cargo test -p chatbotx-api route_coverage -- --nocapture`)
and fails when a list changes, so a list only shrinks on purpose: delete the
line from the fixture in the commit that ports it.

## Summary

| Group | Not ported | Notes |
|---|---:|---|
| A. Public API | 1 of 811 | |
| B. `/rpc` procedures + actions | 90 (62 of 274 procedures, 28 of 678 SPA action names) | 21 called by the SPA; 53 procedures and 16 action stubs have no SPA caller |
| C. Other HTTP routes | 12 | + 2 OAuth callback flows |
| D. Worker | 45 | 18 queue jobs, 16 schedulers (7 cloud / enterprise only, 3 probably n/a), 9 flow steps, 2 job families |
| E. Gaps inside ported code | 22 | features, not ids |

## A. Public API (1)

| Id | Route | Original | Rust state |
|---|---|---|---|
| `sequences.listStepContacts` | `GET /v1/sequences/{id}/steps/{stepId}/contacts` | `apps/builder/src/features/sequences/api/public.ts` | `crates/api/src/handlers/sequences.rs` (needs the sequence step analytics; the `/rpc` twin uses `sequence::list_step_contacts_page`) |

All other 810 routes reach a Rust handler (channel-token and mobile-user
routes included). `realtime.getConnectToken` answers 403 to a workspace
token by design (mobile sessions only).

## B. `/rpc` (90: 62 procedures, 28 actions)

### B1. Procedures the SPA calls (9)

| Procedure | Original |
|---|---|
| `channelPostAPIs.privateGetChannelPostOptionsByIdsAPI` | `apps/builder/src/features/channel-posts/api` |
| `channelPostAPIs.privateListChannelPostOptionsAPI` | `apps/builder/src/features/channel-posts/api` |
| `conversationsAPI.getPostDetailsAuthenticatedAPI` | `apps/builder/src/features/conversations/api` |
| `flowsAPI.privateGetMessengerAdsJsonAPI` | `apps/builder/src/features/flows/api` |
| `integrationWhatsappAPIs.connectWhatsappNumberAPI` | `apps/builder/src/features/integration-whatsapp/api` |
| `integrationWhatsappAPIs.listIntegrationWhatsappInternalAPI` | `apps/builder/src/features/integration-whatsapp/api` |
| `messagesAPI.getWhatsappFlowResponseAuthenticatedAPI` | `apps/builder/src/features/messages/api` |
| `messengerMessageTemplateAPIs.listMessengerMessageTemplatesInternalAPI` | `apps/builder/src/features/integration-messenger/message-templates/api` |
| `productsAPI.listProductsAuthorizedAPI` | `apps/builder/src/features/products/api` |

### B2. Procedures with no `apps/web` caller (53)

The SPA uses server actions for these screens; the procedures were the old
builder / mobile entry points. Port them only if a client still needs them.

| Router key (original) | Procedures |
|---|---|
| `analyticsRoutes` (`@chatbotx.io/analytics-nextjs/routes`) | `getBroadcastStatsAnalyticsAPI`, `getFlowAnalytics`, `getSequenceStepStatsAnalyticsAPI`, `resetFlowAnalytics` |
| `connectSessionsAPI` (`apps/builder/src/features/connections/api`) | `cancelConnectSessionAPI`, `connectSessionTargetsAPI` |
| `connectionsAPI` (`apps/builder/src/features/connections/api`) | `createConnectionAPI`, `disconnectConnectionAPI`, `getConnectionAPI`, `listConnectionProvidersAPI`, `listConnectionsAPI`, `reconnectConnectionAPI`, `refreshConnectionAPI`, `updateConnectionAPI`, `verifyConnectionAPI` |
| `conversationsAPI` (`apps/builder/src/features/conversations/api`) | `archiveConversationsAuthenticatedAPI`, `assignConversationsAuthenticatedAPI`, `disableBotAuthenticatedAPI`, `enableBotAuthenticatedAPI`, `followConversationAuthenticatedAPI`, `readConversationAuthenticatedAPI`, `unarchiveConversationsAuthenticatedAPI`, `unfollowConversationAuthenticatedAPI`, `unreadConversationAuthenticatedAPI` |
| `deviceTokensAPI` (`apps/builder/src/features/device-tokens/api`) | `registerDeviceTokenAPI`, `unregisterDeviceTokenAPI` |
| `fbCommentsAPI` (`apps/builder/src/features/fb-comments/api`) | `createFbCommentAPI`, `deleteFbCommentAPI`, `listFbCommentsAPI`, `updateFbCommentAPI` |
| `igCommentsAPI` (`apps/builder/src/features/ig-comments/api`) | `createIgCommentAPI`, `deleteIgCommentAPI`, `listIgCommentsAPI`, `updateIgCommentAPI` |
| `igStoriesAPI` (`apps/builder/src/features/ig-stories/api`) | `createIgStoryAPI`, `deleteIgStoryAPI`, `listIgStoriesAPI`, `updateIgStoryAPI` |
| `inboxesAPI` (`apps/builder/src/features/inboxes/api`) | `listInboxesAuthenticatedAPI` |
| `integrationFacebookAdsAPI` (`apps/builder/src/features/integration-facebook-ads/api`) | `getAdInsights` |
| `integrationWhatsappAPIs` (`apps/builder/src/features/integration-whatsapp/api`) | `listWhatsappPhoneNumbersInternalAPI` |
| `mediaLibraryAPI` (`apps/builder/src/features/media-library/api`) | `createMediaLibraryFile`, `createMediaLibraryFolder`, `deleteMediaLibraryFile`, `deleteMediaLibraryFolder`, `renameMediaLibraryFolder`, `toggleMediaLibraryFavourite` |
| `messagesAPI` (`apps/builder/src/features/messages/api`) | `changeMessageAttributesAuthenticatedAPI`, `createMessageAuthenticatedAPI`, `deleteMessageAuthenticatedAPI`, `editMessageAuthenticatedAPI`, `findMessageAuthenticatedAPI` |
| `refLinksAPI` (`apps/builder/src/features/reflinks/api`) | `getRefLinksAuthenticatedAPI` |

### B3. Server actions the SPA calls (12)

| Action | SPA stub | Original |
|---|---|---|
| `assertCurrentUserCanAccessChatbot` | `apps/web/src/lib/auth/utils.ts` | `apps/builder/src/lib/auth/utils.ts` |
| `changeMessageAttributes` | `apps/web/src/features/messages/actions/change-message-attributes.action.ts` | `apps/builder/src/features/messages/actions/change-message-attributes.action.ts` |
| `connectMetaCatalogAction` | `apps/web/src/features/products/actions/meta-catalog.action.ts` | `apps/builder/src/features/products/actions/meta-catalog.action.ts` |
| `connectTelegramAction` | `apps/web/src/features/integration-telegram/actions/connect.action.ts` | `apps/builder/src/features/integration-telegram/actions/connect.action.ts` |
| `deleteMessage` | `apps/web/src/features/messages/actions/delete-message.action.ts` | `apps/builder/src/features/messages/actions/delete-message.action.ts` |
| `editMessage` | `apps/web/src/features/messages/actions/edit-message.action.ts` | `apps/builder/src/features/messages/actions/edit-message.action.ts` |
| `forceChangePasswordAction` | `apps/web/src/features/auth/actions/force-change-password.ts` | `apps/builder/src/features/auth/actions/force-change-password.ts` |
| `reconnectWhatsappAction` | `apps/web/src/features/integration-whatsapp/actions/reconnect.action.ts` | `apps/builder/src/features/integration-whatsapp/actions/reconnect.action.ts` |
| `refreshAllChannelTokensAction` | `apps/web/src/features/workspaces/actions/refresh-all-channel-tokens.action.ts` | `apps/builder/src/features/workspaces/actions/refresh-all-channel-tokens.action.ts` |
| `refreshContactProfileAction` | `apps/web/src/features/contacts/actions/refresh-contact-profile.action.ts` | `apps/builder/src/features/contacts/actions/refresh-contact-profile.action.ts` |
| `sendWhatsappTemplateAction` | `apps/web/src/features/messages/actions/send-whatsapp-template.action.ts` | `apps/builder/src/features/messages/actions/send-whatsapp-template.action.ts` |
| `testExternalRequestAction` | `apps/web/src/features/flows/actions/test-external-request-action.ts` | `apps/builder/src/features/flows/actions/test-external-request-action.ts` |

### B4. Action stubs nothing imports (16)

Defined in `apps/web/src` but not imported by any other file: dead client
stubs (delete them, or port when a screen needs them).

| Action | SPA stub | Original |
|---|---|---|
| `createProduct` | `apps/web/src/features/products/actions/create-product-action.ts` | `apps/builder/src/features/products/actions/create-product-action.ts` |
| `deleteContactCustomFields` | `apps/web/src/features/contacts/actions/delete-contact-custom-field.action.ts` | `apps/builder/src/features/contacts/actions/delete-contact-custom-field.action.ts` |
| `enableAutomatedResponse` | `apps/web/src/features/automated-response/actions/enable-automated-response-action.ts` | `apps/builder/src/features/automated-response/actions/enable-automated-response-action.ts` |
| `getCurrentUser` | `apps/web/src/lib/auth/utils.ts` | `apps/builder/src/lib/auth/utils.ts` |
| `getCurrentUserAndAllLinkedWorkspaces` | `apps/web/src/lib/auth/utils.ts` | `apps/builder/src/lib/auth/utils.ts` |
| `getCurrentUserAndTargetWorkspace` | `apps/web/src/lib/auth/utils.ts` | `apps/builder/src/lib/auth/utils.ts` |
| `getCurrentUserId` | `apps/web/src/lib/auth/utils.ts` | `apps/builder/src/lib/auth/utils.ts` |
| `persistReconnectAuthAndResubscribe` | `apps/web/src/features/integration-whatsapp/actions/reconnect.action.ts` | `apps/builder/src/features/integration-whatsapp/actions/reconnect.action.ts` |
| `resetBotFields` | `apps/web/src/features/bot-fields/actions/reset-bot-field.action.ts` | `apps/builder/src/features/bot-fields/actions/reset-bot-field.action.ts` |
| `resetContactCustomFields` | `apps/web/src/features/contacts/actions/reset-contact-custom-fields.action.ts` | `apps/builder/src/features/contacts/actions/reset-contact-custom-fields.action.ts` |
| `submitAvailabilityRange` | `apps/web/src/app/booking/range-picker/actions/submit-availability-range.action.ts` | `apps/builder/src/app/booking/range-picker/actions/submit-availability-range.action.ts` |
| `submitBooking` | `apps/web/src/app/booking/picker/actions/submit-booking.action.ts` | `apps/builder/src/app/booking/picker/actions/submit-booking.action.ts` |
| `updateAutomatedResponse` | `apps/web/src/features/automated-response/actions/update-automated-response-action.ts` | `apps/builder/src/features/automated-response/actions/update-automated-response-action.ts` |
| `updateIntegrationOpenAI` | `apps/web/src/features/integration-openai/actions/update-openai.action.ts` | `apps/builder/src/features/integration-openai/actions/update-openai.action.ts` |
| `updateMagicLink` | `apps/web/src/features/magic-links/actions/update-magic-link.action.ts` | `apps/builder/src/features/magic-links/actions/update-magic-link.action.ts` |
| `updateProduct` | `apps/web/src/features/products/actions/update-product-action.ts` | `apps/builder/src/features/products/actions/update-product-action.ts` |

## C. Other HTTP routes of the original (12 + 2 flows)

Original root: `apps/builder/src/app/`. Missing = no Rust route; the SPA
server serves its HTML shell for an unknown GET, so a link answers a page,
not the file.

| # | Route | Original (LOC) | Who calls it |
|---|---|---|---|
| C1 | `/space/[workspaceId]/flows/[id]/export` | `(no-sidebar)/space/[workspaceId]/flows/[id]/export/route.ts` (60) | SPA `features/flows/lib/download-flow-export.ts` |
| C2 | `/space/[workspaceId]/dashboard/ads/export` | `space/[workspaceId]/dashboard/ads/export/route.ts` (297) | SPA `features/ads/components/ads-analytics-view.tsx` |
| C3 | `/space/[workspaceId]/ads/analytics/export` | `space/[workspaceId]/ads/analytics/export/route.ts` (7, re-export of C2) | old bookmarks |
| C4 | `/api/imports/products/template` | `api/imports/products/template/route.ts` (20) | SPA `features/products/components/import-product-dialog.tsx` |
| C5 | `/media/attachment/[token]` | `media/attachment/[token]/route.ts` (173) | SPA `features/attachments/utils.ts`, `resolveMediaUrl` |
| C6 | `/media/avatar/[token]` | `media/avatar/[token]/route.ts` (99) | Rust `business/src/contact/avatar.rs:155` writes these URLs (broken avatars) |
| C7 | `/email-topic/open` | `email-topic/open/route.ts` (44) | worker email step (tracking not ported, see E12) |
| C8 | `/email-topic/click` | `email-topic/click/route.ts` (103) | worker email step |
| C9 | `/integrations/whatsapp/callback` | `integrations/whatsapp/callback/route.ts` (123) | SPA embedded signup (`WHATSAPP_OAUTH_CALLBACK_PATH`); the generic `/integrations/{provider}/callback` has no WhatsApp branch |
| C10 | `/extensions/me/download` | `extensions/me/download/route.ts` (41) | browser extension download |
| C11 | `/api/public-spec.json` | `api/public-spec.json/route.ts` (126) | OpenAPI document of the public API |
| C12 | `/api/scalar-ui` | `api/scalar-ui/route.ts` (39) | API docs page |
| C13 | callback flow `metaCapi` | `integrations/[...integration]/callback-meta-capi.ts` (238) | Conversions API OAuth connect; not in `crates/api/src/connect_callback.rs` |
| C14 | callback flow `metaCatalog` | `integrations/[...integration]/callback-stores.ts` `storeMetaCatalogConnection` | Meta catalog connect (see B `connectMetaCatalogAction`) |

Replaced by design (not counted): `/api/workspace-presence/report` (in-process
presence, `crates/rt/src/presence.rs`, decision D3), the legacy JSON-state
callback flows (connect sessions, DECISIONS Q.D2c / Q.I2 / Q.F2),
`/space/[workspaceId]/contacts/import/template` (action
`getContactsImportTemplate`), `apps/realtime` (`crates/rt`, `/rt/*`).
Present: every other route, including all webhooks (WhatsApp, Meta x4,
Telegram, TikTok, Zalo + verifier, webchat guest, payments x3, Shopify,
WooCommerce, inbound hooks), auth / OAuth / `.well-known`, invitations,
leaderboards, reflinks, `/r/*`, presigned upload, mini-app uploads,
dynamic images, VoIP. The better-auth endpoints the SPA uses
(`useSession`, `signIn.email|social|magicLink`, `signUp.email`, `signOut`,
`updateUser`, `requestPasswordReset`, `resetPassword`) are all served.

## D. Worker (45)

### D1. Queue jobs without a Rust handler (18)

| Queue | Job | Original handler |
|---|---|---|
| ai-agent | `processConversationSource` | `apps/worker/src/ai-agent/handlers/process-conversation-source` |
| ai-agent | `processConversationSourceEmbedding` | `apps/worker/src/ai-agent/handlers/process-conversation-source-embedding` |
| ai-agent | `processHandoffReentry` | `apps/worker/src/ai-agent/worker.ts` (`processHandoffReentryWithWebhookContext`) |
| ai-agent | `summarizeConversation` | `apps/worker/src/ai-agent/handlers/summarize-conversation` (also `appendHistory`, `crates/worker/src/ai/mod.rs:133`) |
| chat | `checkOutboundAutomatedResponse` | `apps/worker/src/chat/worker.ts` (Rust skips it, `crates/worker/src/chat.rs:62`) |
| default | `syncWelcomeMessageFlows` | `apps/worker/src/default/worker.ts` |
| default | `deleteWelcomeMessageFlows` | `apps/worker/src/default/worker.ts` |
| default | `syncChannelLabels` | `apps/worker/src/default/worker.ts` (`handleSyncChannelLabels`) |
| default | `syncTag` | `apps/worker/src/default/worker.ts` (`handleSyncTag`) |
| default | `processPipelineStageBulk` | `apps/worker/src/default/worker.ts` |
| heavy | `analyzeImage` | `apps/worker/src/heavy/worker.ts` |
| heavy | `extractTextFromFile` | `apps/worker/src/heavy/worker.ts` |
| heavy | `optimizeStoredImage` | `apps/worker/src/heavy/worker.ts` |
| integration | `captureTemplateFlowResponse` | `apps/worker/src/integration/handlers/template-flow-response` |
| integration | `recordEmailTrackingTransition` | `apps/worker/src/integration/handlers/send-email` |
| integration | `threadControlAction` | `apps/worker/src/integration/worker.ts` (`releaseOwnedThread`) |
| integration | `threadControlEvent` | dropped with a log (DECISIONS Q.I1b thread control) |
| integration + low | `updateContactAvatar` | `apps/worker/src/integration/handlers/contact/update-avatar` |

Not counted: `createMessage` (a no-op in the original); `bulkTagContacts`,
`exportContacts`, `exportCoupons`, `runImport`, `sendAuditLog` run in
process in Rust.

### D2. Schedulers without a Rust handler (16)

Original: `apps/worker/src/schedule/handlers/*`; Rust registers only
`schedule/mod.rs` `PORTED` + `BROADCAST_JOBS`.

| Scheduler / job | Note |
|---|---|
| `scanAbandonedCarts` | ecommerce |
| `dispatchProfileSnapshots` | channel profile snapshots (see E5) |
| `refreshChannelTokens` (+ id `refreshShortLivedChannelTokens`) | Zalo / TikTok / Meta token refresh (see E6) |
| `reconcileMetaCatalogSyncs` | Meta catalog |
| `purgeWorkspaces` | workspace deletion after the grace period |
| `purgeExpiredContactExports` | cloud only |
| `clearExpiredSupportAccess` | support access is not ported (cloud) |
| `syncUserQuota`, `reconcileTenants`, `unsubscribeExpiredTrials`, job `teardownExpiredTrial` | cloud only (`CLOUD_ONLY_SCHEDULERS`) |
| `refreshLicense` | enterprise license |
| `maintainMacPartitions`, `maintainLeaderboardPartitions` | Postgres partitions: probably not applicable to SQLite |
| `maintainLeaderboardRankings` | Redis sorted set: probably not applicable (leaderboards read the DB) |

### D3. Flow steps without a handler (9)

They run as an implicit success with the warning "Flow step handler is not
ported yet" (`crates/worker/src/steps/mod.rs:208`). Original:
`apps/worker/src/integration/handlers/flow-utils.ts` (`flowStepHandlers`).

`getUserData`, `questionnaires`, `setUpCoupon`, `markCouponUsed`, `callApi`
(waits for the external-request service, `steps/tools.rs:4`),
`facebookCustomAudience`, `sendMetaCapiEvent` (also the trigger action,
`trigger.rs:680`), `updateMessengerContactData`, `sendCollection` (skipped,
`send_flow_step.rs:399`). `executeJavascript`: dropped.

### D4. Job families left inside ported handlers (2)

- Low queue comment extras (`crates/worker/src/low.rs:10`): deferred TikTok
  private reply, live-comment pacing, leaderboard award, post tracking,
  commenter avatars, comment attachments, user-tag tracking. Original:
  `apps/worker/src/low/`.
- WhatsApp webhook: automatic events, identity changes, account updates
  (`crates/worker/src/channels/whatsapp_webhook.rs:304`). Original:
  `integrations/whatsapp` webhook parser.

## E. Gaps inside ported code (22 features)

| # | Feature | Rust place | Effect |
|---|---|---|---|
| E1 | Device files in message sends | `api/src/handlers/contact_messages.rs:230`, `handlers/messages.rs` | 501 `notPorted` |
| E2 | New attachment on message edit | `api/src/handlers/messages.rs:200` | 501 `notPorted` |
| E3 | Product messages (catalog resolution) | `business/src/message.rs:478` | 501 |
| E4 | Comment-automation scans for Instagram / Threads / TikTok | `business/src/comment_automation.rs:484`; contact scan Instagram adapter `worker/src/contact_scan.rs:7` | scan fails |
| E5 | On-demand channel profile fetch (Graph, Zalo, Telegram) | `business/src/contact/profile_refresh.rs:461`, `dynamic_image/mod.rs:403`, worker `variables.rs:599` (`getProfileSnapshot`), `leaderboard.rs:163` | fetch fails / stored snapshot |
| E6 | OAuth token refresh and OAuth reconnect for providers other than Messenger | `business/src/connection/mod.rs:391,399` | `connectionNotConfigured` |
| E7 | Public `connections.create` for OAuth providers without an adapter (`instagram`, `instagramFacebook`, `threads`, `googleAds`, `googleCalendar`, `googleSheets`) | `api/src/handlers/connections.rs:442`, `business/src/connection/oauth.rs` (adapters: messenger, tiktok, zalo) | "Operation not ported yet" |
| E8 | Thread control (Messenger handover / standby) | DECISIONS Q.I1b; `worker/src/thread_control.rs:223` | events dropped |
| E9 | Ads attribution on inbound (Google click, ad source platform, CTWA `keywordMatched`) | `worker/src/inbound.rs:572`, `channels/whatsapp.rs:894,989`, `ai_agent.rs:185` | no attribution |
| E10 | Inbound side effects: incoming notification job, `message:received` event, photo re-encode, delivery / read state | `worker/src/inbound.rs:1042,1492`; status branch `channels/whatsapp_webhook.rs:277` | not emitted |
| E11 | Media URL presigning (`resolveMediaUrl`) | `business/src/message.rs:120` | raw URLs (see C5) |
| E12 | Email-topic tracking (recipient, open pixel, signed click URLs) | `worker/src/steps/email_marketing.rs:98` | no tracking (see C7, C8, D1) |
| E13 | AI file heavy jobs other than `processAIFile` | `worker/src/ai_file.rs:99` | see D1 heavy |
| E14 | Comment-automation bulk-tag stats | `business/src/contact/bulk_tag_stats.rs:237` | empty list |
| E15 | Shopify / WooCommerce external store links and external checkout | `api/src/rpc_procedures/products.rs:356`, `business/src/webshop.rs:7` | `webshopClosed` |
| E16 | Tenant / white-label: custom domains, channel visibility, tenant platform credentials | `api/src/spa.rs:683,925`, `rpc_procedures/products.rs:524`, `handlers/webshop.rs:143`, `worker/src/channels/whatsapp_webhook.rs:683` | platform defaults only |
| E17 | Support-access sessions | `business/src/mcp_oauth.rs:5`, `rpc/actions.rs:69`, `rpc_procedures/whatsapp_calls.rs:10` | always off |
| E18 | Cloud owner quota / blocked owner answers, edition plan policy, license check | `rpc/actions.rs:11`, `business/src/inbound_webhook.rs:418`, `broadcast/plan_policy.rs:52`, `session/oauth.rs:1002` | never blocks |
| E19 | Messenger template Mini App button, template echo text | DECISIONS Q.D1m; `worker/src/thread_control.rs:223` | button dropped |
| E20 | Provider error log (`logProviderError`) for Meta catalog | `worker/src/meta_catalog/import.rs:635`, `sync.rs:623` | not logged |
| E21 | Sequence / broadcast stats inputs of `analyticsRoutes` | `api/src/rpc_procedures/analytics.rs:26` | see B |
| E22 | MCP OAuth `request` / `request_uri` params | `api/src/session/mcp_oauth.rs:22` | rejected (no client needs them) |

Comment-only notes that block nothing (zod schemas, id generator, retries)
are left out.

## Proposed split (6 units)

Sizes: S < 10 small items, M 10-20, L > 20 or several large features.

| Unit | Scope | Items | Size |
|---|---|---|---|
| U1 Messages and media | B1 `messagesAPI.getWhatsappFlowResponseAuthenticatedAPI`, `conversationsAPI.getPostDetailsAuthenticatedAPI`, `channelPostAPIs.*` (2); B3 `changeMessageAttributes`, `deleteMessage`, `editMessage`, `sendWhatsappTemplateAction`; C5 media attachment proxy; E1 device files, E2 attachment on edit, E3 product messages, E11 media presigning | 13 | M |
| U2 Channel connect and Meta catalog | B1 `integrationWhatsappAPIs.connectWhatsappNumberAPI`, `listIntegrationWhatsappInternalAPI`, `messengerMessageTemplateAPIs.listMessengerMessageTemplatesInternalAPI`, `flowsAPI.privateGetMessengerAdsJsonAPI`; B3 `connectTelegramAction`, `reconnectWhatsappAction`, `connectMetaCatalogAction`; C9 WhatsApp callback, C13 `metaCapi` flow, C14 `metaCatalog` flow; E7 OAuth adapters; D2 `reconcileMetaCatalogSyncs` | 12 | M |
| U3 Tokens, profiles, avatars | B3 `refreshAllChannelTokensAction`, `refreshContactProfileAction`; E5 channel profile fetch (+ `getProfileSnapshot`); E6 OAuth token refresh / reconnect; D2 `refreshChannelTokens` (2 ids), `dispatchProfileSnapshots`; D1 `updateContactAvatar`; C6 avatar proxy | 9 | M (provider APIs) |
| U4 Flow runtime and AI jobs | D3 9 steps; D1 ai-agent (4), heavy (3), `checkOutboundAutomatedResponse`, `captureTemplateFlowResponse`, thread control (2); B3 `testExternalRequestAction` (same external-request service as `callApi`); E8, E13, E19 | 24 | L |
| U5 Exports, email tracking, housekeeping | A `sequences.listStepContacts`; B1 `productsAPI.listProductsAuthorizedAPI`; B3 `forceChangePasswordAction`, `assertCurrentUserCanAccessChatbot`; C1-C4 exports and templates, C7-C8 email-topic, C10-C12 extension download + API docs; D1 `recordEmailTrackingTransition`, default queue (5); D2 `scanAbandonedCarts`, `purgeWorkspaces`; E12, E21 | 24 | L (many small) |
| U6 Comments, ads, inbound extras | E4 Instagram / Threads / TikTok scans; D4 low-queue extras + WhatsApp automatic events; E9 ads attribution; E10 inbound side effects; E14 bulk-tag stats; E20 provider error log | 7 features | L |

Not in a unit (decide first): B2 53 procedures with no SPA caller (port
only for a named client), B4 16 dead SPA stubs (delete), cloud / enterprise
items (D2 7, E16, E17, E18), D2 3 probably n/a, E22.
