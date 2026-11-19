# P4 — New frontend: `apps/web` (Vite + React SPA)

Status: in progress (branch `feat/conversion-web`). Owner: P4 implementer.
Backend agents: read §5 (protocol) and §6 (missing procedures).

## 1. What it is

- `apps/web` is the builder UI (`apps/builder`) moved to a Vite + React SPA.
  Files are **copied** from `apps/builder/src` to the same path under
  `apps/web/src` and changed as little as possible. No redesign.
- Router: TanStack Router, code-based routes (`apps/web/src/routes/*.tsx`,
  tree in `apps/web/src/router.tsx`). Data: the existing TanStack Query +
  oRPC client, pointed at the Rust `/rpc`.
- The Rust binary serves `apps/web/dist` at `/` (SPA fallback to
  `index.html`). No Node / Next.js process at run time.

## 2. Inventories (generated)

`node apps/web/scripts/inventory.mjs` (repo root) rewrites three TSV files.
A row is `rust` when its name is a string in `crates/api/src` outside the
generated `rpc/table.rs` (so: a match arm / handler exists).

| File | Rows | Content |
|---|---|---|
| `P4-routes.tsv` | 263 pages | Next.js `page.tsx` → SPA path → `ported` / `todo` (`// page: app/...` marker in apps/web) |
| `P4-actions.tsv` | 606 exports in 514 `"use server"` files | feature, file, export, kind, `/rpc/actions/<name>`, rust |
| `P4-procedures.tsv` | 183 oRPC procedures called from UI code | procedure, UI uses, used in apps/web, rust |

### 2.1 Route map rule

`app/<path>/page.tsx` → SPA path: drop route groups `(x)`, `[id]` → `$id`,
`[...x]` / `[[...x]]` → `$` (splat). Layouts (`layout.tsx`, 63) become
TanStack layout routes at the same level. Pages by area:

| Area | Pages | Area | Pages |
|---|---|---|---|
| `/space/$workspaceId/settings/*` (channels, integrations, members, inbox teams, menus) | 44 | `/space/$workspaceId/whatsapps/$id/*` | 11 |
| `/admin/*` | 13 | `/manage/*` | 8 |
| `/space/$workspaceId/dashboard/*` | 7 | `/auth/*` | 6 |
| contacts (list, import, scan, histories) | 5 | flows (list, editor, analytics, import histories) | 4 |
| broadcasts | 4 | inbox | 1 |
| templates | 5 | e-commerce (products, orders, discount codes, settings, analytics, categories, vendors) | 14 |
| comment automations (fb / ig / threads / tiktok comments, ig stories) | 18 | AI (agents, files, functions, MCP servers) | 6 |
| public pages (`/booking`, `/shop`, `/l`, `/t`, `/go`, `/bs`, `/leaderboards`, `/mini-apps`, `/minigames`, `/unsubscribe`, `/extensions`, `/oauth`, `/connect`, `/invitations`, `/webchat`, `/channels/*`) | 30 | other workspace tools (polls, questionnaires, qr codes, reflinks, minigames, leaderboards, mini apps, pipelines, sequences, triggers, webhooks, ...) | ~90 |

### 2.2 Server actions → `/rpc/actions/<name>` (by feature)

606 exports: 451 workspace actions (`workspaceActionClient*`, calling / call
history clients), 87 auth actions (`authActionClient`, admin clients,
`actionClient`), 68 plain `"use server"` functions (e.g.
`lib/auth/utils.ts` `getCurrentUserAndAllLinkedWorkspaces`).

Largest groups: integration-whatsapp 28, platform-credentials 25,
pipelines 22, templates 20, contacts 19, enterprise 16, products 15,
messages 15, leaderboards 15, mini-apps 13, integration-messenger 12,
broadcasts 12, conversations 11, workspaces 10, flows 10,
facebook-marketing-messages 10, integration-google-ads 9, lib/auth 8,
questionnaires 8, media-library 8; 110 more features with 1–7 each.
Full list: `P4-actions.tsv`.

The one duplicated name, `updateHandoverResumeFlowAction`, is qualified:
`integration-whatsapp.updateHandoverResumeFlowAction`,
`integration-messenger.updateHandoverResumeFlowAction`.

### 2.3 oRPC procedures the UI calls (by router)

183 procedures: pipelinesAPI 12, adsCampaignAPI 12, couponsAPI 10,
contactsAPIs 8, integrationWhatsappAPIs 7, pollsAPI 6, flowsAPI 5,
broadcastAPIs 5, templatesAPI 4, integrationActiveCampaignAPI 4,
googleAdsAPI 4, facebookMarketingMessagesAPI 4, 59 routers with 1–3.
Full list: `P4-procedures.tsv`.

