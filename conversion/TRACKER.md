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
| P1.1 | 1 | Harness scaffold: `conversion/parity/` package, Dell stack runner for the original stack, fake upstream server (DNS + test CA), bootstrap helpers (sign-up, workspace, API token), smoke tests | done | P0.5 | A: smoke | See "Proof P1.1" | commit `conversion(P1.1)` |
| P1.2 | 1 | Public API parity: `ads`, `aiAgents`, `aiFiles`, `aiFunctions` | done | P1.1 | A: 52 ops, 260 tests | See "Proof P1.2" | commit `conversion(P1.2)` |
| P1.3 | 1 | Public API parity: `aiHandover`, `aiMcpServers`, `analytics` | done | P1.1 | A: 49 ops, 389 tests | See "Proof P1.3" | commit `conversion(P1.3)` |
| P1.4 | 1 | Public API parity: `appointmentCalendars`, `appointmentExternalCalendars`, `appointmentReminders`, `appointments`, `botFields`, `botSimulator`, `broadcasts`, `capabilities`, `carts`, `channelIntegrations`, `channelPosts` | done | P1.1 | A: 55 ops, 355 tests | See "Proof P1.4" | commit `conversion(P1.4)` |
| P1.5 | 1 | Public API parity: `channels`, `commentAutomations`, `connectSessions`, `connectionProviders`, `connections`, `contactScans` | done | P1.1 | A: 25 ops, 210 tests | See "Proof P1.5" | commit `conversion(P1.5+P1.6)` |
| P1.6 | 1 | Public API parity: `contacts` | done | P1.1 | A: 58 ops, 539 tests | See "Proof P1.6" | commit `conversion(P1.5+P1.6)` |
| P1.7 | 1 | Public API parity: `conversations`, `coupons`, `customFields`, `decisionConnections`, `decisionProfiles`, `discountCodes` | done | P1.1 | A: 53 ops, 437 tests | See "Proof P1.7" | commit `868c4acee` |
| P1.8 | 1 | Public API parity: `dynamicImages`, `ecommerceAnalytics`, `ecommerceSettings`, `emailTopics`, `errorLogs`, `externalWebhooks`, `facebookLeadAds`, `facebookMarketingMessages`, `fbComments` | done | P1.1 | A: 48 ops, 335 tests | See "Proof P1.8" | commit `868c4acee` |
| P1.9 | 1 | Public API parity: `flows`, `folders`, `googleAds`, `igComments`, `igStories`, `inboundWebhookChannels`, `inboundWebhooks`, `inboxTeams` | done | P1.1 | A: 53 ops + hook route, 457 tests | See "Proof P1.9" | commit `868c4acee` |
| P1.10 | 1 | Public API parity: `inboxes`, `instagramChannels`, `integrations`, `keywords`, `leaderboards` | done | P1.1 | A: 52 ops, 398 tests | See "Proof P1.10" | commit `c01a4eea9` |
| P1.11 | 1 | Public API parity: `magicLinks`, `me`, `mediaLibrary`, `messages`, `messengerChannels`, `messengerPersonas`, `messengerTemplates` | done | P1.1 | A: 52 ops, 344 tests | See "Proof P1.11" | commit `1da6b576d` |
| P1.12 | 1 | Public API parity: `miniApps`, `minigames`, `openHours`, `orders`, `paymentProviders` | in_progress | P1.1 | A: 43 ops, 297 tests (verification pending) | | |
| P1.13 | 1 | Public API parity: `pipelines`, `placeCategories`, `placeLocations`, `polls` | in_progress | P1.1 | A: 51 ops, 314 tests (verification pending) | | |
| P1.14 | 1 | Public API parity: `productCategories`, `productVendors`, `products`, `qrCodes`, `questionnaires`, `realtime`, `reflinks` | in_progress | P1.1 | A: 54 ops, 330 tests (verification pending) | | |
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
| P2.3 | 2 | `db` crate: embedded SQLite file via `rusqlite` (bundled), migration runner, schema part 1 (auth, users, workspaces, members, tokens, channels, contacts, custom fields, tags) | done | P2.2 | C:48 (+6 n/a) | See "Proof P2.3" | commit `conversion(P2.3)` |
| P2.4 | 2 | `db` crate: schema part 2 (flows, steps, triggers, conversations, messages, inbox, broadcasts, sequences, templates) | done | P2.3 | C:36 (+10 n/a) | See "Proof P2.4" | commit `59ae66f4f` |
| P2.5 | 2 | `db` crate: schema part 3 (ecommerce, ads, analytics time tables, AI/RAG vectors, minigames, leaderboards, payments, remaining tables) | done | P2.4 | C:37 (+6 n/a) | See "Proof P2.5" | commit `6839f5b73` |
| P2.6 | 2 | `jobs` crate: `jobs` table, enqueue/claim/retry/backoff/cron/delayed, tokio workers; map each BullMQ queue in `packages/worker-config` | done | P2.3 | C:455 (+1 n/a) | See "Proof P2.6" | commit `c05a22f96` |
| P2.7 | 2 | Redis replacement: locks (`tokio::sync::Mutex` map), pub/sub (`broadcast`), cache/presence/rate limits (in-memory), bloom filter (unique-index table); map each `packages/redis` use | in_progress | P2.6 | C | | |
| P2.8 | 2 | `rt` crate: axum WebSocket endpoint + `broadcast` fan-out, protocol from `packages/realtime-protocol` | todo | P2.7 | C | | |
| P2.9 | 2 | `files` crate: `object_store` LocalFileSystem (S3 by config), presigned-style upload/download tokens | done | P2.1 | C:40 | See "Proof P2.9" | commit `conversion(P2.9)` |
| P3.1 | 3 | Backend port, shared code → `crates/core`: `packages/utils`, `packages/encryption`, `packages/variables` (no JS interpolation), `packages/logger`, `packages/decision-config` (~13k lines, 80 C test files) | in_progress | P2.1 | C | | |
| P3.2 | 3 | Backend port: `packages/flow-config` (~21k lines, 74 C test files) → `crates/core` | todo | P3.1 | C | | |
| P3.3 | 3 | Backend port: auth (`packages/auth`, builder auth routes/middleware, workspace API tokens, scopes) + axum API skeleton with oRPC-compatible errors → `crates/api` | todo | P3.1, P2.5 | C + A (P1 auth matrices) | | |
| P3.4+ | 3 | Business domains (`packages/business` 161k, `apps/builder` server 159k, `packages/database` repositories 85k), worker handlers (`apps/worker` 82k), integrations (~70k). Units are cut per domain at the start of each batch; each batch must turn the matching P1 parity folders green against the Rust stack | todo | P3.3 | C + A | | |

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

