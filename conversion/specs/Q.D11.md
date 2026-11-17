# Q.D11 — workspace + platform domains

Unit (TRACKER Q.D11). Branch `feat/conversion-d11` (= `conversion` + P3.5f at
`4ac7ce3bb`). One pass per slice: read the TS source and frozen tests, port
the tests, implement. Test map: `specs/Q.D11-test-map.tsv`.

## Slices

| # | Slice | State |
|---|---|---|
| 1 | Workspace core, `/v1/workspace/settings`, workspace server actions, `workspacesAPI.listMy/listDeleted` | code + tests |
| 2 | Workspace members: invite / update / delete actions, member service, `workspaceMembersAPI.*` | code + tests |
| 3 | Workspace API tokens: create / delete actions | code + tests |
| 4 | Error logs: `/v1/errorLogs.list`, delete action, service | code + tests |
| 5 | Capabilities: `/v1/capabilities`, `/v1/schemas/flow-spec`, `getFlowAuthoringContext` (parity `p1-04/07-capabilities`) | code + tests |
| 6 | User quota + quota enforcement (rest) | code + tests |
| 7 | Platform credential (platform admins, branding, SMTP, email templates moved to slice 10: they are `enterprise/features`) | code + tests |
| 8 | Analytics: `/v1/analytics/*` + `analyticsRoutes.*` (parity `p1-03`) | 8a (contacts, conversations, messages, agents, bot messages) code + tests; 8b (broadcasts, sequences, flows, comment automation, magic / ref links) code + tests; 8c (MAC tracking `claimNewActiveContact`, row-level test map, Rust parity ext p1-03) code + tests |
| 9 | Workspace lifecycle (freeze + heavy-data purge) + jobs admin page (replaces Bull Board) | code + tests |
| 10 | Enterprise: license, tenant, billing, custom domain, inbox teams, help items, platform admins, platform branding / SMTP / email templates | 10a (inbox teams, help items, license) and 10b (platform admins, tenant, branding / SMTP / email templates, billing, custom domain): code + tests |
| 11 | Admin workspace | code + tests |

## 1. Slice 1 — workspace core and settings

| TypeScript | Rust |
|---|---|
| `NEXT_PUBLIC_EDITION` (`isCommunity`, …) | `chatbotx_business::keys::Edition` (passed in, read once by the caller) |
| `user-quota/service.ts` (columns), `quota-enforcement` `tryConsume` / `release` | `chatbotx_business::user_quota::{QuotaMetric, has_capacity, adjust, try_consume, release}` |
| `workspace-usage/service.ts` | `chatbotx_business::workspace_usage::{WorkspaceUsageMetric, adjust}` |
| `workspace/deletion-schedule.ts` | `workspace::deletion_schedule` |
| `workspace/service.ts`: `findById`, `update`, `updateSettings`, `setLogoIfEmpty`, `scheduleDeletion`, `cancelDeletion`, `create`, `ensureFirstForOwner`, `isActiveNow` | `chatbotx_business::workspace::*` |
| `features/workspaces/api/public.ts` (`GET` / `PATCH /v1/workspace/settings`, scope `settings`) | `handlers::workspace_settings` |
| `features/workspaces/api/private.ts` `listMyWorkspacesAPI`, `listDeletedWorkspacesAPI` | `rpc::workspace::call` |
| `features/workspaces/actions/*` (basic, advanced, smart delay, status, schedule / cancel deletion) | `POST /rpc/workspaceActions/<name>` (`rpc::actions`, `rpc::workspace::run_action`) |
| `lib/safe-action.ts` clients | `rpc::actions::Gate` |

- Server actions use the P3.5f `authActions` style: key `workspaceActions`,
  name = the TS export without `Action`, body `{ json: { workspaceId,
  ...input } }`. Order: unknown name -> 404 "Action not ported yet"; session
  (401); `mustChangePassword` (403); workspace + membership (a miss is the
  default server error, as `workspaceActionClient` throws); `workspaceActionClient`
  refuses a scheduled workspace (403 `workspaceScheduledDeletion`). Output
  `{ json: <value> }`, `undefined` -> `{ json: null, meta: [[3]] }`.