## 3. Next.js APIs and their shims

Vite `resolve.alias` + tsconfig `paths` send these imports to
`apps/web/src/shims/*` (also for `packages/ui`, which imports some of them).

| Import (uses in builder) | Shim |
|---|---|
| `next/navigation` (685): `useRouter` 354, `notFound` 241, `redirect` 61, `useSearchParams` 38, `usePathname` 38, `useParams` 20, `useSelectedLayoutSegment` 1 | `shims/next-navigation.ts` on TanStack Router. `router.refresh()` = `router.invalidate()` + invalidate all queries. `notFound()` / `redirect()` throw the TanStack errors. |
| `next/link` (154) | `shims/next-link.tsx`: `<a>` + client navigation (`href`, `replace`, `prefetch` ignored, `scroll`). |
| `next/image` (37) | `shims/next-image.tsx`: plain `<img>` (`fill` → absolute cover). |
| `next/dynamic` (6) | `shims/next-dynamic.tsx`: `React.lazy` + `Suspense`. |
| `next/script` (7) | `shims/next-script.tsx`: injects a `<script>` once. |
| `next-intl` (1366): `useTranslations`, `useLocale`, `useFormatter`, `useTimeZone`, `NextIntlClientProvider` | `shims/next-intl.ts` re-exports `use-intl` (same library under next-intl). Messages: `apps/builder/messages/<locale>.json` loaded per locale (lazy chunk). |
| `next-intl/server` (229, server pages) | not in the SPA: pages use `useTranslations`. |
| `next-safe-action/hooks` (279) | kept: it runs in the browser and calls the action function; the action is the stub from §5.1. |
| `nuqs` (15) / `nuqs/server` (123, parsers + `createSearchParamsCache`) | kept; adapter `nuqs/adapters/tanstack-router` (alias for `nuqs/adapters/next/app` in `packages/ui`). |
| `next-themes` (4) | kept (plain React). |
| `next/headers`, `next/server`, `next/cache`, `server-only`, `@/env` server part | server only; not copied. `@/env` is rewritten: client keys from `window.__ENV` (Rust injects it into `index.html`). |
| Server components (`page.tsx`, `layout.tsx`, 326 files) | rewritten as client route components: RSC data comes from an oRPC procedure (when the RSC called the same query function) or a page loader `pages.<name>` (§5.2). |

## 4. Shared packages the UI imports

Browser-safe, imported directly: `@chatbotx.io/ui` (all components),
`@chatbotx.io/utils`, `@chatbotx.io/database/partials` (zod schemas),
`@chatbotx.io/database/types` (types only), `@chatbotx.io/flow-config`,
`@chatbotx.io/sdk`, `@chatbotx.io/auth/client` (better-auth React client),
`@chatbotx.io/analytics/schemas`, `@chatbotx.io/mini-app`,
`@chatbotx.io/minigame-ui`, `@chatbotx.io/realtime-protocol`,
`@chatbotx.io/business/errors` (error classes). Type-only imports of
server packages (`@chatbotx.io/business`, `@chatbotx.io/auth/server`) are
allowed: they are erased at build time.

Not imported at run time (server only): `@chatbotx.io/business` (root),
`database` (root, client, repositories), `redis`, `encryption`,
`ai/server`, `auth/server`, `mail`, `logger`, `filesystem`, `event-bus`,
`worker-config`. `copy-from-builder.mjs` lists copied files that import one.

## 5. Protocol for the Rust server

All calls go to `/rpc/*` with the oRPC RPC encoding (`{ "json": ..., "meta": [] }`
body for POST, `?data=` for GET) and the session cookie. **No batching**:
the SPA client does not use `BatchLinkPlugin` (one request per call), so the
Rust server needs no `/rpc/__batch__`.

### 5.1 Server actions: `POST /rpc/actions/<name>`

`apps/web/scripts/copy-from-builder.mjs` turns each `"use server"` file into
a stub (`apps/web/src/lib/server-action.ts`):

- Input: `{ "args": [...all call arguments, bind args first...], "workspaceId"?: string }`.
  `workspaceId` (= `args[0]`) is set for workspace actions, so the existing
  `/rpc` workspace auth runs. `args` holds what next-safe-action got:
  `action(workspaceId, input)` → `args: [workspaceId, input]`; no-input
  actions → `args: [workspaceId]`.
- Safe actions (`*.action(...)`): success → the action's return value
  (`data`); failure → an oRPC error. The stub maps it back to the
  next-safe-action result: an error with `data.validationErrors` →
  `{ validationErrors }`, any other error → `{ serverError: message }`.
  So Rust returns the action's `data` or `{ code, status, message, data? }`
  like any procedure. `serverError` = the `ChatbotXException` message
  (`handleServerError` in `lib/safe-action.ts`), else the default message.