### Proof P2.9

Analyst spec `conversion/specs/P2.9.md` → porter (40 tests, failing, commit `e116efd04`) → implementer → independent verifier (PASS, 5/5; assertion parity checked case by case against the 6 frozen files).

```
$ git diff --stat e116efd04 -- crates/files/tests      # (empty: ported tests unchanged)
cargo fmt --all -- --check: OK; cargo clippy -p chatbotx-files --all-targets -- -D warnings: OK
Running unittests src/lib.rs          test result: ok. 41 passed; 0 failed
Running tests/bounded_fetch.rs        test result: ok. 8 passed; 0 failed
Running tests/channel_media_prefix.rs test result: ok. 2 passed; 0 failed
Running tests/copy_source.rs          test result: ok. 5 passed; 0 failed
Running tests/mime_type.rs            test result: ok. 11 passed; 0 failed
Running tests/upload_from_url.rs      test result: ok. 7 passed; 0 failed
Running tests/upload_writer.rs        test result: ok. 7 passed; 0 failed
```
Verifier note: the abort-signal port (`upload_from_url`) checks "at least one fetch/put token is cancelled with the caller's token" — weaker than JS signal identity, accepted (child tokens).

### Proof P1.1

Spec `conversion/specs/P1.1.md` → implementer → independent verifier (PASS, 6/6). Harness: `conversion/parity/` (TypeScript + vitest, own package, not in the pnpm workspace). Run: `~/.local/bin/chatbotx-dell bash conversion/parity/stacks/original/run-original.sh --no-build`.