- Action errors: a `ChatbotXException` keeps code / status / message; a
  schema failure is 400 `BAD_REQUEST` "Input validation failed" with
  `data.validationErrors` in next-safe-action's formatted shape; anything
  else is 500 "Something went wrong while executing the operation.".
- `create`: community = one workspace per owner (all rows count); one
  `workspaces` quota unit of the creator; tenant from the creator; owner
  member (full permissions) + extra agent members; `WorkspaceUsage`
  team-member counter. All in one SQLite transaction (no compensation
  needed). Not ported here: MAC pre-provisioning, template staging, create
  audit (slices 6 / 9).
- Deletion log: `scheduleDeletion` writes a `pending` row for the actor (IP /
  user agent from the audit context, clipped to 64 / 1024), closes a
  previous request first, and recovers a request made before the log existed
  as `canceled`. `cancelDeletion` closes the pending row the same way. A
  started purge (`purgeStartedAt`) refuses both (409 `workspaceDeletionStarted`).

## 2. Slices 2-4 — members, API tokens, error logs

| TypeScript | Rust |
|---|---|
| `workspace-member/service.ts` (permissions, predicates, `update`, `delete`) | `chatbotx_business::workspace_member` |
| `invitation/service.ts` `create`, `buildInviteUrl` | `chatbotx_business::invitation` |
| `features/workspace-members/actions/*` | `POST /rpc/workspaceMemberActions/<name>` (`rpc::workspace_members`) |
| `workspaceMembersAPI.listWorkspaceMembersAuthenticatedAPI` | `rpc::workspace_members::call` |
| `workspace-api-token` create / delete + actions | `workspace_api_token::{create_token, delete_token}`, `workspaceActions/{create,delete}WorkspaceToken` |
| `error-log/service.ts` `listErrorLogs`, `deleteErrorLogs`, `logProviderError*` | `chatbotx_business::error_log` |
| `features/error-logs/api/public.ts` (`GET /v1/error-logs`, scope `analytics`) | `handlers::error_logs` |
| `deleteErrorLogAction` | `workspaceActions/deleteErrorLog` |

- `logProviderErrors` emits `error-log:recorded` on the error-log bus
  (`EventBuses`); `None` buses = the no-Redis fallback. The worker side
  (`write-error-log`, `purge-error-logs`) is Q.W2. The thrown value is the
  enum `Thrown` (Error / string / object / nothing).
- The error-log page loader (`app/space/[workspaceId]/error-logs/page.tsx`)
  is a web `pages.*` loader; `error_log::list` returns the rows with the
  contact and its conversation id for it.

## 3. Slice 5 — capabilities

| TypeScript | Rust |
|---|---|
| `capabilities/service.ts` `getCapabilities`, `CAPABILITIES_INCLUDES`, `DEFAULT_INCLUDES` | `chatbotx_business::capabilities` |
| `capabilities/service.ts` `getFlowAuthoringContext` | `chatbotx_business::flow::authoring_context` (now with templates and pipelines) |
| `features/capabilities/api/public.ts` (`capabilities.get`, `schemas.flowSpec`, scope `contacts`) | `handlers::capabilities` |

- Loaders read SQLite directly where no Rust list service exists (inboxes,
  templates, tags, AI agents, sequences) with the TS limits: `perPage: 200`
  through `parsePagination` = 50 rows; tags / templates = first 200.
- The policy step lists keep the `defineChannelPolicy` key order (one
  shared order; the Rust policy values are sorted maps).
- `schemas.flowSpec` serves `z::to_json_schema(flowSpecSchema)`.


## 4. Slice 6 — user quota and quota enforcement

