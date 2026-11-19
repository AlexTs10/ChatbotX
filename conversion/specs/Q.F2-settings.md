# Q.F2-settings: workspace settings screens and the pricing page

Branch `feat/conversion-r2-settings` (from `conversion` 672ea7e5f).

## Done

All the `todo` rows under `app/space/[workspaceId]/(settings)/settings` in
`P4-routes.tsv` that this unit owns are `ported`. Each page has its
`// page:` marker in `apps/web/src/routes/settings.tsx` (pricing: `routes/logs.tsx`).

| Screen | Rust | Web |
| --- | --- | --- |
| Email-marketing API-key integrations (ActiveCampaign, Drip, GetResponse, Klaviyo, Mailchimp, MailerLite, Moosend, SendGrid) | `rpc_procedures/settings_integrations.rs` over `connection::api_key` (`allow_update`) | manage components + actions |
| Make, ChatbotX MCP (grants list / revoke), PayPal connect, SMTP CRUD | `rpc_procedures/settings_integrations.rs` | manage components |
| TikTok / Zalo / Threads channels (list, disconnect, reconnect, TikTok token refresh, comment-to-message) | `channel_integration.rs`, `connection/oauth_channels.rs` (Threads adapter) | `features/integration-{tiktok,zalo,threads}` |
| Google Ads (page, consent, retry, validate request, events list, connect session) | `google_ads.rs`, `google_ads_conversion.rs`, `rpc_procedures/google_ads.rs` | `features/integration-google-ads` |
| Facebook Ads (connect over a signed connect session, disconnect, ad accounts, custom audiences) | `connection/oauth_facebook_ads.rs`, `ads/graph.rs::cached_ad_accounts`, `rpc_procedures/facebook_ads.rs`, `connect_callback.rs` | `features/integration-facebook-ads` |
| AI Router (decision connections: list, create, update, toggle, test) | `decision_connection.rs`, `rpc_procedures/decision_connections.rs` | `features/decision-connections` |
| `(enterprise)/pricing` | none (static page) | `routes/logs.tsx::PricingPage` |

Security: every procedure checks membership and the original role
(super admin for the managing actions; the Google Ads and Facebook Ads
pages are member-level, as the originals). Credentials are sealed
(`email_marketing::seal_auth`, decision credential AAD
`decision-connection:{ws}:{id}`). The decision endpoint test is SSRF-safe
(`ssrf_guard`, pinned resolve, no redirects, 2.5 s, 256 KiB) and throttled
(30 s per connection, as the original). The Facebook Ads OAuth state is a
signed connect session bound to the user and the workspace.

Tests: `crates/business/tests/{channel_integration_settings,connection_oauth_facebook_ads,decision_connection,google_ads_conversion,connection_credentials,connection_oauth_channels}.rs`,
`crates/api/tests/{rpc_settings_integrations,rpc_google_ads_settings}.rs`.
TEST_MAP rows set to `ported` (see the `Q.F2-settings` notes).

Parity suites that cover the work (not run here): p1-05 connections,
p1-07 decisionConnections, p1-08 facebookLeadAds / ads, p1-09 googleAds,
p1-12 paymentProviders, p1-15 smtpIntegrations, p1-16 tiktokChannels,
p1-18 zaloChannels.

## Left, and why

- Google Sheets and Response SLA settings: unit d13a. WhatsApp / Messenger /
  Instagram channel pages: unit metach. Admin / manage pages: not this unit.
- The public `/v1` `decisionConnections.*` routes have no handlers: they are
  not a settings screen (the screen uses `/rpc`). Parity p1-07 covers them.
- `getAdInsights` (Facebook Ads) is not ported: the web does not call it.
- `adapters.test.ts` :25 / :40 (`compileDecisionProfileRequest`) belong to the
  decision-profile runtime, not to a settings screen.
- The channel OAuth start from `/channels/create` belongs to the channels unit.
  A legacy Threads / Zalo row without a `Connection` cannot reconnect from
  settings (it must connect again).
- The TikTok `auth.saved` step is approximated by the token refresh update.
- The pricing page shows the sample plans and only logs the selection, as
  the original (its upgrade call is commented out).
