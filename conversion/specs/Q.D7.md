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
| 13 | Web ads screens (round 2, branch `feat/conversion-r2-ads`): `ads/*` redirects, `dashboard/ads/{$channel,google,conversion-events}`, `messaging-ads/*`, `fb-lead-ads/*`, `fb-marketing-messages/*`; the builder procedures `adsAPI.*`, `adsCampaignAPI.*`, `integrationFacebookAdsAPI.*`, `facebookLeadAdsAPI.*`, `facebookMarketingMessagesAPI.*`; the page queries and actions of those screens; `googleAds` conversion report (Google's own numbers); the Google Ads nav entry (`resolveGoogleAdsDashboardEntry`) | `api::rpc_procedures::ads`, `apps/web/src/routes/ads.tsx` | `app/space/[workspaceId]/{ads,dashboard/ads,messaging-ads,fb-lead-ads,fb-marketing-messages}`, `features/{ads,ads-campaign,facebook-lead-ad-automation,facebook-marketing-messages}` |
| 14 | Meta ads OAuth connects (`connectLeadAdsAction`, `connectMarketingMessagesAction`, `connectMessagingAdsAction`) and their callback flows (`facebookLeadAds` page leadgen subscription, `facebookMarketingMessages` grant, `messagingAds` connection) | `business::ads::connect`, `api::connect_callback` | `*/actions/connect*.action.ts`, `integrations/[...integration]/callback{,-stores}.ts` |
| 15 | Google Ads producers: the `sendGoogleAdsConversion` flow step and trigger action (consent setting, `{{variable}}` fields, error branches and Error Log lines); customer matching at delivery (hashed e-mail / phone, `encoding: HEX`, only with accepted customer-data terms); the blocked-owner check of the delivery job; the Zalo Google-click lookup (`profileLookup`) | `worker::{google_ads_producer,google_click_lookup}`, `business::google_ads_conversion` | `google-ads/{resolve-conversion-inputs,google-ads-input-error,send-conversion,resolve-matching-templates}.ts`, `customer-matching.ts`, `hash-user-data.ts`, `google-click-lookup.ts`, `click-lookup.ts` |
| 16 | Google Ads housekeeping: the processing-status poll (`requestStatus:retrieve`), the stranded / stale-claim sweep, the blocked-owner park, the daily setup sync; crons `googleAdsHousekeeping` (`*/10`) and `googleAdsSyncSetups` (03:30) | `business::google_ads_housekeeping`, `worker::google_ads_housekeeping`, `jobs::Enqueuer::job_state` | `google-ads/{housekeeping,processing-status,redrive,send-queue}.ts`, `schedule/handlers/google-ads-housekeeping.ts` |

## Security

- Customer matching hashes (SHA-256 hex) the normalised e-mail / phone at
  delivery, reads the current contact value, never logs it, and sends user
  data only when the account accepted the customer-data terms. A resolver
  failure retries the delivery (identifiers are not dropped).
- The housekeeping logs only enum-shaped Google reasons (`^[A-Z0-9_]{1,80}$`,
  else `UNKNOWN`), at most 1000 characters; a poll error is sanitised
  without the click id.
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
- The ads screens: `dashboard/ads/*` and `messaging-ads/$channel` are super
  admin only (404 page, as `resolveGuardedWorkspaceId(..., "superAdmin")`);
  every page query checks the role again in Rust (`systemError` 400).
- The Meta ads connects use a signed connect session (provider `messenger`,
  purpose = the flow, `state = {sessionId}.{nonce}`, nonce stored hashed)
  instead of the original's unsigned base64 JSON state. The callback finishes
  a session only for the member who started it (signed-in, still a member);
  `messagingAds` also needs a super admin and an integration of the
  workspace and channel (else 404, as the original). The return path is
  fixed by the action (relative, `validate_return_url`): no open redirect.
  Tokens are stored encrypted (`chatbotx_core::encryption`, no AAD, as the
  original). One callback per session (`claim_authorization`).

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

- The Meta CAPI producers (`sendMetaCapiEvent` flow step and trigger action)
  still skip as `notPorted` (the CAPI send of the channels, Q.I1a / Q.I1b).
- Zalo / Telegram profile reads on contact creation (`persistNewContactGoogleClick`
  runs as the existing-contact lookup instead; see DECISIONS).
- TEST_MAP: 154 class C rows of the six ads sections are still `todo`
  (mostly builder procedure tests and the `integrations/facebook-ads`,
  `integrations/google-ads` and `meta-conversions` adapter tests).
- `reconnectWhatsappAction` / `persistReconnectAuthAndResubscribe` (the
  WhatsApp CAPI-scope reconnect button of the ads account switcher): the
  WhatsApp embedded-signup reconnect has no Rust port yet (WhatsApp channel
  domain, Q.I1a); the button answers the generic action error.
- The channel CAPI tabs `whatsapps|messengers|instagrams/$id/ads` and the
  legacy `facebookAds` / `metaCapi` callback flows (channel pages, Q.I1a /
  Q.I1b).
- A `messaging_referrals` event without a message records no ad touch: the
  Rust `Received` has no provider time (`occurredAt`) to key it.