- Plain `"use server"` functions: the return value; an error rejects.
- Server-component queries (`features/*/queries`, not `"use server"`) that
  have no oRPC procedure run the same way under their export name
  (`findIntegrationClaude`, `listIntegrationApis`, `hasAIIntegration`, ...):
  the SPA helper `serverQuery(name, workspaceId, args)` sends
  `{ args, workspaceId }`, so the `/rpc` membership check replaces
  `assertCurrentUserCanAccessChatbot`. Handlers:
  `crates/api/src/rpc_procedures/integrations.rs`.
- Page queries: server pages whose data is several queries run as one
  `serverQuery` under a page name, `args: [workspaceId, search]`
  (`crates/api/src/rpc_procedures/products.rs`): `productsPage` →
  `{ products: { data, pageCount }, categories, importHistory, metaCatalog,
  externalStoreLinks: null }`; `productEditPage` (`{ id }`) → `{ product,
  metaCatalogId, publicUrl }`; `productCategoriesPage` → `{ data, pageCount,
  parent }`; `productVendorsPage` → `{ vendors, categories, openHours }`;
  `ecommerceShareLinks` → `{ previewUrl, buttonUrls: { shop, cart,
  categories } }`. All need the `ecommerce` permission (else 404). The
  tools lists (`listPolls`, `listQuestionnaires`, `listReflinks`,
  `listQrCodes`, `listMinigames`, `args: [search]`) are the builder query
  names (`rpc_procedures/tools.rs`).
- `redirect()` inside an action is not supported (none of the first screens
  use it); `revalidatePath` is a no-op (the SPA refetches).

### 5.2 Page loaders: `POST /rpc/pages/<name>`

For server components whose data has no oRPC procedure. POST (the RPCLink
default), body `{ json: input }`, answer `{ json, meta: [] }`; errors as the
`/rpc` errors. Input: route params (+ search params when noted). Output: the
props the server component passed to its client components. Implemented in
`crates/api/src/spa.rs`. Resources are the raw table rows as the builder
serialized them: ids as strings, dates as ISO strings, flags as booleans
(`Workspace` without `token`).

| Loader | Replaces | Input | Output |
|---|---|---|---|
| `pages.session` | `getCurrentUser` + `getTenantSettings` in the root layout | – | `{ user: { id, name, email, image, emailVerified, mustChangePassword } \| null, tenant: null }` (no auth needed; `tenant: null` = the SPA defaults) |
| `pages.authOptions` | `resolveAuthPageOptions` | – | `{ enabledProviders: [], genericOAuth: null, signUpAllowed }` |
| `pages.home` | `app/(main)/page.tsx` | – | `{ user, createdWorkspaceId, workspaces[], ownerWorkspaceIds[], superAdminWorkspaceIds[], blocked, reason, isAtLimit, canAccessManage, isPlatformContext, isSuperAdmin, metrics, planName, planStatus, trialEndsAt }`. A user with no membership gets a first workspace (`"{name}'s workspace"`, owner, full permissions) and `createdWorkspaceId`; the SPA then opens it. Community: no quota, `isAtLimit` when the user owns one workspace. |
| `pages.workspace` | `app/space/[workspaceId]/layout.tsx` | `{ workspaceId }` | `{ allWorkspaces[], permissions, isSupportSession, scheduledForDeletion, supportAccessUntil, consentRequired, canAccessManage, isSuperAdmin, blocked, blockReason, quota, tokenRefreshErrors: [], hasCallCapableChannel: false }`. Landing segment and realtime gates are computed in the SPA (pure builder code). |
| `pages.contactsAccess` | `requireContactsAccess` | `{ workspaceId }` | `{ canViewEmailAndPhone }` (`superAdmin \|\| emailAndPhone`); `404 "Page not found"` without `superAdmin`, `contacts` or `onlyAssignedContacts` |
| `pages.folder` | `getCurrentFolder` (`folderService.getWithParents`) | `{ id, workspaceId }` | `{ folder \| null, parents[] }` |
| `pages.deletedWorkspaces` | `features/workspaces/deleted-workspaces.tsx` | `{ page, perPage }` | `{ data: [] }` (deletion records not ported) |
| `pages.flow` | `app/(no-sidebar)/space/[workspaceId]/flows/[id]/page.tsx` | `{ workspaceId, id }` | the `FlowDetail` props: `{ flow (with flowVersions), flowVersion (draft), hasPublishedVersion, canRevertToPublished, isTemplateLocked, openaiCompatibleIntegrations, workspaceTargetCountry }`. Needs the `flows` permission; no flow / no draft = 404. |
| `pages.aiAgents` | the inline options of `(ai)/ai-agents/page.tsx` | `{ workspaceId }` | `AIAgentActionOptions`: `{ flows, tags, customFields (with type), admins, inboxTeams }` as `{ label, value }`. Needs `flows` (else 404). |
| `pages.settingsChannels` | `resolveChannelPolicy` | `{ workspaceId }` | `{ creatable: ["webchat", "api", "telegram"], connected: [channels with a connected inbox] }`; the SPA builds `visibleChannels` / `canCreate` from it as the builder did. Needs `superAdmin` (else 404). |
| `pages.channelsCreate` | `app/(no-sidebar)/channels/create/page.tsx` | `{ workspaceId? }` | `{ offeredChannels: ["webchat", "api"], configuredChannels: [] }`; with `workspaceId` the caller must be its super admin (else 401). |

