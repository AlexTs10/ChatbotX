# Q.F2-automation — automation screens

Branch `feat/conversion-r2-autoweb` (from `conversion` at `672ea7e5f`).
The "todo" automation rows of `P4-routes.tsv`, ported from `apps/builder`
with the Rust procedures and actions they need. 33 rows are now `ported`.

## Screens

| Screens | SPA | Rust |
|---------|-----|------|
| Broadcasts: list (table + calendar), create, `[broadcastId]/edit`, `[broadcastId]/runs` | `routes/broadcasts.tsx` | `rpc_procedures/broadcasts.rs`: page queries `listBroadcasts`, `listBroadcastsForCalendar`, `findDraftBroadcast`, `findRecurringBroadcast`, `findIntegrationWhatsappInboxId`; `broadcastAPIs.privateGetBatchBroadcastStatsAPI` |
| Broadcast audience (create form count + preview dialog) | copied components | `rpc_procedures/contacts.rs`: `contactsAPIs.countContactInboxesAuthenticatedAPI`, `listContactInboxesAudiencePreviewAuthenticatedAPI` (member contact scope), on `handlers::broadcasts::{preview_input, audience_preview}` |
| Stats contact dialogs (broadcast, sequence step): tag the contacts | copied components | `bulkTagStatsContactsAction` (`rpc_procedures/contacts.rs`); `bulk_tag_stats::run` now applies the assigned-only scope |
| Sequences: `create`, `[id]` (step editor, step stats / contacts) | `routes/sequences.tsx` | `getSequence` query; `sequencesAPI.privateGetSequenceStepStatsAPI`, `privateListSequenceStepContactsAPI`; `business::sequence::{step_stats, list_step_contacts_page}` |
| Keywords and page keywords: list (has-folder), `create`, `[id]/edit` | `routes/automated-responses.tsx` | `listAutomatedResponses`, `findAutomatedResponse` (super admin, else 404) |
| Triggers: list (has-folder), `[id]/edit` | `routes/triggers.tsx` | `getTriggers`, `triggerEditPage` (super admin, else 404) |
| Email topics (has-folder) | `routes/email-topics.tsx` | existing `emailTopicsAPI.*` |
| Coupons: `(coupons)/coupons`, `topic-coupons` | `routes/coupons.tsx` | new `rpc_procedures/coupons.rs`: every `couponsAPI.*` procedure, `exportCouponsAction`, `importCouponsAction`; `business::coupon` page / count / options helpers |
| Contacts import (+ histories), scan (+ histories) | `routes/contact-imports.tsx` | new `rpc_procedures/contact_tools.rs`: `importContactsAction`, `scheduleContactScanAction`, `contactScanAPIs.getContactScanStatusAuthenticatedAPI`, `listContactScanHistory`, `listImports`, `getContactsImportTemplate`; `pages.contactScanAccess` (`spa.rs`) |
| `agents` | `routes/agents.tsx` | existing members procedures |
| `(no-sidebar)/flows/[id]/analytics`, `flows/import/histories` | `routes/flow-detail.tsx`, `routes/contact-imports.tsx` | `flowAnalyticsStats` query, `flowsAPI.privateGetFlowStatsAPI` / `privateResetFlowStatsAPI` (`rpc_procedures/flows.rs`), `listImports` (type `flow`) |
| Mini apps (`mini-apps`, `[id]/edit`, `[id]/submissions`, public `/mini-apps`) and the web shop (`shop/[workspaceId]/*`, 5) | already routed (Q.D1m, Q.D8) | rows were stale; every action they call exists (`server::mini_apps`, `api::shop_public`, `rpc_procedures/mini_apps.rs`) |

## Security

- Page guards run twice: the SPA layout (`hasWorkspacePermission`) and the
  Rust page query (the SPA has no server page): `broadcast`, `superAdmin`
  (keywords, triggers: 404), contacts access (`pages.contactsAccess`),
  unrestricted contacts scope for the scan (`pages.contactScanAccess`, 404;
  the action answers `contactScanForbidden`, the status API 403), `flows`
  for the flow analytics and flow import history (404).
- `listContactCouponsAPI`, the audience count / preview and the bulk tag use
  the member contact scope; an assigned-only member sees and tags only
  contacts with a conversation assigned to them.
- The scan and contact import actions store the signed-in user
  (`requestedByUserId`, `Import.userId`), as the original actions.
- No new outbound fetch, no new credential storage.

## Tests

`crates/api/tests/rpc_automation_pages.rs` (wiring of every page query and
procedure above, guards, validation errors) and one new case in
`crates/business/tests/contact_bulk_tag_stats.rs` (assigned-only scope).
TEST_MAP: `contact-scan-histories-page`, `contact-scan-status-api`,
`page-automated-responses-route-guards` set to `ported`.

## Left (and why)

- `flowsAPI.privateGetMessengerAdsJsonAPI`: the flow editor toolbar's Ads JSON
  dialog (ads scope, not this unit).
- Import and scan runs execute in the request (no default-queue or
  contact-scan worker yet, Q.W2); the scan run itself stays `init` until the
  contact-scan worker is ported.
- Team scope (`restrictToTeamIds`) of the bulk tag: Rust members have no team
  restriction yet (as every other contact scope here).
- The class B (jsdom) tests of the copied components stay `todo` (frontend
  tests, unchanged TypeScript).
