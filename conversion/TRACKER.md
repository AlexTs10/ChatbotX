# Tracker

Status values: `todo`, `in_progress`, `blocked` (reason required), `done`, `dropped`.
Only one unit can be `in_progress` at a time — exception (user, 2026-11-11): units whose files do not overlap can run in parallel (for example a Phase 1 harness unit and a Phase 2 Rust unit). Units for Phase 1 and later are added when Phase 0 is done.

| ID | Phase | Unit | Status | Depends on | Tests (A/B/C) | Proof | Commit |
|----|-------|------|--------|------------|---------------|-------|--------|
| P0.1 | 0 | Fork, clone, `conversion` branch, empty tracker files | done | - | - | See "Proof P0.1" | commit `conversion(P0.1)` in git log |
| P0.2 | 0 | Copy all tests to `frozen-tests/` + `MANIFEST.sha256` | done | P0.1 | - | See "Proof P0.2" | commit `conversion(P0.2)` |
| P0.3 | 0 | Run original suite (`pnpm install && pnpm test`), record baseline, mark `baseline-fail` | done | P0.2 | - | See "Proof P0.3" | commit `conversion(P0.3)` |
| P0.7 | 0 | Run the 96 `baseline-skip` tests (need Postgres/Redis) + recheck the 1 `baseline-fail` against the P1.1 stack services on the Dell | todo | P0.3, P1.1 | - | | |
| P0.4 | 0 | Feature inventory in `FEATURES.md` (LOC + test count); keep/drop already decided (see DECISIONS 2026-11-11: drop Execute JavaScript only) | done | P0.1 | - | See "Proof P0.4" | commit `conversion(P0.4+P0.6)` |
| P0.5 | 0 | Classify every test file A/B/C in `TEST_MAP.md`; mark dropped-feature tests `dropped` | done | P0.2, P0.4 | B:665 C:2426 dropped:13 | See "Proof P0.5" | commit `conversion(P0.5)` |
| P0.6 | 0 | RSS baseline of original stack into `MEMORY.md` | done | P0.1 | - | User-set baseline 2 GB (no measurement, see DECISIONS) | commit `conversion(P0.4+P0.6)` |
| P1.1 | 1 | Harness scaffold: `conversion/parity/` package, Dell stack runner for the original stack, fake upstream server (DNS + test CA), bootstrap helpers (sign-up, workspace, API token), smoke tests | in_progress | P0.5 | A: smoke | | |
| P1.2 | 1 | Public API parity: `ads`, `aiAgents`, `aiFiles`, `aiFunctions` | todo | P1.1 | A: 52 ops | | |
| P1.3 | 1 | Public API parity: `aiHandover`, `aiMcpServers`, `analytics` | todo | P1.1 | A: 49 ops | | |
| P1.4 | 1 | Public API parity: `appointmentCalendars`, `appointmentExternalCalendars`, `appointmentReminders`, `appointments`, `botFields`, `botSimulator`, `broadcasts`, `capabilities`, `carts`, `channelIntegrations`, `channelPosts` | todo | P1.1 | A: 55 ops | | |
| P1.5 | 1 | Public API parity: `channels`, `commentAutomations`, `connectSessions`, `connectionProviders`, `connections`, `contactScans` | todo | P1.1 | A: 25 ops | | |
| P1.6 | 1 | Public API parity: `contacts` | todo | P1.1 | A: 58 ops | | |
| P1.7 | 1 | Public API parity: `conversations`, `coupons`, `customFields`, `decisionConnections`, `decisionProfiles`, `discountCodes` | todo | P1.1 | A: 53 ops | | |
| P1.8 | 1 | Public API parity: `dynamicImages`, `ecommerceAnalytics`, `ecommerceSettings`, `emailTopics`, `errorLogs`, `externalWebhooks`, `facebookLeadAds`, `facebookMarketingMessages`, `fbComments` | todo | P1.1 | A: 48 ops | | |
| P1.9 | 1 | Public API parity: `flows`, `folders`, `googleAds`, `igComments`, `igStories`, `inboundWebhookChannels`, `inboundWebhooks`, `inboxTeams` | todo | P1.1 | A: 53 ops | | |
| P1.10 | 1 | Public API parity: `inboxes`, `instagramChannels`, `integrations`, `keywords`, `leaderboards` | todo | P1.1 | A: 52 ops | | |
| P1.11 | 1 | Public API parity: `magicLinks`, `me`, `mediaLibrary`, `messages`, `messengerChannels`, `messengerPersonas`, `messengerTemplates` | todo | P1.1 | A: 52 ops | | |
| P1.12 | 1 | Public API parity: `miniApps`, `minigames`, `openHours`, `orders`, `paymentProviders` | todo | P1.1 | A: 43 ops | | |
| P1.13 | 1 | Public API parity: `pipelines`, `placeCategories`, `placeLocations`, `polls` | todo | P1.1 | A: 51 ops | | |
| P1.14 | 1 | Public API parity: `productCategories`, `productVendors`, `products`, `qrCodes`, `questionnaires`, `realtime`, `reflinks` | todo | P1.1 | A: 54 ops | | |
| P1.15 | 1 | Public API parity: `responseSla`, `savedReplies`, `schemas`, `sequences`, `shopify`, `smtpIntegrations`, `spreadsheets`, `tags`, `templateInstallations` | todo | P1.1 | A: 55 ops | | |
| P1.16 | 1 | Public API parity: `templateMessages`, `templatePreviews`, `templates`, `threadsComments`, `tiktokChannels`, `tiktokComments`, `token`, `triggers`, `userPersistentMenus`, `webchats` | todo | P1.1 | A: 53 ops | | |
| P1.17 | 1 | Public API parity: `webhooks`, `webshop`, `whatsappCalls`, `whatsappChannels`, `whatsappFlows`, `whatsappTemplates`, `woocommerce`, `workingHours`, `workspaceMembers`, `workspaceSettings` | todo | P1.1 | A: 55 ops | | |
| P1.18 | 1 | Public API parity: `zaloChannels` | todo | P1.1 | A: 3 ops | | |
| P1.19 | 1 | Internal `/rpc` procedures not in the public API (frontend calls) | todo | P1.1 | A | | |
| P1.20 | 1 | Webhooks: WhatsApp (`/integrations/whatsapp/webhook/*`, callback, VoIP hangup, call recording) | todo | P1.1 | A | | |
| P1.21 | 1 | Webhooks: Meta (Messenger, Instagram, Instagram-Facebook, Threads) via `/integrations/[...integration]` | todo | P1.1 | A | | |
| P1.22 | 1 | Webhooks: Telegram, TikTok, Zalo (+ zalo-verifier), Webchat, API channel | todo | P1.1 | A | | |
| P1.23 | 1 | Webhooks: payments (Stripe, PayPal), Shopify, WooCommerce, inbound hooks `/api/hooks/[token]` | todo | P1.1 | A | | |
| P1.24 | 1 | Auth + OAuth routes (`/api/auth/*`, `/oauth/*`, `/.well-known/*`), invitations, magic links | todo | P1.1 | A | | |
| P1.25 | 1 | Files: presigned upload, media attachment/avatar, mini-app uploads, dynamic images, exports and import templates | todo | P1.1 | A | | |
| P1.26 | 1 | Public pages API: guest messages + realtime token, leaderboards, reflinks, email-topic open/click, `/r/*` | todo | P1.1 | A | | |
| P1.27 | 1 | Realtime WebSocket events (`apps/realtime`) | todo | P1.1 | A | | |
| P1.28 | 1 | Full Class A run against the original stack: 100% pass, then freeze `parity/` into `MANIFEST.sha256` | todo | P1.2–P1.27 | A | | |
| P2.1 | 2 | Cargo workspace + one binary skeleton: crates `api` (axum), `core`, `db`, `jobs`, `rt`, `files`, bin `chatbotx-server`; config from env, tracing, `/api/health`, graceful shutdown; release profile for size/RSS | done | P0.5 | - | See "Proof P2.1" | commit `conversion(P2.1)` |
| P2.2 | 2 | SQLite schema analysis: map all 224 Drizzle schema files (Postgres types, enums, jsonb, hypertables, pgvector, partial indexes, triggers) to libSQL; spec only | done | P2.1 | - | See "Proof P2.2" | commit `conversion(P2.2)` |
| P2.3 | 2 | `db` crate: embedded SQLite file (crate choice: see DECISIONS P2.2), migration runner, schema part 1 (auth, users, workspaces, members, tokens, channels, contacts, custom fields, tags) | blocked (user decision: rusqlite vs libsql) | P2.2 | C | | |
| P2.4 | 2 | `db` crate: schema part 2 (flows, steps, triggers, conversations, messages, inbox, broadcasts, sequences, templates) | todo | P2.3 | C | | |
| P2.5 | 2 | `db` crate: schema part 3 (ecommerce, ads, analytics time tables, AI/RAG vectors, minigames, leaderboards, payments, remaining tables) | todo | P2.4 | C | | |
| P2.6 | 2 | `jobs` crate: `jobs` table, enqueue/claim/retry/backoff/cron/delayed, tokio workers; map each BullMQ queue in `packages/worker-config` | todo | P2.3 | C | | |
| P2.7 | 2 | Redis replacement: locks (`tokio::sync::Mutex` map), pub/sub (`broadcast`), cache/presence/rate limits (in-memory), bloom filter (unique-index table); map each `packages/redis` use | todo | P2.6 | C | | |
| P2.8 | 2 | `rt` crate: axum WebSocket endpoint + `broadcast` fan-out, protocol from `packages/realtime-protocol` | todo | P2.7 | C | | |
| P2.9 | 2 | `files` crate: `object_store` LocalFileSystem (S3 by config), presigned-style upload/download tokens | in_progress | P2.1 | C | | |