Workspace loaders answer `401` when the caller is not a member.

### 5.3 Where the private procedures and actions live

`/rpc/actions/<name>` is part of the `/rpc` router (`crates/api/src/rpc/mod.rs`):
session check, then the workspace check when `input.workspaceId` is set,
then `crates/api/src/rpc_procedures::action`. Private procedures that are not
in the generated `PORTED` table go to `rpc_procedures::procedure`. One file
per domain: `rpc_procedures/{contacts,conversations,flows,inboxes,integrations,lists,media_library,pipelines,products,tools,workspaces}.rs`. Each
handler is thin: it parses the builder input, calls the `crates/business`
function, and returns the builder JSON. Two shared helpers:

- `Ctx::contact_scope()`: the member contact scope (`buildContactPermissionScope`:
  super admin, PII flag, assigned-only user).
- `public(ctx, route_id, ...)`: when the action or procedure has the same
  JSON as a ported public `/v1` route, it runs that route handler with a
  task-local `Member` (`handlers::dispatch_as_member`), so audit, events and
  side effects stay the same.

Model JSON: `table_row!` models have `to_json()` (ids as strings, date
columns as ISO strings, flags as booleans, JSON columns parsed); `/rpc`
answers put `Date` meta (`[1, ...path]`) on ISO date values, so `z.date()`
outputs become `Date` again. Unknown names: `404 "Operation not ported yet"`
/ `404 "Action not ported yet"`. Tests: `crates/api/tests/rpc_spa_procedures.rs`,
`crates/api/tests/spa_pages.rs`.

## 6. Screen state and missing procedures

Keep current. "works" = the screen loads and saves real data through Rust.
"missing" = Rust answers `404 "Operation not ported yet"` / `"Action not
ported yet"`; the UI shows its error or an empty option list. Full lists:
`P4-procedures.tsv`, `P4-actions.tsv` (column `rust`).