```
 ✓ tests/smoke/03-fixtures.test.ts (5 tests)
 ✓ tests/smoke/04-public-api.test.ts (2 tests)
 ✓ tests/smoke/05-upstream.test.ts (3 tests)   (DoH on 1.1.1.1 + TLS via test CA + pinned dial)
 ✓ tests/smoke/06-realtime.test.ts (2 tests)
 ✓ tests/smoke/99-interception.test.ts (1 test)
 Test Files  7 passed (7)
      Tests  17 passed (17)
[run-original] vitest exit code 0
```
Verifier rerun (clean volumes): Test Files 7 passed (7), Tests 17 passed (17). Stack down after the run; no containers left.
The harness is NOT frozen yet; it is frozen in P1.28 after all Phase 1 tests pass.

### Proof P1.2

Writer (analysis + tests, spec `conversion/specs/P1.2.md`) → independent verifier (PASS 5/5, two runs for flakiness). 52/52 operations, 260 tests in `conversion/parity/tests/p1-02-ads-ai/` + the interception self-check.

```
verifier run 1:  Test Files 12 passed (12)   Tests 261 passed (261)   30.31s   exit 0
verifier run 2:  Test Files 12 passed (12)   Tests 261 passed (261)   29.81s   exit 0
writer run:      [run-original] vitest exit code 0; stopping the stack (down -v)
```
Known gaps (in the spec): 503 reportTimeout, 429 rate limits, three 400 codes that need rows the app never writes, Graph paging > 20 pages, worker retries, aiFiles reindex 409/coalesced (timing), enqueueFailed/502, template locks, aiFunctions with a real flow.

### Proof P2.3

Analyst spec `conversion/specs/P2.3.md` → porter (48 Rust tests, failing, `ecb4fa5a5`; rustfmt-only commits `14a519f85`, `cc571cb45`) → implementer → independent verifier (items 1, 2, 4, 5, 6 PASS; item 3 FAIL only on rustfmt of 6 test files → fixed in `cc571cb45`, formatting only, rerun below).

```
$ chatbotx-dell bash -c 'cargo fmt -p chatbotx-db -- --check && cargo clippy -p chatbotx-db --all-targets -- -D warnings && cargo test -p chatbotx-db'
FMT_OK
    Finished `dev` profile [unoptimized + debuginfo] target(s)      (clippy clean)
passed 113 failed 0      (48 ported in 18 files + 65 unit tests)
test map: 59 cases = 26 ported, 21 adapted, 6 replacement, 6 not-applicable (user decision)
0001_core.sql: 65 STRICT tables, 81 json_valid, 82 enum CHECKs, no part-2/3 FKs
idle RSS (writer + 3 readers, one read + one write, release): VmHWM 10,092 kB
```

### Proof P1.3

Writer (spec `conversion/specs/P1.3.md`) → independent verifier (PASS 5/5, two runs). 49/49 operations, 389 tests in `conversion/parity/tests/p1-03-ai-handover-analytics/` + interception check.

```
verifier run 1:  Test Files 13 passed (13)   Tests 390 passed (390)   26.49s   exit 0
verifier run 2:  Test Files 13 passed (13)   Tests 390 passed (390)   26.32s   exit 0
```
Analytics numbers asserted in UTC, Asia/Ho_Chi_Minh and America/New_York across the 2025-03-09 DST change.

### Proof P1.4

Writer (spec `conversion/specs/P1.4.md`) → independent verifier (PASS 6/6, two green runs; 12 operations cross-checked against the source). 55/55 operations, 355 tests in `conversion/parity/tests/p1-04-appointments-broadcasts/` (13 normalized snapshot files) + interception check.

```
verifier run 1:  Test Files 15 passed (15)   Tests 356 passed (356)   348.59s   exit 0
verifier run 2:  Test Files 15 passed (15)   Tests 356 passed (356)   395.58s   exit 0
```
Two earlier run-2 attempts failed before any test ran (infra): worker boot wait timed out under load (exit 4); rsync collision with another unit's sync (exit 23). See DECISIONS "Harness infra flakiness".