| TypeScript | Rust |
|---|---|
| `user-quota/service.ts` `ensureBootstrapPlan`, `getForUser` (default-plan overlay), `getAccessState(FromQuota)`, `getPlanIdentity`, `listDueExpiredTrials`, `markChannelsTornDown`, white-label helpers | `chatbotx_business::user_quota::*` |
| `user-quota/service.ts` `reconcileUserSelfUsage`, `reconcileOwnerPoolUsage`, `reconcileOwnerAfterTeardown`, `raiseMacUsed`, `applyMonthlyBotMessagesReset` | `user_quota::{reconcile_user_self_usage, reconcile_owner_pool_usage, reconcile_owner_after_teardown, raise_mac_used, apply_monthly_bot_messages_reset}` |
| `quota-enforcement/service.ts` (`tryConsume`, `isAtLimit`, `incrementBy`, `releaseBy`, `getDualRemainingSlots`, `hasReachedLimit`, `hasRoomForTeamMembers`, `getUsageSummary`, `getWorkspaceUsageSummary`, `getAtLimitMap`, `createNewContactWithMac`, `createContactWithoutMac`) | `chatbotx_business::quota_enforcement` (sync, on the caller's connection) |

- Levels: an active non-root tenant pools on its owner's row (pool first);
  the reseller acting directly uses the pool only; a suspended tenant has no
  pool. A refusal names the full level and writes nothing.
- No Redis live counters, lock or admission strategy: one SQLite write
  transaction (DECISIONS). The MAC ledger claim is the caller's `create`
  closure until analytics (slice 8) ports `claimNewActiveContact`.
- The default-plan snapshot is read from the durable kv store; a sub-account
  of a reseller reads only `entitlements:default-plan:<tenant>`.

## 5. Slice 7 — platform credentials

| TypeScript | Rust |
|---|---|
| `platform-credential/service.ts` (`findDecrypted*`, `resolveForOwner`, `upsert*`, `remove*`, `resolvePublicForUser`, `findPlatformPublic`, `resolvePlatformAppAccessToken`, `findDecryptedThreadsByClientId`, `resolveWhatsappSystemUserToken`, `canUseOwnCredentials`) and `schemas.ts` | `chatbotx_business::platform_credential` (one field-spec table per type) |
| `features/platform-credentials/scope.ts` `resolveCredentialScopedUserId` | `rpc::platform_credentials::resolve_scope` |
| `features/platform-credentials/actions/*` (update / delete per type, `clearGoogleAdsDeveloperToken`, `cloneFromMessenger`) | `/rpc/platformCredentialActions/<name>` |
| `platformCredentialsAPI.getGiphyApiKeyAPI` | `/rpc/platformCredentialsAPI/getGiphyApiKeyAPI` |

- The bound `scope` argument of the TS actions is the input field `scope`
  (`user` / `platform`).
- `ApiState.encryption` holds the `ENCRYPTION_KEY` keys (`None` = the
  credential actions fail as internal errors).
- Not ported here: Stripe update / delete (P3.6-D1o), TikTok update (Q.I2),
  the WhatsApp `calls` webhook warning (Q.I1), `resolvePlatformOwnerId`
  (slice 10, needs the custom-domain host lookup).

## 6. Slice 8a — analytics reads

| TS | Rust |
| --- | --- |
| `contactStatsRepository`, `contactAnalyticsService` (reads), `macRepository.getCurrentMac`, `activeContactsRepository` | `chatbotx_business::analytics::contacts` |
| `conversationStatsRepository`, `conversationAnalyticsService` | `analytics::conversations` |
| `messageStatsRepository`, `agentResponseStatsRepository`, `agentResponseAnalyticsService` | `analytics::messages` |
| `botMessageStatsRepository`, `botMessageAnalyticsService` | `analytics::bot_messages` |
| `buildEventFilter`, `normalizeAnalyticsFilter`, `fillDaily*` / `fillMonthly*`, time range | `analytics::{Filter, TimeRange, Granularity}` |
| 24 `/v1/analytics/*` contact / conversation / message / agent / bot routes | `crates/api/src/handlers/analytics.rs` |

Notes:
- No TimescaleDB: `shouldUseCagg` is always false (the raw event tables are read) and `withCache` is not ported.
- Local days and months use the SQLite `tz_wall_ms` function; the tz name is inlined as a SQL literal (tz database names have no quotes).
- Repeated query keys (`channels=a&channels=b`) build the filter arrays from the raw query.
- Tests: `crates/business/tests/analytics_{contacts,events}.rs` (the p1-03 seeds), `crates/api/tests/analytics_{contacts,events}_public_api.rs` (shapes, 422 paths). The row-level test map of `packages/analytics/__tests__` is written with slice 8b.
- Gap: the dashboard event consumer that writes `contact_created` / conversation / message events is not in Rust, so the p1-03 test "contact analytics fed by the public API" can fail until the event writers are ported (owner: the event-bus port).

## 6b. Slice 8b — campaign, flow, link and comment automation analytics

| TS | Rust |
| --- | --- |
| `magicLinkAnalyticsService`, `refLinkAnalyticsService` (stats by day, contacts) | `analytics::links` |
| `flowAnalyticsService.getFlowStats` / `resetStatsSession`, `buildSmartDelayNodeStats` | `analytics::flows` |
| `broadcastAnalyticsService`, `sequenceAnalyticsService` (step stats) | `analytics::campaigns` |
| `commentAutomationAnalyticsService` reads, `commentAutomationMissRepository.getMissContacts`, `listCommentAutomationContacts` | `analytics::comment_automation` |
| 12 routes (`magicLink*`, `refLink*`, `flowStats`, `resetFlowStats`, `flowSmartDelayStats`, `broadcastStats`, `sequenceStepStats`, `commentAutomation*`) | `crates/api/src/handlers/analytics.rs` |

Notes:
- The comment automation range reads use `from` / `to` as given (no floor clamp). Unknown or foreign automation: `[]` or an empty page; the contacts drill-down is 404 (`findOrFail`) and its `total` is the automation's counter column.
- The counted reply channel is `public` for `threads` only (no DM), else `private`.
- Tests: `crates/business/tests/analytics_{links,flows,campaigns}.rs`, `crates/api/tests/analytics_comment_automation_public_api.rs` and `link_routes` in `analytics_events_public_api.rs` (the p1-03 seeds and 422 paths).

## 6c. Slice 8c — MAC tracking, test map, parity ext

| TS | Rust |
| --- | --- |
| `claimNewActiveContact(s)`, `ensureWorkspaceMac`, `anchoredPeriod` (`mac-period.ts`) | `analytics::mac` |
| p1-03 fixtures on SQLite | `conversion/parity/stacks/rust/ext/p1-03.ts` (`SqliteP103Fixtures`, place id 7) |

Notes:
- The claim writes `ContactActiveMonthly` / `ContactActiveHourly` with `ON CONFLICT DO NOTHING` and adds the new rows to `WorkspaceMac.macCount` in the same transaction; no Redis claim cache.
- Test map: 299 `packages/analytics/__tests__` rows. The event writers, broadcast drill-downs, email topic stats and contact-id pages are `todo` with their owner unit; Redis, cagg and mock-only rows are `n-a`.
- Tests: `crates/business/tests/analytics_mac.rs` and the `anchored_period_cases` unit test.

## 7. Slice 9 — workspace lifecycle and jobs admin page

| TS | Rust |
| --- | --- |
| `resolveWorkspaceFreezeReason`, `drainInBatches`, `freezeWorkspaceRuntime`, `purgeWorkspaceHeavyData`, `purgeLeaderboardDataForWorkspace` (rows only) | `workspace::lifecycle` |
| Bull Board (`/developer/queues`) | `crates/api/src/jobs_admin.rs` + `JobStore::{failed_jobs, retry_failed}` |

Notes:
- Each batch is one write transaction. No Postgres lock / statement timeouts and no analytics slice halving (SQLite has no statement timeout); an elapsed deadline or a full last batch is `PurgeError::Incomplete` ("Workspace purge is incomplete for <table>").
- The freeze returns the canceled dispatch `(id, bucket)` and smart-delay `(id, triggerAt)` pairs; the caller removes them from the sequence schedule and the job queue. Armed recurrences get `nextRunAt = NULL`, `pausedAt = now` without the slot ledger.
- Not ported (owners: channel / connection / integration units): `disconnectWorkspaceChannels`, `disconnectWorkspaceIntegrations`, `deactivateOwnerWorkspaces`, the leaderboard Redis drop.
- Jobs page: server-rendered HTML for a super admin (a `PlatformAdmin` row); anyone else gets 404. It shows the counts per queue and state, and the failed jobs (newest first) with their reason, with Retry (back to `waiting`, attempts kept) and Remove. A POST whose `Origin` host is not the `Host` gets 403.
- Tests: `crates/business/tests/workspace_lifecycle.rs`, `crates/api/tests/jobs_admin_page.rs`.

## 8. Slice 10a — inbox teams, help items, license

| TS | Rust |
| --- | --- |
| `inboxTeamService` (list / find / create / update / delete / addMembers / removeMembers) | `chatbotx_business::inbox_team` |
| `inboxTeamsPublicRouter` (`/v1/teams`, scope `inbox`) | `crates/api/src/handlers/inbox_teams.rs` |
| inbox team server actions + `inboxTeamsAPI.listInboxTeamsAuthenticatedAPI` | `crates/api/src/rpc/inbox_teams.rs` (key `inboxTeamActions`) |
| `tenantHelpItemService` + help item actions + `features/help-items/scope.ts` | `chatbotx_business::tenant_help_item`, `crates/api/src/rpc/help_items.rs` (key `helpItemActions`) |
| `license.service.ts` (`verifyLicenseToken`, `getLicenseStatus`, `hasEnterpriseFeatures`, `assertEnterpriseFeatures`) | `chatbotx_business::license` |

Notes:
- Inbox team mutations revoke the realtime connections of the touched members (reason `reauth`) after the commit and write the audit row. No `assertWorkspaceNotBlocked`: the workspace-token gate refuses POST / PATCH for a blocked owner.
- Help items: on a non-cloud edition, and for the `platform` scope, a super admin resolves to the root tenant (1); anyone else gets 403. On cloud the `tenant` scope needs an active tenant owned by the caller.
- License: Ed25519 JWS verify with `ring` (jose order of checks and messages). The stored `License` token wins when it has the same `licenseId` and a better state. Community is never entitled.
- No caches (inbox teams, help items, license status).
- Tests: `crates/api/tests/{inbox_teams_public_api,rpc_inbox_teams,rpc_help_items}.rs`, `crates/business/tests/license_service.rs`.

## 9. Slice 10b — platform admins, tenant, settings, billing, custom domain

| TS | Rust |
| --- | --- |
| `platformAdminService.add / remove`, `findOrProvisionRootUser`, add / remove platform admin actions | `crates/api/src/rpc/platform_admins.rs` (key `platformAdminActions`) |
| `tenantService` (find, visible channels, provision, upserts, status, suspend / reactivate / downgrade, reconcile), `customDomainService` | `chatbotx_business::tenant` |
| branding, email template and platform SMTP actions | `crates/api/src/rpc/platform_settings.rs` (key `platformSettingsActions`) |
| `reconcileTenantEntitlementAction` | same file (key `billingActions`) |
| `billingService.provisionDefaultPlan` | `chatbotx_business::billing` |
| `assertEnterpriseFeatures` of the actions | `ApiState::assert_enterprise_features` (`license_token` from `LICENSE_KEY`, `license_keys` = the built-in keys; tests set both) |

Notes:
- `superAdminActionClient` / `platformAdminActionClient` run before the input schema (403 `unauthorized`); `assertEnterpriseFeatures` runs after it (403 `enterpriseFeatureRequired`).
- The account credential mail is plain text over the auth `Mailer` (now on `ActionContext.mailer`); a failed mail creates no user.
- Not ported (owners): `previewEmailTemplate` and the SMTP `verifySmtpTransport` (mail package port); the `suspend` workspace pause and the `reactivate` connection / inbox resume (connection lifecycle units); `listPlatformAdmins` (no web procedure asks for it).
- License refresh and startup: `license::refresh_license` (vendor POST, a newer valid renewal of the same license is stored in `License`) and `license::startup_check` (the server exits 1 on a missing or invalid license for enterprise / cloud). The refresh schedule belongs to the worker cron unit.
- Tests: `crates/api/tests/{platform_admins,platform_settings}.rs`, `crates/business/tests/{tenant_service,billing_service,license_refresh}.rs`.

## 10. Slice 11 — admin workspace

| TS | Rust |
| --- | --- |
| `adminWorkspaceService.createWithMembers`, `createAdminWorkspaceAction` | `crates/api/src/rpc/admin_workspaces.rs` (key `adminWorkspaceActions`, action `createAdminWorkspace`) |
| `listAdminWorkspaces` (`workspaceSupportAccessService.listWorkspaces`) | same file, action `listAdminWorkspaces` (not a super admin: 404) |

Notes:
- Order kept: owner resolved first; existing workspace (409) before the team-member room check; members resolved after both; one create transaction with every member; audit `create` for the actor.
- The action returns `{ workspace, owner, members }`; `workspace` has the id, dates, name, owner and tenant only.
- Tests: `crates/api/tests/admin_workspaces.rs`.

## Decisions (also in DECISIONS.md)

- Server actions: key `workspaceActions` (authActions style), not the P4 web
  stub `/rpc/actions/<name>`; web switches its action client to this path.
- Quota and live counters: database columns only, inside the write
  transaction (no Redis counters, no distributed lock: SQLite has one writer).
- Support sessions are not ported (`isSupportSession` is always false).
- No workspace caches (`invalidateCacheByTags` tests are n-a).
- `updateWorkspaceAdvanced`: a Default Reply that is not a numeric id matches
  no flow (404 "Flow not found"), as the original `findFirst` does.

## State

- Done 2026-11-17. Slices 1-11: code + tests, gated on the Dell (fmt, clippy `-D warnings`, cargo test of business, api, jobs, server).
- Commits: ef702c69b (slices 1-8a), 8c80d2511 (8b), 5d6211132 (8c, 9, 10a), f4a49ed67 (10b, 11), 742e69e77 (merge origin/conversion), c934287e5 (platform credential owner), b3064d71c (purge job).
- Test map: 271 ported, 261 adapted, 146 n-a, 155 todo. Every todo row names its owner unit: event-bus port (analytics event writer), email campaigns, broadcasts, bulk tag worker, connection-state / inbox (channel teardown), worker cron (license refresh schedule; also schedules `purge_due_scheduled`).
- Parity (Rust stack, community, fresh build): p1-03 analytics + p1-04/07-capabilities 121 / 129 pass. The 8 failures are outside Q.D11: magic-link create route (5) and sequence step create route (2) answer 404 (their units), and the contacts end-to-end test waits for the event-bus analytics writer (1).
- Not ported here (owner units): `disconnectWorkspaceIntegrations` / `disconnectWorkspaceChannels` in the purge, the MJML mail templates (plain text until the mail port), the worker cron entries.
- The Dell link was down from 22:40 to 23:57 on 2026-11-16; the gates of slices 1-5 ran after it came back.
