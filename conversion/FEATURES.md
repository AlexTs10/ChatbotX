# Features

User decision 2026-11-11: **drop** the Execute JavaScript flow step; **keep** all other features (see DECISIONS.md).
Inventory made by the P0.4 analyst subagent. Test files were assigned by path and file-name keyword, so a test that touches two features can be in the neighbouring row.

## Features

| Feature | Location (main paths) | LOC | Test files | Decision | Decided on |
|---|---|---|---|---|---|
| WhatsApp channel | `apps/builder/src/features/integration-whatsapp`; `integrations/whatsapp`; `packages/database/src`; `packages/business/src/integration-whatsapp` (+6 more dirs) | 42782 | 124 | keep | 2026-11-11 |
| WhatsApp message templates | `packages/flow-config/src`; `apps/worker/src/chat`; `apps/builder/src/features/flows`; `apps/worker/src/integration` (+2 more dirs) | 3339 | 13 | keep | 2026-11-11 |
| WhatsApp Flows (forms) | `packages/business/src/whatsapp-flow-response`; `packages/business/src/whatsapp-flow`; `packages/flow-config/src`; `apps/worker/src/integration` (+1 more dirs) | 1278 | 10 | keep | 2026-11-11 |
| WhatsApp calling (VoIP) | `packages/business/src/whatsapp-call`; `apps/worker/src/integration`; `packages/database/src`; `apps/builder/src/features/whatsapp-calls` (+5 more dirs) | 9700 | 93 | keep | 2026-11-11 |
| Coexist (existing-app history sync) | `apps/worker/src/integration`; `packages/database/src`; `packages/business/src/coexist-import`; `packages/business/src/coexist` (+1 more dirs) | 8999 | 34 | keep | 2026-11-11 |
| Messenger channel | `integrations/messenger`; `apps/builder/src/features/integration-messenger`; `packages/database/src`; `apps/builder/src/features/flows` (+14 more dirs) | 23046 | 88 | keep | 2026-11-11 |
| Instagram channel | `integrations/instagram`; `integrations/instagram-facebook`; `apps/builder/src/features/integration-instagram`; `packages/database/src` (+5 more dirs) | 12124 | 74 | keep | 2026-11-11 |
| Threads channel | `integrations/threads`; `packages/business/src/integration-threads`; `apps/builder/src/features/integration-threads`; `apps/worker/src/schedule` (+3 more dirs) | 2627 | 15 | keep | 2026-11-11 |
| TikTok channel | `integrations/tiktok`; `apps/builder/src/features/integration-tiktok`; `packages/business/src/integration-tiktok`; `packages/flow-config/src` (+3 more dirs) | 5330 | 23 | keep | 2026-11-11 |
| Zalo channel | `integrations/zalo`; `apps/builder/src/features/integration-zalo`; `packages/business/src/integration-zalo`; `packages/database/src` (+4 more dirs) | 3836 | 19 | keep | 2026-11-11 |
| Telegram channel | `integrations/telegram`; `apps/builder/src/features/integration-telegram`; `packages/business/src/integration-telegram`; `apps/worker` (+4 more dirs) | 3854 | 12 | keep | 2026-11-11 |
| Webchat channel | `apps/builder/src/features/integration-webchat`; `packages/business/src/integration-webchat`; `apps/builder/src/app`; `apps/builder/src/lib` (+3 more dirs) | 5402 | 20 | keep | 2026-11-11 |
| API channel integration (custom channels) | `apps/builder/src/features/integration-api`; `integrations/api`; `packages/business/src/integration-api`; `packages/database/src` | 1915 | 5 | keep | 2026-11-11 |
| ChatbotX integration (cross-instance client) | `integrations/chatbotx` | 58 | 1 | keep | 2026-11-11 |
| Facebook Marketing Messages | `apps/builder/src/features/facebook-marketing-messages`; `packages/business/src/facebook-marketing-message-delivery`; `packages/database/src`; `packages/business/src/facebook-marketing-message-audience` (+5 more dirs) | 11530 | 39 | keep | 2026-11-11 |
| Facebook Lead Ads automation | `apps/builder/src/features/facebook-lead-ad-automation`; `apps/worker/src/integration`; `packages/business/src/facebook-lead-ads-automation`; `packages/database/src` (+1 more dirs) | 2524 | 5 | keep | 2026-11-11 |
| Comment automation (FB/IG/Threads/TikTok) | `apps/worker/src/integration`; `apps/builder/src/features/fb-comments`; `apps/builder/src/features/ig-comments`; `apps/builder/src/features/ig-stories` (+10 more dirs) | 22685 | 45 | keep | 2026-11-11 |
| Thread control (handover protocol) | `packages/business/src/thread-control`; `apps/worker/src/integration`; `packages/database/src`; `apps/worker/src/chat` (+2 more dirs) | 2471 | 30 | keep | 2026-11-11 |
| Channel connection and OAuth connect flow | `apps/builder/src/features/channel-connect`; `apps/builder/src/app`; `packages/business/src/connection`; `packages/connections/src` (+15 more dirs) | 25664 | 115 | keep | 2026-11-11 |
| Shopify integration | `packages/business/src/shopify`; `integrations/shopify`; `apps/builder/src/features/shopify`; `packages/database/src` (+2 more dirs) | 3797 | 6 | keep | 2026-11-11 |
| WooCommerce integration | `packages/business/src/woocommerce`; `integrations/woocommerce`; `apps/builder/src/features/woocommerce`; `packages/database/src` (+2 more dirs) | 3815 | 6 | keep | 2026-11-11 |
| Mailchimp integration | `apps/builder/src/features/flows`; `integrations/mailchimp`; `apps/worker/src/integration`; `apps/builder/src/features/integration-mailchimp` (+4 more dirs) | 1364 | 1 | keep | 2026-11-11 |
| Klaviyo integration | `integrations/klaviyo`; `apps/builder/src/features/flows`; `apps/builder/src/features/integration-klaviyo`; `apps/worker/src/integration` (+4 more dirs) | 1235 | 10 | keep | 2026-11-11 |
| ActiveCampaign integration | `integrations/active-campaign`; `apps/builder/src/features/flows`; `apps/builder/src/features/integration-active-campaign`; `apps/worker/src/integration` (+4 more dirs) | 1842 | 7 | keep | 2026-11-11 |
| Drip integration | `apps/builder/src/features/flows`; `integrations/drip`; `apps/builder/src/features/integration-drip`; `apps/worker/src/integration` (+4 more dirs) | 1361 | 4 | keep | 2026-11-11 |
| GetResponse integration | `integrations/get-response`; `apps/builder/src/features/flows`; `apps/builder/src/features/integration-get-response`; `apps/worker/src/integration` (+4 more dirs) | 1305 | 6 | keep | 2026-11-11 |
| MailerLite integration | `integrations/mailer-lite`; `apps/builder/src/features/flows`; `apps/builder/src/features/integration-mailer-lite`; `apps/worker/src/integration` (+4 more dirs) | 1322 | 9 | keep | 2026-11-11 |
| Moosend integration | `integrations/moosend`; `apps/builder/src/features/flows`; `apps/builder/src/features/integration-moosend`; `apps/worker/src/integration` (+4 more dirs) | 1233 | 10 | keep | 2026-11-11 |
| SendGrid integration | `integrations/sendgrid`; `apps/builder/src/features/flows`; `apps/builder/src/features/integration-sendgrid`; `apps/worker/src/integration` (+4 more dirs) | 1454 | 8 | keep | 2026-11-11 |
| SMTP email integration | `apps/builder/src/features/integration-smtp`; `apps/builder/src/features/flows`; `apps/worker/src/integration`; `packages/business/src/integration-smtp` (+5 more dirs) | 2552 | 5 | keep | 2026-11-11 |
| Google Sheets integration | `apps/builder/src/features/spreadsheets`; `apps/builder/src/features/flows`; `apps/worker/src/integration`; `packages/flow-config/src` (+7 more dirs) | 4744 | 12 | keep | 2026-11-11 |
| Google Calendar integration | `apps/builder/src/features/external-calendars`; `integrations/google-calendar`; `packages/business/src/appointment-external-calendar`; `apps/worker/src/default` (+2 more dirs) | 2342 | 6 | keep | 2026-11-11 |
| Google Ads integration | `apps/builder/src/features/integration-google-ads`; `packages/business/src/google-ads`; `integrations/google-ads`; `packages/database/src` (+8 more dirs) | 16568 | 95 | keep | 2026-11-11 |
| Facebook Ads integration | `integrations/facebook-ads`; `apps/builder/src/features/integration-facebook-ads`; `apps/worker/src/integration`; `packages/business/src/integration-facebook-ads` (+4 more dirs) | 5749 | 35 | keep | 2026-11-11 |
| Meta Conversions API (CAPI) | `apps/builder/src/features/meta-conversions`; `packages/business/src/meta-conversions`; `packages/database/src`; `apps/worker/src/integration` (+5 more dirs) | 8863 | 41 | keep | 2026-11-11 |
| Meta Catalog | `packages/business/src/meta-catalog`; `integrations/meta-catalog`; `apps/worker/src/default`; `packages/database/src` (+1 more dirs) | 4558 | 20 | keep | 2026-11-11 |
| Make / n8n automation steps | `apps/builder/src/features/flows`; `apps/worker/src/integration`; `apps/builder/src/features/integration-make`; `packages/flow-config/src` | 551 | 4 | keep | 2026-11-11 |
| OpenAI integration | `apps/builder/src/features/integration-openai`; `integrations/openai`; `packages/business/src/integration-openai`; `packages/database/src` (+1 more dirs) | 640 | 1 | keep | 2026-11-11 |
| OpenAI-compatible integration | `apps/builder/src/features/integration-openai-compatible`; `packages/business/src/integration-openai-compatible`; `packages/database/src`; `apps/builder/src/app` | 1457 | 10 | keep | 2026-11-11 |
| Claude (Anthropic) integration | `apps/builder/src/features/integration-claude`; `packages/business/src/integration-claude`; `packages/database/src` | 366 | 0 | keep | 2026-11-11 |
| Gemini integration | `apps/builder/src/features/integration-gemini`; `packages/business/src/integration-gemini`; `packages/database/src` | 362 | 0 | keep | 2026-11-11 |
| DeepSeek integration | `apps/builder/src/features/integration-deepseek`; `packages/business/src/integration-deepseek`; `packages/database/src` | 370 | 0 | keep | 2026-11-11 |
| OpenRouter integration | `apps/builder/src/features/integration-openrouter`; `packages/business/src/integration-openrouter`; `packages/database/src` | 422 | 0 | keep | 2026-11-11 |
| Muse integration | `apps/builder/src/features/integration-muse`; `packages/business/src/integration-muse`; `packages/database/src`; `apps/builder/src/app` | 383 | 3 | keep | 2026-11-11 |
| AI agents (tools, MCP client, functions) | `packages/ai/src`; `apps/builder/src/features/ai-agents`; `apps/worker/src/integration`; `apps/builder/src/features/ai-mcp-servers` (+11 more dirs) | 15849 | 49 | keep | 2026-11-11 |
| AI knowledge base (RAG, AI files) | `apps/builder/src/features/ai-files`; `apps/worker/src/ai-agent`; `packages/database/src`; `packages/business/src/ai-file` (+3 more dirs) | 7562 | 20 | keep | 2026-11-11 |
| AI handover (human takeover) | `apps/worker/src/integration`; `apps/builder/src/features/integration-ai-handover`; `packages/database/src`; `packages/business/src/ai-handover-bulk-run` (+3 more dirs) | 6375 | 24 | keep | 2026-11-11 |
| AI flow steps (text, image, speech, extraction) | `apps/builder/src/features/flows`; `apps/worker/src/integration`; `apps/worker/src/heavy`; `packages/flow-config/src` (+3 more dirs) | 9119 | 33 | keep | 2026-11-11 |
| Flow builder and execution | `apps/builder/src/features/flows`; `packages/flow-config/src`; `apps/worker/src/integration`; `apps/builder/src/features/folders` (+12 more dirs) | 31008 | 65 | keep | 2026-11-11 |
| Flow steps: messages and buttons | `apps/builder/src/features/flows`; `packages/flow-config/src`; `apps/worker/src/integration`; `apps/worker/src/chat` | 5211 | 38 | keep | 2026-11-11 |
| Flow steps: contact and conversation actions | `apps/builder/src/features/flows`; `packages/flow-config/src` | 1366 | 7 | keep | 2026-11-11 |
| Flow steps: logic and utilities | `apps/builder/src/features/flows`; `packages/flow-config/src`; `packages/utils/src`; `apps/worker/src/integration` (+2 more dirs) | 4934 | 26 | keep | 2026-11-11 |
| Execute JavaScript flow step | `packages/business/src/javascript-execution`; `apps/javascript-executor`; `apps/builder/src/features/flows`; `packages/javascript-sandbox/src` (+1 more dirs) | 1605 | 12 | drop | 2026-11-11 |
| Triggers | `apps/builder/src/features/triggers`; `apps/worker/src/trigger`; `apps/builder/src/features/conditions`; `packages/business/src/trigger` (+3 more dirs) | 7902 | 24 | keep | 2026-11-11 |
| Automated responses and keywords | `apps/builder/src/features/automated-response`; `apps/worker/src/integration`; `packages/automated-response/src`; `packages/business/src/automated-response` (+7 more dirs) | 7598 | 27 | keep | 2026-11-11 |
| Decision engine (routing profiles) | `apps/builder/src/features/decision-profiles`; `packages/business/src/decision`; `apps/builder/src/features/decision-connections`; `apps/worker/src/integration` (+5 more dirs) | 8063 | 12 | keep | 2026-11-11 |
| Smart delay | `packages/business/src/smart-delay`; `apps/worker/src/integration`; `apps/worker/src/schedule`; `packages/database/src` | 1180 | 6 | keep | 2026-11-11 |
| Sequences | `apps/builder/src/features/sequences`; `packages/business/src/sequence`; `apps/worker/src/sequence-scheduler`; `packages/sequence-scheduler/src` (+8 more dirs) | 10212 | 33 | keep | 2026-11-11 |
| Broadcasts (incl. recurring) | `apps/builder/src/features/broadcasts`; `packages/business/src/broadcast`; `packages/database/src`; `apps/worker/src/schedule` (+6 more dirs) | 21538 | 102 | keep | 2026-11-11 |
| Inbound webhooks | `apps/builder/src/features/inbound-webhooks`; `packages/business/src/inbound-webhook`; `packages/database/src`; `apps/worker/src/default` (+3 more dirs) | 7553 | 22 | keep | 2026-11-11 |
| Outbound webhooks | `apps/builder/src/features/webhooks`; `apps/worker/src/webhook`; `apps/builder/src/features/flows`; `packages/business/src/webhook` (+7 more dirs) | 5787 | 24 | keep | 2026-11-11 |
| Template marketplace (install/share/sync) | `packages/business/src/template`; `apps/builder/src/features/templates`; `packages/database/src`; `packages/flow-config/src` (+9 more dirs) | 41570 | 167 | keep | 2026-11-11 |
| Inbox and live chat (conversations, messages) | `packages/database/src`; `apps/builder/src/features/conversations`; `apps/worker/src/integration`; `packages/business/src/conversation` (+19 more dirs) | 37304 | 104 | keep | 2026-11-11 |
| Saved replies | `apps/builder/src/features/saved-replies`; `packages/business/src/saved-reply`; `packages/database/src` | 1866 | 3 | keep | 2026-11-11 |
| Response SLA | `packages/business/src/response-sla`; `apps/builder/src/features/response-sla`; `packages/database/src` | 1323 | 4 | keep | 2026-11-11 |
| Working hours | `apps/builder/src/features/working-hours`; `packages/business/src/working-hours`; `packages/database/src`; `apps/builder/src/app` | 4890 | 21 | keep | 2026-11-11 |
| Contacts (profiles, notes, activity) | `apps/builder/src/features/contacts`; `packages/business/src/contact`; `packages/database/src`; `packages/business/src/contact-inbox` (+15 more dirs) | 24206 | 111 | keep | 2026-11-11 |
| Contact filters and segments | `apps/builder/src/features/contact-filter`; `packages/database/src`; `packages/flow-config/src`; `packages/utils/src` | 11277 | 20 | keep | 2026-11-11 |
| Custom fields | `apps/builder/src/features/custom-fields`; `packages/business/src/contact-custom-field`; `packages/utils/src`; `packages/business/src/custom-field` (+7 more dirs) | 7571 | 43 | keep | 2026-11-11 |
| System fields | `packages/business/src/system-field`; `apps/builder/src/features/system-fields`; `apps/builder/src/features/shared-fields`; `packages/database/src` | 864 | 2 | keep | 2026-11-11 |
| Bot fields | `apps/builder/src/features/bot-fields`; `packages/business/src/bot-field`; `packages/database/src` | 2922 | 9 | keep | 2026-11-11 |
| Tags and labels | `packages/business/src/tag`; `apps/builder/src/features/tags`; `apps/worker/src/default`; `apps/worker/src/integration` (+5 more dirs) | 5405 | 27 | keep | 2026-11-11 |
| Contact scan | `apps/builder/src/features/contact-scan`; `apps/worker/src/integration`; `packages/business/src/contact-scan`; `apps/worker/src/schedule` | 2541 | 12 | keep | 2026-11-11 |
| Pipelines (CRM deals) | `apps/builder/src/features/pipelines`; `packages/business/src/pipeline`; `packages/database/src`; `apps/builder/src/features/flows` (+5 more dirs) | 25306 | 81 | keep | 2026-11-11 |
| Imports and exports (contacts, products, flows) | `apps/worker/src/default`; `packages/flow-config/src`; `apps/builder/src/features/import`; `packages/imports/src` (+5 more dirs) | 7130 | 52 | keep | 2026-11-11 |
| Media library and file storage | `apps/builder/src/features/media-library`; `packages/utils/src`; `packages/filesystem/src`; `apps/builder/src/components` (+7 more dirs) | 6543 | 30 | keep | 2026-11-11 |
| Email topics and tracking | `apps/builder/src/features/email-topics`; `packages/business/src/email-topic`; `apps/builder/src/app`; `packages/database/src` (+2 more dirs) | 2543 | 10 | keep | 2026-11-11 |
| Ecommerce: products, categories, vendors | `apps/builder/src/features/products`; `packages/database/src`; `apps/builder/src/features/flows`; `apps/builder/src/features/product-categories` (+10 more dirs) | 16218 | 25 | keep | 2026-11-11 |
| Ecommerce: orders and payment providers | `packages/business/src/order`; `apps/builder/src/features/orders`; `packages/business/src/payment-provider`; `packages/database/src` (+6 more dirs) | 10029 | 15 | keep | 2026-11-11 |
| Ecommerce: webshop storefront | `apps/builder/src/features/webshop`; `packages/business/src/webshop`; `apps/builder/src/app` | 7831 | 9 | keep | 2026-11-11 |
| Ecommerce: settings, open hours, analytics | `packages/database/src`; `apps/builder/src/features/ecommerce-settings`; `packages/business/src/ecommerce-settings`; `apps/builder/src/features/ecommerce-open-hours` (+6 more dirs) | 4926 | 14 | keep | 2026-11-11 |
| Coupons and discount codes | `apps/builder/src/features/coupons`; `apps/builder/src/features/discount-codes`; `packages/database/src`; `packages/business/src/discount-code` (+5 more dirs) | 8287 | 14 | keep | 2026-11-11 |
| Appointments and booking | `apps/builder/src/features/appointment-calendars`; `packages/database/src`; `packages/business/src/appointment`; `packages/business/src/appointment-calendar` (+10 more dirs) | 13044 | 30 | keep | 2026-11-11 |
| Ads analytics and CTWA tracking | `apps/builder/src/features/ads`; `packages/business/src/ads-conversion`; `packages/database/src`; `packages/business/src/ads-analytics` (+4 more dirs) | 10603 | 31 | keep | 2026-11-11 |
| Ads campaigns (Meta messaging ads) | `apps/builder/src/features/ads-campaign`; `packages/business/src/messaging-ads`; `packages/database/src`; `packages/business/src/messaging-ads-connection` (+1 more dirs) | 8008 | 40 | keep | 2026-11-11 |
| Leaderboards | `apps/builder/src/features/leaderboards`; `packages/business/src/leaderboard`; `packages/database/src`; `apps/builder/src/app` (+8 more dirs) | 13874 | 23 | keep | 2026-11-11 |
| Minigames | `apps/builder/src/features/minigames`; `packages/business/src/minigame`; `packages/database/src`; `apps/builder/src/app` | 8192 | 14 | keep | 2026-11-11 |
| Mini-apps | `apps/builder/src/features/mini-apps`; `packages/mini-app/src`; `packages/minigame-ui/src`; `packages/business/src/mini-app` (+5 more dirs) | 19351 | 26 | keep | 2026-11-11 |
| Polls | `apps/builder/src/features/polls`; `packages/business/src/poll`; `packages/database/src`; `apps/worker/src/integration` (+2 more dirs) | 4149 | 13 | keep | 2026-11-11 |
| Questionnaires | `apps/builder/src/features/questionnaires`; `packages/business/src/questionnaire`; `apps/worker/src/questionnaires`; `packages/database/src` (+4 more dirs) | 5914 | 10 | keep | 2026-11-11 |
| Dynamic images | `apps/builder/src/features/dynamic-images`; `packages/business/src/dynamic-image`; `apps/builder/src/app`; `packages/database/src` | 4321 | 2 | keep | 2026-11-11 |
| Places near me | `apps/builder/src/features/places-near-me`; `packages/database/src`; `packages/business/src/place-near-me`; `apps/builder/src/features/flows` (+3 more dirs) | 2737 | 13 | keep | 2026-11-11 |
| Get user data webview | `apps/worker/src/integration`; `apps/builder/src/features/get-user-data-webview`; `apps/builder/src/features/flows`; `packages/business/src/get-user-data` (+1 more dirs) | 2770 | 4 | keep | 2026-11-11 |
| Public links (open-link, QR codes, reflinks) | `apps/builder/src/features/reflinks`; `apps/builder/src/features/qr-codes`; `packages/business/src/qr-code`; `apps/builder/src/app` (+3 more dirs) | 4391 | 6 | keep | 2026-11-11 |
| Workspaces | `apps/builder/src/features/workspaces`; `apps/builder/src/app`; `packages/business/src/workspace`; `packages/database/src` (+10 more dirs) | 13591 | 52 | keep | 2026-11-11 |
| Workspace members, roles and invitations | `apps/builder/src/features/workspace-members`; `packages/business/src/workspace-member`; `apps/builder/src/lib`; `packages/database/src` (+6 more dirs) | 4214 | 14 | keep | 2026-11-11 |
| Authentication (sign-in, social, mobile, magic links) | `packages/auth/src`; `apps/builder/src/features/auth`; `apps/builder/src/features/magic-links`; `apps/builder/src/lib` (+8 more dirs) | 8560 | 42 | keep | 2026-11-11 |
| Multi-tenancy (white-label tenants) | `apps/builder/src/enterprise`; `packages/business/src/enterprise`; `apps/builder/src/app`; `apps/builder/src/features/help-items` (+3 more dirs) | 4060 | 6 | keep | 2026-11-11 |
| Platform admin | `apps/builder/src/features/platform-credentials`; `packages/business/src/platform`; `packages/business/src/platform-credential`; `apps/builder/src/features/admin-workspaces` (+6 more dirs) | 7770 | 5 | keep | 2026-11-11 |
| Licensing, billing and quotas | `packages/payments/src`; `packages/business/src/user-quota`; `packages/business/src/quota-enforcement`; `packages/business/src/enterprise` (+8 more dirs) | 6820 | 37 | keep | 2026-11-11 |
| Workspace API tokens and public API | `apps/builder/src/lib`; `apps/builder/src/routers`; `apps/builder/src/middlewares`; `packages/business/src/workspace-api-token` (+5 more dirs) | 5466 | 27 | keep | 2026-11-11 |
| Capabilities discovery | `packages/business/src/capabilities`; `apps/builder/src/features/capabilities` | 670 | 1 | keep | 2026-11-11 |
| CLI (chatbotx command) | `apps/cli` | 1155 | 5 | keep | 2026-11-11 |
| MCP server | `apps/mcp-server` | 7604 | 15 | keep | 2026-11-11 |
| Realtime (WebSocket gateway) | `apps/realtime`; `packages/realtime-protocol/src`; `apps/builder/src/features/realtime`; `apps/builder/src/app` (+4 more dirs) | 5807 | 31 | keep | 2026-11-11 |
| Scheduler and scheduled jobs | `apps/worker/src/schedule`; `apps/worker/src/lib`; `packages/scheduler/src`; `apps/builder/src/components` | 1560 | 13 | keep | 2026-11-11 |
| Transactional mail (system emails) | `packages/mail/src` | 762 | 1 | keep | 2026-11-11 |
| Analytics dashboards | `packages/analytics/src`; `packages/analytics-nextjs/src`; `apps/builder/src/features/analytics`; `apps/builder/src/features/ecommerce-analytics` (+4 more dirs) | 19873 | 51 | keep | 2026-11-11 |
| Audit logs | `packages/business/src/audit`; `apps/worker/src/default`; `apps/worker/src/lib` | 416 | 7 | keep | 2026-11-11 |
| Error logs | `apps/builder/src/features/error-logs`; `packages/business/src/error-log`; `packages/utils/src`; `packages/database/src` (+2 more dirs) | 1843 | 10 | keep | 2026-11-11 |
| Workspace support access | `packages/business/src/workspace-support-access`; `apps/builder/src/components`; `apps/worker/src/schedule` | 441 | 6 | keep | 2026-11-11 |
| Mobile API and push notifications | `apps/worker/src/notification`; `packages/business/src/mobile-auth`; `packages/business/src/device-token` | 734 | 3 | keep | 2026-11-11 |
| i18n (UI translations) | `apps/builder/src/i18n`; `apps/builder/src/components`; `apps/builder/src/lib` | 375 | 6 | keep | 2026-11-11 |
| Developer queues dashboard | `apps/builder/src/app` | 94 | 0 | keep | 2026-11-11 |
| **Total features (114)** | | **830402** | **2905** | | |