### Proof P1.5

Writer (spec `conversion/specs/P1.5.md`) → independent verifier (PASS 6/6, two runs; 8 operations cross-checked). 25/25 operations, 210 tests in `conversion/parity/tests/p1-05-channels-connections/` + interception check.

```
verifier run 1:  Test Files 10 passed (10)   Tests 211 passed (211)   85.10s   EXIT=0
verifier run 2:  Test Files 10 passed (10)   Tests 211 passed (211)   89.99s   EXIT=0
```
Minor gaps: `connectTargets` outcomes failed/notSelectable, failed/inProgress, limitReached; some unused fake scripts on shared Graph paths may remain after a test.

### Proof P1.6

Writer (spec `conversion/specs/P1.6.md`) → independent verifier (PASS 6/6, two runs; 10 operations cross-checked incl. search/sort/count rules). 58/58 operations, 539 tests in `conversion/parity/tests/p1-06-contacts/` (11 normalized snapshot files) + interception check.

```
verifier run 1:  Test Files 14 passed (14)   Tests 540 passed (540)   35.56s
verifier run 2:  Test Files 14 passed (14)   Tests 540 passed (540)   34.72s
```

### Proof P2.4

Analyst spec `conversion/specs/P2.4.md` → porter (36 tests, failing, `28cf15dd0`) → implementer (`59ae66f4f`) → independent verifier (PASS 6/6).

```
git diff --stat 28cf15dd0 HEAD -- crates/db/tests      (empty)
cargo fmt -p chatbotx-db -- --check: OK; clippy -D warnings: clean
cargo test -p chatbotx-db: 156 passed, 0 failed  (84 ported integration tests = 48 P2.3 + 36 P2.4; 72 unit tests incl. EXPLAIN QUERY PLAN index checks)
0002_messaging.sql: 65 STRICT tables, 71 enum CHECKs, 64 json_valid, 6 part-1 -> Flow ALTER columns + indexes, Message_workspaceId_sourceId_idx added
bundled SQLite has ENABLE_STAT4; PRAGMA optimize on open and after migrations
test map: 46 cases = 27 ported, 3 adapted, 6 replacement, 10 not-applicable
```

### Proof P1.7

Writer (spec `conversion/specs/P1.7.md`) → independent verifier (PASS 4/4, two runs; 7 operations cross-checked). 53/53 operations, 437 tests in `conversion/parity/tests/p1-07-conversations-decisions/` + interception check.

```
verifier run 1:  Test Files 12 passed (12)   Tests 438 passed (438)   42.60s   exit 0
verifier run 2:  Test Files 12 passed (12)   Tests 438 passed (438)   54.33s   exit 0
```
Minor: `decisionProfiles.get` has no stand-alone happy test (read back after create; 404/422 tested).

### Proof P1.8

Writer (spec `conversion/specs/P1.8.md`) → independent verifier (PASS 4/4, two runs; 8 operations cross-checked incl. encrypted grant/credential format). 48/48 operations, 335 tests in `conversion/parity/tests/p1-08-ecommerce-webhooks-fb/` + interception check.

```
verifier run 1:  Test Files 15 passed (15)   Tests 336 passed (336)   127.86s   exit 0
verifier run 2:  Test Files 15 passed (15)   Tests 336 passed (336)   exit 0
```
Timing risk: `campaigns.send` waits up to 150 s for the worker (took 58–59 s).

### Proof P1.9

Writer (spec `conversion/specs/P1.9.md`) → independent verifier (PASS 5/5, two runs; Execute JavaScript appears only as a rejected unknown step type). 53/53 operations + `POST /api/hooks/<token>`, 457 tests in `conversion/parity/tests/p1-09-flows-webhooks-ig/` + interception check.

```
verifier run 1:  Test Files 18 passed (18)   Tests 458 passed (458)   63.76s   exit 0
verifier run 2:  Test Files 18 passed (18)   Tests 458 passed (458)   63.24s   exit 0
```
Minor gaps: hook `WORKSPACE_BLOCKED` 403 / `WORKSPACE_UNAVAILABLE` 410; per-webhook rate limit.

### Proof P1.10