| Screen (SPA path) | State | Missing (needs a backend domain unit) |
|---|---|---|
| sign-in, sign-up, password pages (`/auth/*`) | works | OAuth providers (`pages.authOptions` lists none) |
| workspace list (`/`) | works; the first workspace is created on first load | `pages.deletedWorkspaces` is empty (deletion records); `updateWorkspaceStatusAction`, `reconcileTenantEntitlementAction` |
| new workspace (`/channels/create`) | works for webchat and API (`createWebchatAction`, `createApiAction` create the workspace). The API form keeps Create disabled while the optional Callback URL is empty: the builder does the same | the other channels: tenant channel visibility and platform credentials (`platformCredentialService`, `tenantService.resolveVisibleChannels`), Telegram / WhatsApp / Messenger / Instagram / Threads / Zalo / TikTok connect; `clearTemplateInstallIntentAction`, `rememberTemplateInstallIntentAction` |
| workspace shell (`/space/$id`) | works; realtime token 200 and the workspace socket connects when `REALTIME_BROADCAST_SECRET` is set (§7) | quota, support session, token refresh errors and call-capable channel are fixed defaults (`pages.workspace`); `refreshAllChannelTokensAction` |
| contacts list (`/space/$id/contacts`) | works: list, count, filters, tag / custom field / bot field / member / inbox / team / sequence options, bulk tag, sequence, custom field, delete, create (needs a connected inbox), export, import header peek, create custom field | filter options `emailTopicsAPI.privateListWorkspaceEmailTopicsAPI`, `workingHoursAPI.listWorkingHoursOptionsWorkspaceAuthAPI` (no Rust domain); the table avatar is not hydrated from the inbox |
| contact detail (sheet) | works: contact, tags, custom fields (edit, reset), notes (list, create, edit, delete), sequences, block / unblock, bot on / off, ad referrals | `contactsAPIs.countContactInboxesAuthenticatedAPI` (broadcast audience count: no broadcast domain), `couponsAPI.listContactCouponsAPI`, `appointmentsAPI.listContactAppointmentsAPI`, `refreshContactProfileAction` (channel profile fetch) |
| inbox (`/space/$id/inbox`) | works: conversation list (filters, search, PII rule), conversation, messages, archive / unarchive, assign, read / unread, follow, bot on / off, thread control (take / pass / release / sync, Q.D12), send (webchat: stored, `sendChannelMessage` job completed, live update over the workspace socket; API channel: needs a public callback URL, the SSRF guard blocks private hosts), edit / delete / attributes of a message | `conversationsAPI.getPostDetailsAuthenticatedAPI` (Graph API), calls (`*Voip*`, `getCall*`), saved replies (`savedRepliesAPI.*`, 3 actions), appointments, `couponsAPI.listContactCouponsAPI`, the contact panel pipelines (`pipelinesAPI.privateListContactItemsAPI`, item / comment / attachment actions), `responseSlaAPI.*`, `messagesAPI.getWhatsappFlowResponseAuthenticatedAPI`, inbox team CRUD (enterprise) |
| flows list + folders (`/space/$id/flows`) | works: list, folders (create, rename, delete, move), create, rename, duplicate, import, delete; lock badges (`templatesAPI.listLockedIdsAPI`) | – |
| tags (`/space/$id/tags`) | works: list, folders, create (name error under the field), rename, delete | – |
| custom fields + bot fields (`/space/$id/custom-fields`) | works: custom field list / update / delete, bot field list / create / update / delete / reset | – |
| sequences (`/space/$id/sequences`) | list works (needs `broadcast`) | `updateSequenceAction`, `deleteSequenceAction` and the step editor (Rust has list + create only) |
| flow editor (`/space/$id/flows/$flowId`) | works: open (`pages.flow`), save draft, publish, revert, restore, versions; option lists for tags, custom / bot fields, sequences, inboxes, pipelines (member grants), reflinks, questionnaires, place categories, sendable products, media library (lists + 8 actions) | `couponsAPI.listCouponTopicOptionsAPI`, `leaderboardsAPI.*` (no Rust domain); `messengerMessageTemplateAPIs.*`, `broadcastAPIs.*`, `decisionProfilesAPI.*`, `appointmentCalendarsAPI.*`, `spreadsheetsAPI.*`, `personasAPIs.*`, `dynamicImagesAPI.*`, `googleAdsAPI.getIntegration`, `platformCredentialsAPI.getGiphyApiKeyAPI`, the email / CRM integrations (`integration{ActiveCampaign,Drip,FacebookAds,GetResponse,Klaviyo,Mailchimp,MailerLite,Moosend,SendGrid}API.*`), `flowsAPI.privateGetFlowStatsAPI`, `flowsAPI.privateGetMessengerAdsJsonAPI`, `testExternalRequestAction`. AI option lists (agents, files, functions, MCP servers, OpenAI-compatible integrations) work |
| AI (`/space/$id/ai-agents`, `ai-files`, `ai-files/$id`, `ai-files/test-query`, `ai-functions`, `ai-mcp-servers`) | works: guard (`flows` permission, else 404), "no AI integration" page (`hasAIIntegration`), lists, create / update / delete of agents, files, functions, MCP servers; agent action options (`pages.aiAgents`) | `templatesAPI.listLockedIdsAPI` (lock badges) |
| settings shell (`/space/$id/settings/*`) | works: guard (super admin, else 404), tabs, `general` (form, save basic / advanced / smart delay, schedule / cancel deletion), `admins` (members list, invite / update / remove). The `/rpc/actions/<export>` stub of a Q.D11 action runs the D11 handler (DECISIONS 2026-11-17 Q.F2) | support sessions (a platform super admin entering a workspace with support access) are not ported: `toggleSupportAccessAction` works (super admin, not in a support session, 7-day window, logged + audited) but nothing reads `supportAccessUntil` yet; working hours, holidays and response SLA (a "not available yet" panel on `settings/inbox-teams`), email topics: no Rust domain. Works since web run 7: `settings/inbox-teams` (teams list + the 5 team actions), `settings/integrations/workspace-token` (`listWorkspaceTokens`, create / delete), `/space/$id/error-logs` (`listErrorLogs`, delete), `/space/$id/audit-logs` (`listAuditLogs`, `listAuditLogAdmins`: enterprise + super admin), `/invitations/$code` (`pages.invitation`, `acceptInvitationAction`; one tenant, no invitation cookie: "sign up to join" goes to `/auth/sign-up?callbackURL=…`), `settings/user-persistent-menus` (+ `create`, `$menuId`: `listUserPersistentMenus`, `findUserPersistentMenu`, create / update / delete) |
| settings channels (`/space/$id/settings/channels/$channel`) | works for `api` (list, rotate token, delete; `updateApiAction` is ported, but no builder screen calls it), `telegram` (list, connect for an existing workspace, disconnect), `webchat` (list, delete); channel policy from `pages.settingsChannels` | Telegram webhook registration; the other channels render an empty panel (tenant channel visibility, platform credentials) |
| platform admin console (`/admin/*`) | works for a `PlatformAdmin` user (`pages.admin`, else 404; `isSuperAdmin` of `pages.home` / `pages.workspace` shows the menu link): `workspaces` (`listAdminWorkspaces`, `createAdminWorkspace`), `admins` and `help-items` (enterprise: `listPlatformAdmins`, `listRootHelpItems` in `rpc_procedures/admin.rs`, add / remove admin, help item CRUD) | routed, "not available yet": platform credentials, deleted workspaces, platform channels, default template (Q.D4), platform audit logs, SMTP, branding, email templates (no read procedures yet); support sessions ("open" a workspace you are not a member of); `/manage/*` (cloud reseller console: one tenant here, `canAccessManage` stays false) |
| settings integrations (`/space/$id/settings/integrations/$slug`) | works for openai, claude, deepseek, gemini, muse, openrouter (connect with key check, update, disconnect) and openai-compatible (connect, update, enable, disconnect). A real key needs `ENCRYPTION_KEY` (not checked: no key in the QA) | the other integrations (email / CRM / ads / Google, Shopify, ...) render an empty panel |
| dashboard (`/space/$id/dashboard`, `contacts`, `conversations`) | works: guard (`analytics`, else 404), inbox cards, channel filter, every chart panel (`analyticsRoutes.*AnalyticsAPI` run the ported `analytics.*` routes); `dashboard/ads`, `dashboard/ads/$channel` show "not available yet" | ads analytics (Q.D7); `analyticsRoutes.getBroadcastStats*`, `getSequenceStepStats*`, `commentAutomation*`, `refLink*` / `magicLink*`, `getFlowAnalytics`, `resetFlowAnalytics` (other inputs, not on the dashboard) |
| e-commerce products (`/space/$id/products`, `products/create`, `products/$id/edit`) | works: guard (`ecommerce`, else 404), list with category sidebar, create, edit, delete, active toggle, import (start + history), meta catalog state / select / create / sync / disconnect, webshop links; the orders, discount codes, settings and analytics tabs are routed and show "not available yet" | `connectMetaCatalogAction` (OAuth redirect, platform credentials); Shopify / WooCommerce store links (`externalStoreLinks: null`); webshop public pages and tenant custom domains (links use the app URL); orders, discount codes, e-commerce settings and analytics (Q.D8) |
| product categories, vendors (`/space/$id/product-categories`, `product-vendors`) | works: list, create, update, delete, visible / active toggle, open hours create / update / delete | – |
| tools lists and detail pages (`/space/$id/polls`, `questionnaires`, `reflinks`, `qr-codes`, `minigames` and their sub-pages) | works: lists (search, sort, paging), questionnaire create / rename / duplicate / delete and the editor (`questionnaires/$id/edit`: `getQuestionnaire`, `updateQuestionnaireAction` on the public routes), poll create / edit / analytics, reflink create / update / widget / delete, QR code create / edit / delete (`findQrCode`, `create/updateQrCodeAction`), minigame create / edit / history / delete / enable (`findMinigame`, `listMinigameHistory`, `create/updateMinigameAction`, `getMinigamePlaysAction`, `findContactConversationAction`), reflink analytics (`findReflink`, `analyticsAPI.refLinkStats / refLinkContacts`), questionnaire applicants (`listQuestionnaireSubmissions`, `getQuestionnaireDashboardSummary`, detail, single and bulk delete); the bridges are in `rpc_procedures/tools.rs` | leaderboards (no Rust domain); mini apps (Q.D1m: list, editor, submissions, public runner `/mini-apps`; WhatsApp publish not ported) are in `rpc_procedures/mini_apps.rs` and `server::mini_apps` |
| pipelines (`/space/$id/pipelines`, `pipelines/$id`) | works: guard (contacts access, else 404), list (`pipelinesAPI.privateListPipelinesAPI`, member scope), create, rename / settings, duplicate, delete; the board (`canViewPipeline` via `privateGetPipelineAPI`, else 404): `privateGetBoardAPI` (`pipeline::query::list_board`), `privateListStagePageAPI`, item sheet (`privateGetItemAPI`, comments, activity), analytics sheet (`privateGetStatsAPI`), settings members (`privateListPipelineMembersAPI`), the 18 stage / field / member / item / comment / attachment actions (member scope, user actor, realtime + flow triggers) | the card contact `channel` is `null` (no last-connection lookup); `privateListContactItemsAPI` is ported, but the inbox contact panel that calls it is not copied yet |
| comment and story automations (`/space/$id/fb-comments`, `ig-comments`, `ig-stories` with folders, `create`, `$id`, `$id/analytics`; `threads-comments`, `tiktok-comments` with `create`, `$id`) | works: list (search, filters, folders), get, create (field errors under the fields), update, delete, bulk delete (FB / IG / stories), missed-comment replay start and its running state, account picker, IG media / stories and FB / Threads post pickers, analytics (reply stats, user comments, bot replies, errors, contacts, moderation logs and summary) | the template edit guard (`assertResourceEditable`) |
| guest webchat (widget, `/bs/...`, `/webchat`) | not routed. The Rust socket accepts guest tokens (`crates/rt`), but nothing issues them | `POST /api/guest/realtime-token`, `GET/POST /api/guest/messages` (webchat access token, authorized domains, guest rate limits, `handleCreateWebchatMessage`): webchat widget unit |