## Shared infrastructure

| Feature | Location (main paths) | LOC | Test files | Decision | Decided on |
|---|---|---|---|---|---|
| Database (Drizzle, schema, repositories, sharding) | `packages/database/src`; `packages/database/scripts` | 4918 | 11 | keep | 2026-11-11 |
| Redis (cache, locks, queues, stores) | `packages/redis/src` | 1761 | 7 | keep | 2026-11-11 |
| Event bus and domain events | `packages/events/src`; `packages/event-bus/src` | 2938 | 10 | keep | 2026-11-11 |
| Logger | `packages/logger/src` | 271 | 3 | keep | 2026-11-11 |
| Encryption and signed tokens | `packages/encryption/src` | 939 | 11 | keep | 2026-11-11 |
| Variables (contact/bot/system variable interpolation) | `packages/variables/src`; `apps/builder/src/components` | 4839 | 25 | keep | 2026-11-11 |
| Utils (shared helpers) | `packages/utils/src`; `packages/business/src/net` | 2341 | 13 | keep | 2026-11-11 |
| UI component library | `packages/ui/src` | 20484 | 22 | keep | 2026-11-11 |
| Worker config (queues, job schemas) | `packages/worker-config/src` | 5058 | 20 | keep | 2026-11-11 |
| Worker runtime (boot, queues, shutdown) | `apps/worker/src/lib`; `apps/worker/src/default`; `apps/worker/src/low`; `apps/worker/src/env.ts` (+4 more dirs) | 1474 | 16 | keep | 2026-11-11 |
| Channel SDK and registry | `packages/sdk/src`; `packages/channel-registry/src` | 5850 | 15 | keep | 2026-11-11 |
| Business layer shared services | `packages/business/src/errors.ts`; `packages/business/src/index.ts`; `packages/business/src/count-mode.ts`; `packages/business/src/utils.ts` (+5 more dirs) | 990 | 8 | keep | 2026-11-11 |
| Builder app shell and shared components | `apps/builder/src/features/shared`; `apps/builder/src/components`; `apps/builder/src/lib`; `apps/builder/src/app` (+8 more dirs) | 9342 | 36 | keep | 2026-11-11 |
| Tooling configs | `packages/vitest-config/src` | 610 | 2 | keep | 2026-11-11 |
| **Total infrastructure (14)** | | **61815** | **199** | | |

