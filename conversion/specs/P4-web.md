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
| `pages.flow` | `app/(no-sidebar)/space/[workspaceId]/flows/[id]/page.tsx` | `{ workspaceId, id }` | the `FlowDetail` props: `{ flow (with flowVersions), flowVersion (draft), hasPublishedVersion, canRevertToPublished, isTemplateLocked, openaiCompatibleIntegrations: [], workspaceTargetCountry }`. Needs the `flows` permission; no flow / no draft = 404. |
| `pages.channelsCreate` | `app/(no-sidebar)/channels/create/page.tsx` | `{ workspaceId? }` | `{ offeredChannels: ["webchat", "api"], configuredChannels: [] }`; with `workspaceId` the caller must be its super admin (else 401). |

Workspace loaders answer `401` when the caller is not a member.

### 5.3 Where the private procedures and actions live

`/rpc/actions/<name>` is part of the `/rpc` router (`crates/api/src/rpc/mod.rs`):
session check, then the workspace check when `input.workspaceId` is set,
then `crates/api/src/rpc_procedures::action`. Private procedures that are not
in the generated `PORTED` table go to `rpc_procedures::procedure`. One file
per domain: `rpc_procedures/{contacts,conversations,flows,inboxes,lists,media_library,pipelines,workspaces}.rs`. Each
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
| workspace shell (`/space/$id`) | works | quota, support session, token refresh errors and call-capable channel are fixed defaults (`pages.workspace`); `refreshAllChannelTokensAction`; `realtimeAPI.mintWorkspaceConnectTokenAuthenticatedAPI` |
| contacts list (`/space/$id/contacts`) | works: list, count, filters, tag / custom field / bot field / member / inbox / team / sequence options, bulk tag, sequence, custom field, delete, create (needs a connected inbox), export, import header peek, create custom field | filter options `emailTopicsAPI.privateListWorkspaceEmailTopicsAPI`, `workingHoursAPI.listWorkingHoursOptionsWorkspaceAuthAPI` (no Rust domain); the table avatar is not hydrated from the inbox |
| contact detail (sheet) | works: contact, tags, custom fields (edit, reset), notes (list, create, edit, delete), sequences, block / unblock, bot on / off, ad referrals | `contactsAPIs.countContactInboxesAuthenticatedAPI` (broadcast audience count: no broadcast domain), `couponsAPI.listContactCouponsAPI`, `appointmentsAPI.listContactAppointmentsAPI`, `refreshContactProfileAction` (channel profile fetch) |
| inbox (`/space/$id/inbox`) | works: conversation list (filters, search, PII rule), conversation, messages, archive / unarchive, assign, read / unread, follow, bot on / off, send (stored and queued; delivery needs the worker core), edit / delete / attributes of a message | `threadControlAction`, `syncThreadOwnerAction` (501 in Rust), `conversationsAPI.getPostDetailsAuthenticatedAPI` (Graph API), calls (`*Voip*`, `getCall*`), saved replies (`savedRepliesAPI.*`, 3 actions), appointments, `couponsAPI.listContactCouponsAPI`, the contact panel pipelines (`pipelinesAPI.privateListContactItemsAPI`, item / comment / attachment actions), `responseSlaAPI.*`, `whatsappMessageTemplateAPIs.*`, `messagesAPI.getWhatsappFlowResponseAuthenticatedAPI`, inbox team CRUD (enterprise) |
| flows list + folders (`/space/$id/flows`) | works: list, folders (create, rename, delete, move), create, rename, duplicate, import, delete | `templatesAPI.listLockedIdsAPI` (lock badges) |
| tags (`/space/$id/tags`) | works: list, folders, create (name error under the field), rename, delete | – |
| custom fields + bot fields (`/space/$id/custom-fields`) | works: custom field list / update / delete, bot field list / create / update / delete / reset | – |
| sequences (`/space/$id/sequences`) | list works (needs `broadcast`) | `updateSequenceAction`, `deleteSequenceAction` and the step editor (Rust has list + create only) |
| flow editor (`/space/$id/flows/$flowId`) | works: open (`pages.flow`), save draft, publish, revert, restore, versions; option lists for tags, custom / bot fields, sequences, inboxes, pipelines (member grants), reflinks, questionnaires, place categories, sendable products, media library (lists + 8 actions) | `productsAPI.listProductFormOptionsAPI`, `couponsAPI.listCouponTopicOptionsAPI`, `leaderboardsAPI.*`, `miniAppsAPI.*` (no Rust domain); `whatsappMessageTemplateAPIs.*`, `whatsappFlowAPIs.*`, `messengerMessageTemplateAPIs.*`, `aiAgentsAPI.*`, `aiFilesAPI.*`, `aiFunctionsAPI.*`, `aiMcpServerAPIs.*`, `broadcastAPIs.*`, `decisionProfilesAPI.*`, `appointmentCalendarsAPI.*`, `spreadsheetsAPI.*`, `personasAPIs.*`, `userPersistentMenusAPI.*`, `dynamicImagesAPI.*`, `googleAdsAPI.getIntegration`, `platformCredentialsAPI.getGiphyApiKeyAPI`, the email / CRM integrations (`integration{ActiveCampaign,Drip,FacebookAds,GetResponse,Klaviyo,Mailchimp,MailerLite,Moosend,SendGrid}API.*`), `flowsAPI.privateGetFlowStatsAPI`, `flowsAPI.privateGetMessengerAdsJsonAPI`, `testExternalRequestAction`; OpenAI-compatible integrations (`pages.flow` sends `[]`) |
| settings pages | not routed | members list exists (`workspaceMembersAPI.listWorkspaceMembersAuthenticatedAPI`), but invite / update / remove have no Rust business; API tokens (no management domain), email topics, working hours, workspace general, channels: no Rust domain |

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

Development: `CHATBOTX_SERVER_URL=http://localhost:3000 pnpm --filter web dev`
(Vite on :3123, proxies `/rpc`, `/api`, `/files`, `/realtime` to the Rust
server).

## 8. Decisions

See `conversion/DECISIONS.md` (2026-11-16, P4 lines).