## Proof

### Proof P0.1
Verifier subagent (independent, read-only): overall PASS.

```
$ git remote -v
origin    https://github.com/AlexTs10/ChatbotX.git (fetch/push)
upstream  https://github.com/ChatbotXIO/ChatbotX.git (fetch/push)
$ gh repo view AlexTs10/ChatbotX --json isFork,parent
{"isFork":true,"parent":{"name":"ChatbotX","owner":{"login":"ChatbotXIO"}}}
$ git branch --show-current
conversion
$ git rev-parse HEAD
40e733cee28157db0372bd4bd6e801da3199aa83
$ git ls-remote upstream refs/heads/main     # upstream moved 1 commit after the clone; HEAD is its ancestor
f371c5c5a2480897d2d57edd3a58304427fd25f5
$ git status --porcelain -uall               # only new files under conversion/
$ git ls-files | grep -E '\.(test|spec)\.(ts|tsx|js|jsx|mts|cts)$' | wc -l
3100
```

Manifest check: not applicable (frozen-tests/ is empty until P0.2).

### Proof P0.2

Frozen set = 3,104 test files (`*.test.*`, `*.spec.*`) + 115 support files: files in `__tests__/`, `__mocks__/`, `fixtures/`, `test-utils/`, `testing/`, all `vitest*.config.*` files, and `packages/vitest-config/`. Total 3,219 files, 30 MB. Paths are the same as in the repo.
`conversion/` is outside the pnpm workspace, the root Vitest `projects` globs, and the Biome `includes`, so the copies do not run twice and are not linted.

