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

### 5.2 Page loaders: `GET /rpc/pages/<name>`

For server components whose data has no oRPC procedure. Input: route params
(+ search params when noted). Output: the props the server component passed
to its client components. Implemented in `crates/api/src/spa.rs` (P4).

| Loader | Replaces | Input | Output |
|---|---|---|---|
| `pages.session` | `getCurrentUser` in every protected layout | – | `{ user: { id, name, email, image, mustChangePassword } \| null }` |
| `pages.home` | `app/page.tsx` | – | `{ user, workspaces[], ownerWorkspaceIds[], superAdminWorkspaceIds[], blocked, reason, isAtLimit, canAccessManage, isPlatformContext, isSuperAdmin, metrics, planName, planStatus, trialEndsAt }` (community: quota fields null / false) |
| `pages.workspace` | `app/space/[workspaceId]/layout.tsx` + `page.tsx` | `{ workspaceId }` | `{ allWorkspaces[], permissions, landingSegment, canAccessManage, isSuperAdmin, scheduledForDeletion, isSupportSession, consentRequired, supportAccessUntil, quota, blocked, blockReason, tokenRefreshErrors[], realtimeGates }` |
| `pages.authOptions` | `resolveAuthPageOptions` | – | `{ enabledProviders[], genericOAuth, signUpAllowed }` |
| `pages.contactsAccess` | `requireContactsAccess` | `{ workspaceId }` | `{ canViewEmailAndPhone }` |

## 6. Procedures the SPA needs that Rust does not have yet

Keep current: the SPA status per screen. "missing" = Rust answers
`404 "Operation not ported yet"` (or route 404).

| Screen | Procedure / action | Rust |
|---|---|---|
| sign-in / sign-up / session | `/api/auth/*` (better-auth) | yes (P3.5d) |
| all | `pages.session`, `pages.authOptions` | P4 (`spa.rs`) |
| workspace list | `pages.home`; action `createWorkspaceAction` | P4 / missing |
| workspace shell | `pages.workspace` | P4 (`spa.rs`) |
| contacts list | `contactsAPIs.listContactsByPOSTAuthenticatedAPI`, `contactsAPIs.countContactsAuthenticatedAPI`, `contactFilterAPI.*` (3), `tagsAPI.*` (2), `customFieldsAPI.*` (2), `pages.contactsAccess` | missing (P3.5f) |
| contact detail | `contactsAPIs.getContactAuthenticatedAPI`, `contactNotesAPI.listContactNotesAuthenticatedAPI`, `contactSequencesAPI.listContactSequencesAuthenticatedAPI`, `contactsAPIs.countContactInboxesAuthenticatedAPI`, `contactsAPIs.listContactAdReferralsAuthenticatedAPI`, `couponsAPI.listContactCouponsAPI`, `appointmentsAPI.listContactAppointmentsAPI`; contact actions (19, `features/contacts`) | missing (P3.5f) |
| flows list | `flowsAPI.privateListFlowsAPI`, `foldersAPI.*`; flow actions (10, `features/flows`), folder actions (4) | missing (P3.5f) |

## 7. Build and run

(Filled in when the Rust static serving lands.)

## 8. Decisions

See `conversion/DECISIONS.md` (2026-11-16, P4 lines).
