# Q.D11 — workspace + platform domains

Unit (TRACKER Q.D11). Branch `feat/conversion-d11` (= `conversion` + P3.5f at
`4ac7ce3bb`). One pass per slice: read the TS source and frozen tests, port
the tests, implement. Test map: `specs/Q.D11-test-map.tsv`.

## Slices

| # | Slice | State |
|---|---|---|
| 1 | Workspace core, `/v1/workspace/settings`, workspace server actions, `workspacesAPI.listMy/listDeleted` | in progress |
| 2 | Workspace members: invite / update / delete actions, member service, `workspaceMembersAPI.*` | code + tests, gate pending |
| 3 | Workspace API tokens: create / delete actions | code + tests, gate pending |
| 4 | Error logs: `/v1/errorLogs.list`, delete action, service | code + tests, gate pending |
| 5 | Capabilities: `/v1/capabilities`, `/v1/schemas/flow-spec`, `getFlowAuthoringContext` (parity `p1-04/07-capabilities`) | code + tests, gate pending |
| 6 | User quota + quota enforcement (rest) | code + tests, gate pending |
| 7 | Platform credential (platform admins, branding, SMTP, email templates moved to slice 10: they are `enterprise/features`) | code + tests, gate pending |
| 8 | Analytics: `/v1/analytics/*` + `analyticsRoutes.*` (parity `p1-03`) | 8a (contacts, conversations, messages, agents, bot messages) code + tests; 8b (broadcasts, sequences, flows, comment automation, magic / ref links) code + tests; 8c (MAC tracking `claimNewActiveContact`, row-level test map, Rust parity ext p1-03) todo |
| 9 | Workspace lifecycle (purge job) + jobs admin page (replaces Bull Board) | todo |
| 10 | Enterprise: license, tenant, billing, custom domain, inbox teams, help items, platform admins, platform branding / SMTP / email templates | todo |
| 11 | Admin workspace | todo |

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

- Slices 1-5: code + tests written; gate pending (Dell link down since 22:40).
