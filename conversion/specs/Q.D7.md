# Q.D7 — Ads

Branch `feat/conversion-d7` (from `conversion` at `1b7521097`). Follows
P3.6-D2. Slice 1 of the unit: the read and connection routes with parity
tests, the lead-ads job and the ad-touch log. Later slices: marketing
messages, the ad campaign writes, the Google Ads connect and conversion
delivery. The web pages and the Google Ads producers are left (see "Not done").

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
| 9 | Facebook marketing messages (16 routes, delivery, worker) | `business::marketing_messages` | `features/facebook-marketing-messages` |
| 10 | `ads.*` campaign writes (create / list / retry / publish / pause / delete / insights, ad accounts, pages, image / video upload, video status) and `ads.startRetargetAudienceSync` + the `syncRetargetAudience` job | `business::ads::{campaigns,retarget}`, `api::handlers::ads_campaigns`, `worker::integration` | `features/ads-campaign`, `messaging-ads` |
| 11 | Google Ads OAuth connect (adapter `googleAds`), the settings actions (start / reconnect / pick / cancel / disconnect / sync) and `googleAdsAPI.getIntegration` / `getInFlightConnectSession` | `business::{google_ads_api,google_ads_setup}`, `connection::oauth_google_ads`, `api::rpc_procedures::google_ads` | `integrations/google-ads`, `integration-google-ads/service.ts`, `features/integration-google-ads/actions` |
| 12 | Google Ads conversion record + delivery (Data Manager `events:ingest`, legacy `uploadClickConversions`) and the `sendGoogleAdsConversion` job | `business::google_ads_conversion`, `worker::integration` | `google-ads/{record-conversion,delivery}.ts`, `apis/*` |

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

- Google Ads producers: the `sendGoogleAdsConversion` flow step and trigger
  action (template resolution, consent setting, error branches) still skip
  as `notPorted`; `record` has no caller yet.
- Google Ads housekeeping: the processing-status poll
  (`requestStatus:retrieve`), the stranded / stale-claim sweeps, the daily
  conversion-action sync, `googleAdsAPI.listEvents` (procedure) and
  customer matching (hashed e-mail / phone at delivery).
- Web SPA ads pages (`specs/P4-web.md` §6): `dashboard/ads/*` still shows
  "not available yet".
- A `messaging_referrals` event without a message records no ad touch: the
  Rust `Received` has no provider time (`occurredAt`) to key it.