```
$ conversion/check-manifest.sh
manifest OK: 3219 files
$ # byte compare of each original with its copy
byte diffs: 0
$ shasum -a 256 conversion/frozen-tests/MANIFEST.sha256
09e8faff6524ab94ca17b9ce8dae03419229df71afac0cb5277f9858a6c850f8  conversion/frozen-tests/MANIFEST.sha256
```

Rule from now: run `conversion/check-manifest.sh` before every commit. If it fails, stop and revert.

### Proof P0.4

```
features: 114 rows + 14 shared-infrastructure rows
non-test source LOC: ~830,400 (features) + ~61,800 (infra) = ~892,200; SQL migrations: 308 files, ~10,776 lines (separate)
test files assigned: 3104 / 3104 (2905 feature rows, 199 infra rows), unassigned: 0
rows with decision drop: 1 (Execute JavaScript flow step, 12 test files)
per-test feature: conversion/feature-of-test.tsv (3104 lines)
```

### Proof P0.6

No measurement. The user set the baseline on 2026-11-11: original stack ≈ 2 GB RSS. Target stays ≤ 200 MB.

### Proof P0.5

Classifier: `conversion/tools/classify_tests.py` (rules in its docstring). Output: `conversion/TEST_MAP.md`.
`baseline-fail` status is added by P0.3 (the baseline run on the Dell) over these rows.