## 7. Build and run

One command (repo root, after `pnpm install`):

```sh
BETTER_AUTH_SECRET=<32+ chars> pnpm --filter web serve
```

It runs `vite build` (→ `apps/web/dist`) and then
`cargo run --release -p chatbotx-server` from the repo root. The server
serves the SPA from `WEB_DIR` (default `apps/web/dist`): files as they are
(`/assets/*` immutable), any other path that no API route matched →
`index.html` with `window.__ENV` (the `NEXT_PUBLIC_*` variables) in place of
`<!--chatbotx-env-->`. Unmatched `/api/`, `/rpc/`, `/files/`, `/realtime`
paths stay 404. Without `WEB_DIR/index.html` the server runs API-only.

Checked on the Dell (2026-11-16, release build, empty `DATA_DIR`): `GET /`
and `GET /auth/sign-in` → 200 `index.html` with `window.__ENV`;
`/assets/index-*.js` → 200, immutable; `GET /api/nope` → 404 (public API);
sign-up → sign-in → `pages.session` (user) → `pages.home` (first workspace
created) → `pages.workspace` → `pages.contactsAccess` all 200;
`contactsAPIs.listContactsByPOSTAuthenticatedAPI` and
`flowsAPI.privateListFlowsAPI` → 404 "Operation not ported yet" (§6). Idle
RSS of `chatbotx-server`: 24 MB.