## Notes

- LOC counting: `git ls-files` (excluding `conversion/`), keeping only `.ts`, `.tsx`, `.js` files; excluded `*.test.*`, `*.spec.*`, `__tests__/`, `*.d.ts`, `messages/*.json`, generated-looking files, `drizzle/`, `docs/`, `scripts/`, `fixtures/`, `*.config.ts`; raw line count (`wc -l`) of each file, summed per feature. Each file counts in exactly one row (first matching rule: directory rules first, then file-name keyword rules, then package fallback). Total counted source: 892217 lines (830402 in features, 61815 in infrastructure, 0 unmapped).
- SQL migrations (not in LOC): about 10776 lines in 308 `.sql` files under `packages/database/drizzle` and `packages/database/src/sharding/migrations`. They belong to the Database infrastructure row.
- Test files: each test file is counted in one row only. Test file = path matching `\.(test|spec)\.` (this includes 4 `.snap` snapshot files). Assignment is by directory first, then file-name keywords; cross-cutting tests may be in a neighbouring row.
- Total test files assigned: 3104 of 3104. Unassigned: 0.
- Execute JavaScript flow step: `drop`. All other rows: `keep`.

- Attribution caveats: database schema/repository/relation files are assigned to features by file-name keyword, so the Database infrastructure row holds only the generic DB code (client, sharding helpers, shared utils). Worker handler files and `flow-config` step files are also assigned by file-name keyword. `packages/variables/src/javascript-interpolation.ts` and its test `packages/variables/__tests__/interpolate-into-javascript.test.ts` are in the Variables row, not in the dropped feature, so check them when removing the JavaScript step. Tests in the "Builder app shell", "Worker runtime", "Database", "Utils" and "Business layer shared services" rows were assigned by package fallback only (no feature keyword matched).