```
$ python3 conversion/tools/classify_tests.py
('-', 'dropped') 13
('B', 'todo') 665
('C', 'todo') 2426
total 3104
```

Independent verifier (60-row sample, seed 7), first pass: builder-B 4/25 wrong (server tests without `@vitest-environment node`), builder-C 0/15, other-C 0/10, cli/mcp/ui-B 0/5, dropped 12/12 correct; 1 dropped-feature test not marked (`packages/variables/__tests__/interpolate-into-javascript.test.ts`).
Fixes applied (DOM check + server-module rule, extra dropped path). Re-check of the 5 wrong rows: all now C; DOM test `use-mark-conversation-read.test.tsx` stays B; `interpolate-into-javascript.test.ts` now dropped.

### Proof P0.3

Run on the Dell (Node 24.22.1, pnpm 10.34.5, `turbo run test --concurrency=4 --continue`, JSON reporter per package), 25 min. No Postgres/Redis running.
Per-package lines: `conversion/baseline/p0.3-per-package.txt`; per-file status: `conversion/baseline/p0.3-summary.json`.

```
 Tasks:    66 successful, 67 total
Failed:    builder#test
builder:test:  Test Files  1 failed | 1156 passed (1157)
builder:test:       Tests  10474 passed | 41 skipped (10515)
FAIL __tests__/public-spec-operations.test.ts  Error: Hook timed out in 120000ms.  (public-spec-operations.test.ts:153 beforeAll)
all packages: test files 3093, tests 36633, passed 35629, failed 0, skipped 1004
TEST_MAP: baseline-fail 1, baseline-skip 96 (88 integration files that skip without DATABASE_URL/REDIS_URL + 8 worker integration files outside the default config)
```

### Proof P2.1

Implementer + independent verifier (overall PASS, 7/7). Run on the Dell.

```
FMT_OK
CLIPPY_OK   (cargo clippy --workspace --all-targets -- -D warnings)
test result: ok. 5 passed; 0 failed   (chatbotx-api iso8601)
test result: ok. 2 passed; 0 failed   (chatbotx-api tests/health.rs)
test result: ok. 5 passed; 0 failed   (chatbotx-server config)
TEST_OK
release binary: 1442632 bytes (target/release/chatbotx-server), 63 crates
idle RSS: 3832 KiB, 3 threads (main + 2 workers)
GET /api/health -> 200 application/json {"status":"healthy","timestamp":"2026-11-11T09:13:54.885Z","service":"builder"}
SIGTERM -> "shutdown complete", exit 0
verifier: iso8601 vs python datetime on 200,000 random instants 1970-9999: 0 mismatches
```

### Proof P2.2

Spec: `conversion/specs/P2.2.md` (872 lines), from the newest Drizzle snapshot + all 308 SQL migrations.

```
tables 258 | columns 3273 | indexes 745 (116 partial) | foreign keys 556 | enums 154 | CHECK 50
no sequences, no RLS, no plain views, no LISTEN/NOTIFY
vector columns: 2 (1536-d, cosine); HNSW index declared but never created by a migration
partitioned tables 11 -> one table each; message sharding tables not created
trigger functions 4 (3 -> SQLite triggers, updatedAt -> Rust); trigram indexes 5 -> FTS5 trigram
locks: FOR UPDATE 87, SKIP LOCKED 30, advisory 22 -> BEGIN IMMEDIATE + lease claims
analytics: time_bucket 42, AT TIME ZONE 79, continuous aggregates 4 -> rollup tables written in the same tx
part sizes: P2.3 65 tables, P2.4 65 (+2 not created), P2.5 126
```
