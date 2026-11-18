# Q.D7 — Ads

Branch `feat/conversion-d7` (from `conversion` at `1b7521097`). Follows
P3.6-D2. Slice 1 of the unit: the read and connection routes with parity
tests, the lead-ads job and the ad-touch log. Marketing messages, the ad
campaign writes and the web pages are left (see "Not done").

## Slices

| # | Slice | Rust | Original |
|---|-------|------|----------|
| 1 | `googleAds.getStats` / `listEvents` / `getConnection` (read only) | `business::google_ads`, `api::handlers::google_ads` | `features/integration-google-ads/api/public.ts` |
| 2 | Channel CAPI routes of Messenger, Instagram, WhatsApp (`setCapiDataset`, `disconnectCapi`, `setCapiTestEventCode`, `sendCapiTestEvent`) | `business::meta_capi`, `api::handlers::capi` | `features/*-channel/api/public` CAPI procedures, `packages/business/src/meta-capi` |
| 3 | `facebookLeadAds.*` (7 routes) + worker `processLeadgen` | `business::facebook_lead_ads`, `api::handlers::facebook_lead_ads`, `worker::lead_ads` | `features/facebook-lead-ads`, `apps/worker/.../leadgen.ts` |
| 4 | `ads.*` conversion rules (list / get / create / update / updateRuleLegacy / toggleRuleStatus / delete), `referralStats`, `listReferralAds` | `business::ads::{rules,referral}`, `api::handlers::ads` | `features/ads/api/public/*` |
| 5 | `ads.getFunnel` / `getFunnelTimeseries` / `getCapiDelivery` / `listConversionExportRows` / `getConversion` | `business::ads::funnel` | `ads-conversion/service.ts` |
| 6 | `ads.getAnalyticsOverview` / `getAnalyticsTimeseries` / `listChannelAdAccounts` / `listCustomAudiences` | `business::ads::{analytics,graph}` | `facebook-ads` analytics, `messaging-ads-connection` |
| 7 | `ads.checkCampaignPrerequisites` / `listConnections` / `disconnectConnection` | `business::ads::graph` | `features/ads-campaign/api/public.ts`, `messaging-ads-connection/service.ts` |
| 8 | Ad-touch log of inbound referrals (`recordAdTouches`: Meta + Google touch, `adClicked` activity) | `business::contact_referral_event::record_ad_touches_contained`, `worker::inbound` | `contact-referral-event/ad-touch.ts`, `service.ts` |

## Security

- Every route is a workspace-token route of scope `ads` (route table); the
  workspace id comes from the token, every query is scoped to it.
- Lead-ads `create` subscribes the page to `leadgen` before it stores the
  row; the Graph token is the page token of the workspace's Messenger
  integration. `debug_token` uses the app token `clientId|clientSecret`.
- CAPI sends only the fields the original sends; user data is hashed as the
  original hashes it (`chatbotx_core::utils::meta_capi`).
- Graph calls go through `HttpUpstream` (fixed `graph.facebook.com` host,
  SSRF-safe client). Path ids are percent-encoded.
- `IntegrationFacebookAds` / `MessagingAdsConnection` auth is decrypted with
  `chatbotx_core::encryption` (no AAD, as the original); a connection whose
  auth does not decrypt is marked `invalid` (reconnect needed, 409).
- `disconnectConnection` revokes the token (`DELETE me/permissions`) best
  effort, then always deletes the row; a corrupt auth makes no Graph call.

## Decisions

See `conversion/DECISIONS.md` (2026-11-17 "Q.D7" lines): Google Ads read
only; in-process TTL cache for Graph reads; no report statement timeout;
CAPI path ids percent-encoded; WhatsApp CAPI dataset from the connect token
only; referral-only events record no ad touch.

## Tests

- Unit tests: `facebook_lead_ads` (field targets), `worker::lead_ads`
  (PSID of the inbox URL), `ads::graph` (cache, dedupe), `ads::analytics`
  (merge), `contact_referral_event::ad_touch_tests` (Meta / Google touch
  keys, organic referral).
- Parity fixtures: `parity/stacks/rust/ext/p1-02.ts` (place id 11),
  `p1-08.ts` (place id 10: Messenger channel, marketing-message rows),
  `p1-09.ts` (Google Ads).

## Parity (to run)

p1-09 `45`; p1-10 `21`; p1-11 `51`; p1-08 `07`; p1-02 ads rules, referral,
funnel, analytics and connections suites. Not run in this unit (orchestrator
runs parity).

## Not done

- Marketing messages: the 16 `facebookMarketingMessages.*` routes, the
  delivery service and the worker `processMarketingMessageWebhook` (about
  5k lines of TS; p1-08 `30`–`34`). The Rust p1-08 fixtures for them are in.
- Ad campaign writes: `ads.createCampaign` / `listCampaigns` / retry /
  publish / pause / delete / insights, campaign ad accounts, ad account
  details, image / video upload, video status, messenger pages, and
  `ads.startRetargetAudienceSync` (202 + upload job).
- Google Ads conversion upload (the flow step) and the Google Ads OAuth
  connect.
- Web SPA ads dashboard pages (`specs/P4-web.md` §6): still "not available
  yet".
- A `messaging_referrals` event without a message records no ad touch: the
  Rust `Received` has no provider time (`occurredAt`) to key it.