Writer (spec `conversion/specs/P1.10.md`) → independent verifier (PASS 4/4, two runs; one earlier attempt stopped in rsync before vitest, infra only). 52/52 operations, 398 tests in `conversion/parity/tests/p1-10-inboxes-keywords-leaderboards/` + interception check.

```
verifier run 1:  Test Files 13 passed (13)   Tests 399 passed (399)   159.65s   exit 0
verifier run 2:  Test Files 13 passed (13)   Tests 399 passed (399)   141.44s   exit 0
```
Minor gaps: `perPage` > 50 clamp not proven (only 11 rows); leaderboards `adjust`/`export`/`triggerRewards` field names checked by error codes only; unlimited 200 for `DELETE graph.instagram.com /v*/me/messenger_profile` in the fake upstream (check P1.9 IG tests do not expect an error there at P1.28).

### Proof P1.11

Writer (spec `conversion/specs/P1.11.md`) → independent verifier (PASS 4/4, two runs). 52/52 operations, 344 tests in `conversion/parity/tests/p1-11-messages-messenger/` + interception check.

```
verifier run 1:  Test Files 11 passed (11)   Tests 344 passed (344)   74.57s   exit 0
verifier run 2:  Test Files 11 passed (11)   Tests 344 passed (344)   122.24s  exit 0
```
Minor gaps: `messengerTemplates.create` Meta errors checked by status only; foreign `moveFiles` not proven; `messages.create` with `files` and `update` with `newAttachmentPath` are testable (multipart / presigned PUT) but not written — add in P1.28 gap pass.

### Proof P2.5

Analyst spec `conversion/specs/P2.5.md` → porter (37 tests, failing, `63beab6e2`) → implementer (`6839f5b73`) → independent verifier (PASS 9/9). Migration `0004_rest.sql`: 130 tables (all STRICT), 358 indexes, 300 FKs, 12 rollup triggers; `vector32` SQL function; exact cosine scan with `INDEXED BY "AIEmbedding_aiFileId_chunkIndex_idx"`.

```
git diff --stat 63beab6e2 HEAD -- crates/db/tests   (only the lint-only allow line from d0e4a791b)
cargo fmt --check: OK; cargo clippy -p chatbotx-db --all-targets -- -D warnings: clean
cargo test -p chatbotx-db: 121 integration passed (84 + 37), 88 unit passed, 0 failed
cargo test -p chatbotx-jobs: 493 passed, 0 failed
vector scan, 5,000 chunks, LIMIT 10, release on the Dell: median 28.4 ms (20 runs)
```
Verifier checks: rollups equal GROUP BY after insert/update/delete/cascade (NULL, '' and 0 separate); Asia/Kolkata day split matches the original `message-stats.repository.ts:213` (rollup path buckets at UTC hours); zero vector → NULL sorted last = pgvector NaN sorted last, callers treat both the same.

### Proof P2.6

Analyst spec `conversion/specs/P2.6.md` → porter (455 tests, failing, `9895f4db9`) → implementer (`c05a22f96`; lint-only allows `d0e4a791b`) → independent verifier (PASS 6/6; user decisions confirmed in code: per-job retention, UTC cron, quota skip on community, BullMQ backoff, workspace filter in every sequence query, no write tx across network I/O, no idle polling).

```
git diff 9895f4db9 HEAD -- crates/jobs/tests    (only 3 one-line clippy allow attributes)
cargo fmt --check: OK; cargo clippy -p chatbotx-jobs --all-targets -- -D warnings: clean
cargo test -p chatbotx-jobs: 493 passed, 0 failed (455 ported = 335 ported + 49 adapted + 71 formerly blocked-by-P2.4; 38 unit)
idle (release example, 17 workers + sequence producer + 4 event-bus consumers, 30 s): 0.17 context switches/s, VmHWM 16.4 MB; with 44 crons 4.0/s (9 cron runs), 15.7 MB
```
Open for later units: flock guard against a second process, producer claim+enqueue in one tx, group commit for event emits, real `SequenceConsumerServices`, typed event payloads, `*_WORKER_CONCURRENCY` envs (worker boot tests, Phase 3).