Browser check on the Dell (2026-11-17, release build, empty `DATA_DIR`,
`BETTER_AUTH_URL=http://dell-5570:3456`): sign-up 200; sign-in 403 until
the email is verified (no mail in this setup; the QA set `emailVerified` by
hand), then 200 and `/space/<id>/dashboard` with the first workspace;
contacts list 200 ("0 contacts", filter and actions render); Create Contact
blocked: it needs an inbox and `inboxesAPI.listAllInboxesAuthenticatedAPI`
is missing; flows: Create Flow → editor opens (`pages.flow` 200, start node
rendered) → Publish 200 ("A new version has been published");
`/channels/create?channel=api` → `createApiAction` 200 → token dialog, the
new workspace is in `pages.home`. Without `REALTIME_SECRET` the realtime
token call answers 500 and the shell retries (no realtime in this check).
Main entry chunk: 595 KB (was 5.4 MB); every route is its own chunk.

Second browser check on the Dell (2026-11-17, release build at `ab7ce2209`
+ the two web fixes below, empty `DATA_DIR`). The T3 preview tab was hidden,
so there are no screenshots; the QA drove the page with JavaScript and read
the DOM text. Results: sign-up, sign-in, first workspace; `createApiAction`
with `workspaceId` adds an API inbox; `inboxesAPI.listAllInboxesAuthenticatedAPI`
200; Create Contact dialog opens with Source / Inbox and `createContactAction`
(channel `api`, User ID) 200; contact note create 200; contacts list
"1 contact"; inbox `/space/<id>/inbox` lists "Ada Lovelace — Hello from QA",
opens the conversation (message, "Assigned to me", "Bot is active", contact
panel with "Notes (1)"); `createMessageAction` 200 (stored, queued);
archive / unarchive / assign 200; tags page lists `vip`; custom fields page
shows Custom Fields and Bot Fields; sequences page renders. Two bugs found
and fixed: (1) search params with an id above 2^53 were rounded (the router
JSON-parsed them), so `/channels/create?workspaceId=<id>` answered 401 and
rendered blank; the router now keeps search values as strings; (2) the
sequences page re-created its list promise on each suspend (endless
`listSequencesWorkspaceAuthAPI` calls); the table now has its own
`<Suspense>`. Still 404 / 500 on these screens: `responseSlaAPI.*`,
`getPendingIncomingVoipCallAction`, `resolveOutboundCallModeAction`,
`templatesAPI.listLockedIdsAPI`, the realtime token (no `REALTIME_SECRET`).
Select popups do not open in a hidden tab, so the dialog's own submit was
checked through the same action call.

Third browser check on the Dell (2026-11-17, release build at `50f0097be`,
empty `DATA_DIR`). Environment:

```sh
BETTER_AUTH_SECRET=<32+ chars> BETTER_AUTH_URL=http://dell-5570:3456 \
NEXT_PUBLIC_BUILDER_URL=http://dell-5570:3456 \
REALTIME_BROADCAST_SECRET=<32+ chars> PORT=3456 DATA_DIR=<empty dir> \
./target/release/chatbotx-server
```

`REALTIME_BROADCAST_SECRET` (not `REALTIME_SECRET`) turns the realtime
gateway on; without it the server logs a warning at start and the token
call answers 500. `ENCRYPTION_KEY` is needed to store real AI provider
keys and flow variables (warning at start without it).

The T3 preview tab was hidden (no screenshots); the QA drove the page with
clicks and typing through the page's own handlers and read the DOM text,
the network entries and the QA database. Results: webchat created for the
existing workspace (`/channels/create?channel=webchat&workspaceId=`) and
listed in settings; webchat contact; inbox send → `createMessageAction`
200, the `sendChannelMessage` job completed, `ContactInbox.lastOutboundMessageAt`
set, realtime token 200 and one workspace socket (gateway metrics:
upgrade 1, events sent); API channel with a private callback URL → the
SSRF guard rejects it (the builder shows the same generic message);
Claude connect with a fake key → "Invalid API key" under the field;
connected state (row added by hand) → Auto Reply switch saves
(`updateIntegrationClaudeAction`), Disconnect → `disconnectClaudeAction`
200 and the page shows Connect again; AI agents: no-integration page, then
list, Create AI Agent dialog → `createAIAgentAction` 200 and the new row;
Knowledge, test query, AI Functions, MCP Servers, settings general (form
with the workspace data), admins (owner listed), channels telegram / api /
webchat, integrations openai / openai-compatible / gemini: all `/rpc`
calls 200 (only `getPendingIncomingVoipCallAction` 404). Not checked:
API callback delivery (needs a public URL), the guest webchat socket
(no guest token without the webchat widget unit), Telegram connect (needs
a real bot token), real AI keys, webchat delete.

Fourth browser check on the Dell (2026-11-17, release build at `e70fb5f11`,
empty `DATA_DIR`, the environment above plus `ENCRYPTION_KEY`; hidden
preview tab: DOM clicks and typing, DOM text and the network entries).
Results: sign-up, sign-in (email verified by hand), first workspace opens
`/dashboard` → redirect to `dashboard/contacts`; dashboard layout, nav and
"Create Inbox" card render, `inboxesAPI.listAllInboxesAuthenticatedAPI`
200, every chart panel "Error" (all `analyticsRoutes.*` 404, §6); products
list (`productsPage` 200, share footer `ecommerceShareLinks` 200);
categories: Add dialog → `createProductCategoryAction` 200, row "Drinks";
vendors list 200; products/create (`productsAPI.listProductFormOptionsAPI`
200) → Save → `createProductAction` 200, "Products created successfully",
row "Lemonade $4.50"; products/$id/edit loads the form (`productEditPage`
200) → Save → `updateProductAction` 200; polls, questionnaires, reflinks,
QR codes, minigames lists 200; questionnaire Add → `createQuestionnaireAction`
200 (then the editor 404, §6); pipelines list 200, Add pipeline →
`createPipelineAction` 200 (then the board 404, §6); settings channels API:
Rotate token → `rotateApiTokenAction` 200 and the token dialog, Delete →
`deleteApiAction` 200 and "No data". Other 404s on these screens:
`templatesAPI.listLockedIdsAPI`, `getPendingIncomingVoipCallAction`. Not
checked: product import, Meta catalog, vendor / open hours writes, the
other tool writes (reflink, QR code, minigame), the guest webchat (§6).

Development: `CHATBOTX_SERVER_URL=http://localhost:3000 pnpm --filter web dev`
(Vite on :3123, proxies `/rpc`, `/api`, `/files`, `/realtime` to the Rust
server).

## 8. Decisions

See `conversion/DECISIONS.md` (2026-11-16, P4 lines).
