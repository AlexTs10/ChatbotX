# Decisions

Dated log of each decision and each question to the user. Newest entry at the bottom.

## 2026-11-11 — P0.1 setup

- The user has READ access only to `ChatbotXIO/ChatbotX`. So the work is on a fork: `AlexTs10/ChatbotX` (`origin`). `upstream` points to `ChatbotXIO/ChatbotX`.
- Clone location: `~/Desktop/ChatbotX-lean`. The old clone at `~/Desktop/ChatbotX` (tracks upstream `main`) is not changed.
- Baseline: upstream `main` at `40e733cee` (2026-11-11).
- The orchestrator prompt says "about 2,779 test files". On the baseline commit, `git ls-files | grep -E '\.(test|spec)\.(ts|tsx|js|jsx|mts|cts)$'` gives **3,100** files. P0.2 must freeze the real set (and also test helpers/fixtures that tests import), not the old number.
- The manifest check (`shasum -a 256 -c MANIFEST.sha256`) is not possible in P0.1, because `frozen-tests/` is empty until P0.2. From P0.2, it runs before each commit.
- Upstream `main` moved to `f371c5c5a` (1 commit) after the clone. The baseline stays `40e733cee`. Do not merge upstream into `conversion` without a new decision.

## 2026-11-11 — Feature scope (user decision, given before P0.4)

- User decision: **drop** the "Execute JavaScript" flow step (custom JavaScript executor). **Keep all other features.**
- Effect of the drop:
  - Delete `apps/javascript-executor` and `packages/javascript-sandbox`.
  - Delete the builder step `apps/builder/src/features/flows/react-flow/steps/execute-javascript/` and its menu entry and registration.
  - Delete the worker handler path in `apps/worker/src/integration/handlers/flow-utils.ts` and the `execute-javascript` i18n strings.
  - The Rust `js` crate (`rquickjs`) is **not** built.
- Tests of the dropped feature are marked `dropped` in TEST_MAP.md (P0.5), never deleted silently. Known now: 5 test files in `apps/javascript-executor` + `packages/javascript-sandbox`, `apps/builder/.../steps/__tests__/execute-javascript-editor.test.tsx`, `apps/builder/.../steps/__tests__/execute-javascript-registration.test.ts`, `apps/worker/__tests__/execute-javascript-handler.test.ts`. P0.5 must search for more.
- P0.4 still makes the inventory (LOC + test count for each feature), but it does not ask the user again. It writes `keep` for each feature, except `drop` for Execute JavaScript.
- Risk to watch (not a new decision): with all other features kept, the 200 MB RSS budget is tight. P0.6 and Phase 5 measure it.

## 2026-11-11 — P0.2 frozen set

- The frozen set includes test support files (helpers, fixtures, mocks, Vitest configs), not only `*.test.*` files, because the tests import them. 3,104 test files + 115 support files = 3,219.
- `MANIFEST.sha256` itself has SHA-256 `09e8faff6524ab94ca17b9ce8dae03419229df71afac0cb5277f9858a6c850f8`. If this hash changes, the manifest was edited.
- The originals in `apps/`, `packages/`, `integrations/` can later be deleted at cutover. The copies in `frozen-tests/` never change.

## 2026-11-11 — Heavy work and baseline (user decision)

- User: do not run heavy work on the Mac. Heavy commands (pnpm test/build, tsc, cargo, Docker) run on the Dell through a private helper `~/.local/bin/chatbotx-dell` (not in the repo). It copies the worktree to `dell-5570:~/chatbotx-remote/ChatbotX-lean` and runs with Node 24.22.1, pnpm 10.34.5, Rust stable.
- User: no full benchmark of the original stack. Baseline RSS = **2 GB** (user-set). P0.6 is done with this value.
- User: do units back to back; do not stop after one unit.
- P0.3 (baseline test run) runs on the Dell. The stack services (Postgres/Redis) are not started for it; tests that need them are recorded as `baseline-fail` with reason `needs-service`.
- P0.4 analyst note: `packages/variables/src/javascript-interpolation.ts` (+ `interpolate-into-javascript.test.ts`) is used only by the Execute JavaScript step? To check when the step is deleted. If it is used only there, it is dropped too.
- The repo commit-msg hook (lefthook) rejects the `conversion(<ID>): …` format. Conversion commits use `git commit -n` only when the staged files are all under `conversion/` or Rust (no TS staged), so no lint hook is skipped for TS code; the pre-commit hooks still run.

## 2026-11-11 — P0.5 classification

- Original tests are only class B or C. Class A tests are new (Phase 1, `conversion/parity/`).
- `apps/cli` and `apps/mcp-server` stay TypeScript. They are HTTP clients of the public API (no business/database imports). Their tests are class B and must pass unchanged against the Rust API.
- `packages/variables/src/javascript-interpolation.ts` and its test are part of the dropped Execute JavaScript feature → `dropped`.
- Doubt to settle in Phase 3 (not blocking): pure zod schemas in `packages/flow-config` are class C now. If the TanStack frontend still imports that package, such tests can move to B; record the move here if it happens.
- Phase 1 has 28 units: 17 public-API groups (811 operations in the committed public spec), internal `/rpc`, 7 route-handler/webhook groups, realtime, and the final freeze.

## 2026-11-11 — P1.1 harness decisions (orchestrator; tell the user, change if they object)

Spec: `conversion/specs/P1.1.md`. Answers to its 5 open questions:
1. **Fixture driver = direct DB inserts, one adapter for each stack.** The setup (user, workspace, API token, platform credentials, channels) is written straight to the database: a Postgres adapter for the original stack, a SQLite adapter for the Rust stack. Build-specific Next.js server-action IDs are not used. All **assertions** stay black-box (HTTP, WebSocket, fake upstream records, files). The sign-up + email-verify + sign-in path is still tested over HTTP once (smoke test).
2. The Rust stack does **not** keep the Postgres schema (target is libSQL). So the SQLite adapter is written in Phase 2/3 from the same fixture spec (same logical rows).
3. The 514 server-action files become Rust HTTP routes in Phase 4. They have no stable HTTP contract in the original, so Class A does not cover them; their behavior is covered by Class B/C tests. No new Phase 1 units.
4. Parity runs the **community** edition only. Paths gated to `cloud`/`enterprise` editions (private quota worker, signed licence) get no Class A tests; their Class C tests still port.
5. The parity Docker network maps `1.1.1.1` (hard-coded DoH resolver of the SSRF guard) and the real upstream hostnames to the fake upstream, with a test CA trusted through `NODE_EXTRA_CA_CERTS` (Node) and `SSL_CERT_FILE` (Rust). Test-only; production code is not changed.

- Dell: user `alextoska` added to group `docker`; Docker service enabled (2026-11-11, with the user's sudo password, given through a one-use secret card).
- Dell memory: the Android emulator (~5 GB, Nana QA) runs there. It is not stopped by this project. If `next build` runs out of memory, the build is retried with lower concurrency; if it still fails, ask the user to stop the emulator.

## 2026-11-11 — P2.9 files crate (orchestrator)

Spec: `conversion/specs/P2.9.md` (40 test cases from 6 frozen files).
- Backend by config: `FILES_BACKEND=local|s3` (default `local`), `FILES_LOCAL_ROOT` (default `$DATA_DIR/files`). The `S3_*` variables keep their original names.
- `public-read` ACL: on `local`, the app serves keys under `public/` without auth; on `s3`, a bucket policy on `public/` (documented, not set by the app). The ACL argument stays in the API for parity.
- Presigned URLs on `local`: HMAC-SHA256-signed app URLs with the same expiry (upload 300 s, download default 3600 s). The route that serves them comes in a Phase 3/4 unit.
- `packages/filesystem/src/lib/node-upload.ts` has no importers (dead code) → not ported. It has no tests.
- `build_copy_source` is ported (tests 11–15) even if `object_store` builds its own copy source.
- Image dimensions: if a test needs SVG sizes, parse `width`/`height`/`viewBox` with std; no new crate for SVG.
- Clock and HTTP: injected (`*_at(.., date)` variants and an `HttpFetcher` trait), so ported tests need no global mocks. The assertions stay the same.

## 2026-11-11 — P2.2 schema mapping (orchestrator) + one question to the user

From `conversion/specs/P2.2.md`, adopted:
- Ids: app-generated snowflakes → `INTEGER PRIMARY KEY`; the API returns them as strings (as now).
- Timestamps: `INTEGER` unix ms; JSON output keeps `toISOString()` format.
- Money: `INTEGER` × 10,000; the 3 unbounded `numeric` columns → `TEXT` + `rust_decimal`.
- jsonb/arrays → `TEXT` + `json_valid` CHECK; pgEnum → `TEXT CHECK(IN …)`; vector(1536) → `F32_BLOB(1536)` layout, exact filtered cosine scan.
- TimescaleDB → plain tables + time index + retention jobs + rollup tables in the same write tx. Sharding not used.
- Locks → `BEGIN IMMEDIATE` (one writer) + lease-based claims; keep transactions short, never hold a write tx across network I/O.
- Semantic traps to test for: enum sort order (declaration vs alphabetical), `GREATEST`/`LEAST` NULLs, ASCII-only `LIKE`/`lower()`, 0/1 booleans, aggregate types.

**Question to the user (P2.3 blocked until answered):** the prompt names Turso/libSQL. The analyst recommends `rusqlite` (bundled SQLite) because it can register custom SQL functions (Unicode lower, ilike, time zone, regexp, cosine distance), and it is the most mature crate. The file stays a plain SQLite file with the libSQL vector byte layout, so a later move to libSQL is small.
- **User answer 2026-11-11: use `rusqlite` (bundled SQLite).** P2.3 is unblocked.

## 2026-11-11 — P2.3 disputed tests (user decision)

- **User approved** the 12 `disputed` rows of `conversion/specs/P2.3.md` section 5 as proposed: use the SQLite replacement checks where the spec gives one; Postgres-only mechanics (hash partitions, fillfactor, migration-file text, TS type-level checks) are `not-applicable` with the reason in TEST_MAP.
- **No import of existing Postgres data.** The 4 better-auth 1.6→1.7 data-migration tests (`integration/better-auth-1-7-migration.test.ts:92,101,109,116`) are `not-applicable`.
- better-auth table contract (`packages/auth/__tests__/schema-parity.test.ts:34`): generate the `getExpectedSchema` output once from the TS repo into `conversion/fixtures/better-auth-expected-schema.json`, add it to `MANIFEST.sha256` (frozen), and assert against it in Rust.
- P2.3 analyst choices adopted: no DB default `'[]'` for the 10 JSON columns and no extra FK indexes (both pinned by frozen tests); double-quoted string literals OFF on every connection; Rust always supplies ids; one writer thread + 3 readers, cache 8+3×4 MiB, soft heap limit 32 MiB, mmap off.

## 2026-11-11 — P2.9 files crate implementer choices (orchestrator)

- New env names: `FILES_SIGNING_SECRET` (HMAC key; local presign fails without it), `FILES_URL_BASE` (default `/files`), `FILES_LOCAL_ROOT` default `$DATA_DIR/files` (else `./data/files`).
- The local backend keeps no content-type/cache-control/content-disposition (object_store LocalFileSystem limit). The files route that serves local objects (Phase 3) must derive content-type from the key/MIME helpers.
- `object_store` `aws` feature pulls ~180 crates. Accepted for now (S3 by config is a requirement). Revisit in Phase 5 if binary size or RSS is too high (option: feature-gate `s3`).

## 2026-11-11 — P1.1 harness findings (orchestrator)

- Original stack idle RSS, measured once by the P1.1 run (not a benchmark): ≈ 1.52 GB (builder 709 MiB, worker 462, postgres 154, realtime 93, others < 50). Recorded in MEMORY.md.
- Possible SSRF/DNS-rebinding gap in the original: `packages/business/src/decision/transport.ts` "pinned dial" still does a normal DNS lookup after the DoH check. The Rust port must pin the connection to the DoH-resolved IP (no second lookup). Parity tests do not assert the extra lookup, so this is not a parity break.
- The Rust stack must: accept `MCP_PUBLIC_URL=https://app/mcp` in the parity env; pass a single-value `X-Forwarded-For` through the edge (better-auth rate limits per IP).
- Snapshots made with `--update-snapshots` on the Dell must be copied back to the Mac before the next `chatbotx-dell` sync (it deletes PC-only files).

## 2026-11-11 — Phase 1 work method (orchestrator)

- For each Phase 1 unit (P1.2–P1.27), one subagent does the analysis and writes the parity tests (the "implementation" under test is the original stack, which already exists), and a separate independent verifier checks the tests against the original source and reruns them. The spec goes to `conversion/specs/<ID>.md` as usual.
- Up to 3 Phase 1 units run in parallel. Each unit writes only under `conversion/parity/tests/<unit-folder>/`, `src/fixtures/ext/<unit>.ts` and `stacks/original/ext/<unit>.ts` (loader: `src/fixtures/ext.ts`). Shared harness files (`src/*.ts`, `driver.ts`, `stacks/original/fixtures.ts`, compose, Caddyfile, run script) change only through the orchestrator.
- Each unit runs only its own folder on the Dell: `run-original.sh --no-build -- tests/<unit-folder>`. Runs queue behind the `chatbotx-dell` lock.
- Frozen fixtures live in `conversion/fixtures/` with their own `MANIFEST.sha256`; `check-manifest.sh` checks both folders. First entry: `better-auth-expected-schema.json` (better-auth 1.7.14 `getExpectedSchema`, sha256 `c518f2dd…7647`), made by the P2.3 porter from the frozen test options.

## 2026-11-11 — P2.6 jobs (user decisions)

Spec: `conversion/specs/P2.6.md` (19 queues, 44 cron schedulers, 381 frozen cases in 50 files).
- **Cron time zone: UTC only** (user). No `TZ` support in the scheduler.
- **Retention: per-job** (user). Each finished job row is deleted only by its own rule (age or count). This differs from BullMQ's queue-wide trim (likely a bug in the original); not visible to parity tests.
- **`quota` queue: skip enqueue on the community edition** (user). The private enterprise quota-worker is out of scope.
- **Bull Board: replaced by a small read-only page** (user) in Phase 4: counts per state, failed jobs with reason, retry/remove buttons. Added as a Phase 4 unit.
- Adopted from the spec: one `jobs` table (unique (queue, job_id)), `job_dedup` table with BullMQ's 4 dedup modes, batch claim with lease, BullMQ backoff formulas, tokio Notify wake-up, per-queue semaphore + rate limiter, std-only cron, in-process broadcast for QueueEvents, `sequence_schedule` and `event_entry` tables. Visible improvements accepted: sequence sends at due time (not up to 5 s late); batch schedule writes all-or-nothing (the frozen `smoke.test.ts:10` port asserts the error and no row written — marked `adapted`).
- The 105 sequence-scheduler cases need the P2.4 schema: port them failing and mark them blocked by P2.4 if P2.4 is not done.
- P2.3: the orchestrator ran `cargo fmt -p chatbotx-db` on the ported test files (formatting only: import order and line breaks; no token change other than whitespace and commas). Separate commit, so the verifier can check it.

## 2026-11-11 — Phase 1 findings to carry into Phase 3 (orchestrator)

- P1.2: behavior of the original that the parity tests now lock in (the Rust port must match): `ads.listCustomAudiences` / `startRetargetAudienceSync` without a Facebook Ads connection → 500 `INTERNAL_SERVER_ERROR`; `ads.createCampaign` writes the draft before the connection check (row stays `pending` after a 409); `ads.deleteCampaign` → 200 and keeps the row; `ads.updateRuleLegacy` (PUT) is a partial update; aiAgents PATCH accepts `actionPrompt`/`actionRules` (create rejects them); delete of an unknown aiAgents id → 204, unknown aiFunctions/aiFiles id → 404.
- P1.3: some analytics routes return 500 for an unparsable `from`/`to`, an unknown time zone, or a non-numeric id. These cases are **not** asserted by parity tests. Open (not blocking): the Rust port may return 422 there; decide in the Phase 3 analytics unit and record it. `InboxContactStat` is kept by a DB trigger (the SQLite schema has it, P2.3).

## 2026-11-11 — P2.4 schema part 2 (orchestrator)

Spec: `conversion/specs/P2.4.md` (65 tables, 234 indexes, 155 FKs; 46 runtime cases: 27 1:1, 9 replacement/adapted, 10 not-applicable per the P2.3 user decision).
1. Message dedup key stays 3 columns `(contactInboxId, sourceId, createdAt)` (a frozen test pins this conflict target).
2. Composite PKs (`Message`, `Attachment`, `ContactOnSequence`, `SequenceDispatch`) → `PRIMARY KEY (id)` + `UNIQUE (id, workspaceId)` where a composite FK needs it.
3. Add the SQLite-only index `Message (workspaceId, sourceId)` for per-webhook "update by source id" queries, **unless** a frozen test pins the exact index set of `Message` (the implementer checks; if pinned, leave it out and note it for Phase 5).
4. `crates/db/src/error.rs` reads the CHECK constraint name from the SQLite message (two frozen tests assert it). This change to P2.3 code is part of P2.4.
5. Planner statistics: build bundled SQLite with `SQLITE_ENABLE_STAT4` if rusqlite allows it through `LIBSQLITE3_FLAGS` (else note it); run `PRAGMA optimize` on open and after migrations. Message hot queries must pick the spec's index (check with `EXPLAIN QUERY PLAN` in a unit test).
- The P2.3 migration-runner unit test that expects exactly 1 migration / 65 tables is updated by the P2.4 implementer (it is the implementer's own unit test, not a ported test).

## 2026-11-11 — Harness fixes after P1.4 (orchestrator)

- `run-original.sh`: `--update-snapshots` now appends `--update` after the test filters (before, `-u <path>` made vitest read the path as the flag's value and run every test).
- `chatbotx-dell` (private helper): rsync protects `**/__snapshots__/**` on the PC from `--delete`, so new snapshots made by a run are not deleted by another unit's sync. An existing snapshot that changed on the PC can still be overwritten by the Mac copy, so units copy snapshots back at once and **prefer explicit assertions over snapshots**.
- Parallel units use a per-unit subfolder in the scratchpad for logs.
- P1.4 findings the Rust port must match: `broadcasts.moveToDraft` on a non-scheduled broadcast → 400 `systemError` (route doc says 404); webchat allowed domains limit the bot-simulator link only after `PATCH /v1/webchats/{id}`, not on create; a broadcast needs `contactFilter` (empty `conditions` accepted); invalid bot-field value → 400 `invalidCustomFieldValue`; `finalizeBroadcasts` runs once a minute (a broadcast reaches `sent` in up to ~2 min).
- P1.6 findings the Rust port must match (details: `conversion/specs/P1.6.md` §5): contact keyword search never matches the full name, does not fold accents, does fold non-ASCII case, works for 1–2 character keywords (the FTS5 trigram index needs a LIKE fallback for < 3 chars), `@` → email only, phone needs ≥ 6 digits, `sourceId` exact only; `count` without a filter sums per-inbox counters (a contact on two inboxes counts twice); sort has no id tie-break, NULLs last asc / first desc; array query params need `key[]=`; a bad filter date drops the condition; avatars for messenger/instagram without one get a `/media/avatar/<token>` URL (token changes per read); export prefixes `'` before `+ = - @`.

## 2026-11-11 — Harness infra flakiness (orchestrator)

- Seen in P1.4 verification: `run-original.sh` boot wait for "Trigger worker started" can time out when the Dell is loaded (exit 4), and a `chatbotx-dell` rsync can collide with another unit's sync (exit 23). Neither is a test failure. Rule: a run that fails before vitest starts is retried; it does not count as a red run. To do before P1.28 (full run + freeze): raise the boot wait time-out, and make `chatbotx-dell` retry rsync once on exit 23/24.
- P1.5 findings the Rust port must match (`conversion/specs/P1.5.md`): `connectionProviders.list` → 422 `invalidRequestData` (bare zod issue array, `defined: false`) unless `kind=integration` (likely defect: the `chatbotx` adapter name is not a `channelTypes` value); a disconnected `api` channel's `cbx_api_` token still authenticates; disconnecting an integration-kind connection cascades the `Connection` row (later GET 404, reconnect = new id); provider-config ZodError → 422 with `data` = bare issue array; `connectSessionLimitReached` 429 is `defined: false`; a Telegram bot id is unique across workspaces (409 `connectionAlreadyConnected`); a Messenger subscribe that always fails still connects as `degraded`/`verify_failed`.
- Open for the Phase 3 contacts unit: phone normalization of `+15550001111` was not deterministic in the original (once `+848864158`, once unchanged). Parity tests do not assert it. Find the cause before porting.
- Checked: the P1.5 note that the P1.2 fixture encrypts `IntegrationMessenger.auth` is wrong — P1.2 writes it as plain JSON (only `MessagingAdsConnection.auth` is encrypted, as in the app).

## 2026-11-11 — P2.6 porter results (user decision + orchestrator)

- **User decision:** `packages/sequence-scheduler/__tests__/dispatch-manager.test.ts:312` ("calls useExisting to obtain the redis client before scheduling removal") is **not-applicable** — the Redis handle does not exist in Rust; removal itself is covered by the other dispatch-manager tests.
- Sequence test seeds use the P2.4 DDL (integer snowflake ids, exact column names); assertions compare with the decimal string of the seeded id (API returns ids as strings). 76 tests changed only in this id mapping (noted in the tsv).
- 4 ported tests are marked `adapted (weakened)`: the composite FK `(enrollmentId, workspaceId)` forces the other-workspace decoy to use a different enrollment id, so the workspace filter is not checked alone (`cancel_pending_dispatches::select_filters_enrollmentid_workspaceid_status_pending`, `cancel_pending_dispatches::update_where_has_id_workspaceid_status_redis_removed_per`, `sequence_dispatch::update_where_includes_workspaceid_and_status_pending`, `sequence_dispatch::uses_the_provided_workspaceid_and_enrollmentid_when`). The implementer must still filter by workspace (code review in verification).
- Log format pinned by the ports: snake_case fields, Debug lists, "Unhandled rejection" at ERROR (tracing has no fatal).

## 2026-11-11 — Subagent limit (user)

- **User: at most 1–2 subagents at the same time** (helpers spawned by a subagent count too). Phase 1 units no longer run 3 in parallel.

## 2026-11-11 — Findings from P1.7–P1.9 (orchestrator; details in each spec)

- **Security (P1.7):** `decisionConnections.test` follows a provider 3xx redirect with a GET, and the redirect target gets **no DoH/SSRF check**. The Rust port must re-check every redirect hop (as `crates/files` `fetch_following_safe_redirects` does). Parity tests do not assert the unsafe follow.
- **Security (P1.8):** dynamic-image static `url` layers are fetched without the SSRF guard (only `customField` layers are checked). Rust port: guard all fetched URLs. Not asserted by parity tests.
- **Infra (P1.9):** the daily `googleAdsSyncSetups` cron runs at 03:30 UTC; a run that crosses 03:30 UTC with a test Google Ads connection can make an unscripted upstream call and fail `99-interception`. Do not schedule full runs across 03:30 UTC (or script that call in P1.28).
- **Harness (P1.8):** marketing-messages tests script shared Graph paths (`/debug_token`, `/me/adaccounts`); correct only while `3x` files run after `0x` files. Optional later: a query matcher in `fake-upstream/store.ts` (orchestrator change).
- Behaviors the Rust port must match are listed in `conversion/specs/P1.7.md` (conversations list ignores `status`; coupon `issueCoupon` 500 `couponIssueUnavailable`, `markCouponUsed` 500 `couponNotIssued`; discountCodes create 200; customFields delete 204 for unknown ids; same-second thread-control precedence), `P1.8.md` §13, `P1.9.md` §8 (flows publish keeps old `startNodeId`; `folders.list` without `parentId` lists sub-folders; flows delete unknown → 204; compile errors 422 bare array).

## 2026-11-11 — P2.5 schema part 3 (orchestrator)

Spec: `conversion/specs/P2.5.md` (126 tables + 4 rollup tables, 358 indexes, 300 FKs, 12 triggers; 43 cases = 31 1:1, 5 adapted, 1 replacement, 6 not-applicable). Kept as one unit, migration version 4 (after `0003_jobs.sql` from P2.6).
1. Vector columns: `BLOB` (6,144 bytes, libSQL byte layout) in STRICT tables, not the `F32_BLOB(1536)` type name (STRICT allows only base types). Exact cosine scan; the file search uses `INDEXED BY "AIEmbedding_aiFileId_chunkIndex_idx"`.
2. Rollups are kept by **SQLite triggers** in the same transaction (they also catch FK cascades and backfills), not by Rust code as P2.2 proposed.
3. No production access for ChatbotX, so the real chunk counts are unknown. Risk accepted: an agent with ≥ 50,000 chunks makes the scan slow; revisit in Phase 5 (option: libSQL vector index).
4. The HNSW index is not created (no migration ever made it). `AIAgent.webSearchAuthorizedDomains` gets `DEFAULT '[]'` (production has it) **unless** a frozen test pins "no DB default" for it — the implementer checks.
5. The 4 `lower_u` indexes are accepted (tools without the app functions cannot write those 4 tables).
- Retention: new cron `purgeAnalyticsEvents` on the `schedule` queue at 03:45 UTC (events 10 years; rollups 3 years, message rollup 10 years). `maintainMacPartitions` becomes a no-op; `maintainLeaderboardPartitions` deletes in chunks. These go into the jobs crate in Phase 3 (worker handlers).
- The P2.3 ports `schema_default_parity` (≥ 10) and `workspace_purge_coverage` (≥ 18) used minimum limits because parts 2–3 did not exist. Now that the full schema exists, the **P2.5 porter** sets them to the frozen originals' exact values (16 columns, 83 baseline entries). This makes the ports equal to the originals, not weaker; recorded here.
- P2.6 gate: the orchestrator added one file-level `#![allow(clippy::<lint>)]` line to 4 ported test files (`template_queue.rs` assertions_on_constants, `dispatch_manager.rs` let_and_return, `queue_events.rs` await_holding_lock, `crates/db/tests/integration_google_ads_conversion_event_fk.rs` type_complexity). Lint only, commit `d0e4a791b`; no test logic changed.
- P1.10 findings (`conversion/specs/P1.10.md` §9): `inboxes.list` without `perPage` returns up to 20 rows but counts pages of 10 (with `perPage` > 50: 50 rows, pages counted with the requested size); `integrations.list` empty → `pageCount: 1`; `integrations.get` returns a `metaCatalog` row that `list` hides; Instagram settings PUT on unknown id → 400 `systemError` (PATCH → 404); keyword create with unknown flow → 404 `defined:false`, update → 422; DELETE keyword reads `type` only from the JSON body; oRPC coerces `"on"`/`"off"` to booleans; leaderboard CSV formula guard also prefixes phone numbers (`'+84…`); manual score adjust ignores the time window and never goes below 0.
- Infra: daily Instagram/Zalo token refresh crons at 02:00 and 14:00 UTC can make unscripted upstream calls when fixture rows exist (like the 03:30 Google Ads cron). The P1.28 full run must avoid 02:00, 03:30 and 14:00 UTC, or script those calls.
- P2.5 verifier notes for Phase 3: `recursive_triggers` is OFF, so `INSERT OR REPLACE` on an analytics event table skips the delete trigger and counts the rollup 2 times. Phase 3 code must not use `OR REPLACE` on event tables (use `ON CONFLICT DO NOTHING/UPDATE`). `vector32` rejects `.5`/`+1` and accepts `[]` (pgvector does the reverse); JS `join(",")` never makes these forms. Hour bucketing `(ms / 3600000) * 3600000` differs from `time_bucket` only before 1970.

## 2026-11-12 — P2.7 Redis replacement (user decision + orchestrator)

- **User decision:** the 8 candidate cases of `conversion/specs/P2.7.md` §3.4 (T2 legacy plain-JSON cache key; T16, T26, T27, T36 Lua `defineCommand` once per client; T71, T72, T78 store-down fallbacks) are **not-applicable**. The other 73 cases are ported.
- Orchestrator: keep the spec's durable list (§1.6) as written, including the borderline K14, K35, K36 and K40 rows, and the live quota counters K25–K28 (on community the live fields are the only copy). Cost is a few short writes; the gain is no visible loss on restart.
- New crate `crates/kv` (`chatbotx-kv`) between `db` and `jobs`; `Clock` moves from `jobs` to `kv`; `LockMap` replaces the P2.6 interim `SqliteSchedule::try_lock`. Code rule for Phase 3: never call `LockMap::acquire` inside a `db.write` closure.
- P1.11 finding: a flow publish copies the flow to every Messenger page of the workspace (`POST /<pageId>/welcome_message_flows`); units that create Messenger pages and publish flows must script that call. Possible cross-workspace gap: `mediaLibrary` accepts a `folderId` of another workspace (FK check only) — the Phase 3 media unit decides. Full runs must also avoid 02:00 and 14:00 UTC (token refresh crons) — same as P1.10.
- Harness rule from the P1.11 verifier: fake-upstream scripts stay for the full run, so an unlimited script in one unit can answer an unscripted call of a later unit with success (order-dependent results). From P1.12 on, writers must not add unlimited scripts on shared paths; use `times` that the test consumes, or a path with a per-test unique id/token. The P1.28 full run is the check for order effects.
- Snowflake place ids: free ids are 0–4 and 13. From P1.12 on, one writer batch uses one place id and one ext file pair: P1.12–P1.14 → 4 (`p1-12.ts`), P1.15–P1.18 → 3, P1.19–P1.23 → 2, P1.24–P1.27 → 1; 0 and 13 stay spare.

## 2026-11-12 — Speed rules (user) and P2.8 decisions (user)

- User (2026-11-12): give each subagent a bigger batch; do not stop to ask — take the recommended option and record it here; start Phase 3 now, in parallel with the rest of Phase 1/2 (files do not overlap).
- Phase 3 work method (orchestrator, for speed): one agent first ports the batch's Class C tests (they must compile and fail); the orchestrator commits that state; then the same agent implements, with the tests locked. The independent verifier diffs the tests against the porter commit and checks the ports against the frozen originals. Analyst and porter are one step (the porter writes the spec/test map).
- P2.8 user decisions: the 51 candidate cases of `conversion/specs/P2.8.md` §4.3 are not-applicable; the presence report HTTP hop is removed (in-process `PresenceSink`); `frame-reader.test.ts` and `realtime-socket.test.ts` (21 cases, browser client) move from class C to class B. Orchestrator: D1 crate order `db <- kv <- {jobs, rt} <- core <- {api, server}`; D5 no permessage-deflate; D6 `REALTIME_BROADCAST_SECRET` stays required.
- P2.7: the ported test `crates/kv/tests/leaderboard_store.rs` got one file-level `#![allow(clippy::int_plus_one)]` line (lint only, same rule as the P2.6 gate); the crate keeps `[lints] workspace = true`.
- Infra (P2.7 check): with the Dell at load ~78 and swap full (other projects' tsc runs), 2 db unit tests failed once in `Db::open` on a temp file; the rerun passed (808/808). Treat a `Db::open`/time-out failure under such load as infra and rerun; if it repeats at normal load, open a db unit.

## 2026-11-12 — Speed rules 2 (user: "this goes way too slowly")

- Orchestrator, recommended option (user standing rule 2026-11-12): agents commit their own stage 1 (named paths, check-manifest, `git commit -n`) and go on to stage 2 at once; no wait for an orchestrator handshake. The stage 1 commit SHA stays the lock point.
- Verification of Phase 3 units is done by the orchestrator (not a separate agent): diff of the test files against the stage 1 commit, plus the full test, clippy and fmt gate on the Dell. Both agent slots are kept for implementation.
- Lint-only `#![allow(clippy::...)]` lines in locked test files are allowed (orchestrator only), as in P2.7. P3.1: 6 files; P3.2: 1 file, plus one rustfmt whitespace fix.
- P3.1 notes: JPEG pixels decode with `jpeg-decoder` (zune-jpeg misdecodes some 4:2:0 files). JSON object keys sort (no `preserve_order`). Invalid recurrence time zone falls back to UTC. Phone parity flake comes from `update-from-message.ts` digit scan of the message text; fix in the business port or the fixture.
- P3.2: scope was too large for one batch; the rest goes to new unit P3.2b. Postgres row/advisory locks are no-ops under the single SQLite writer.

## 2026-11-12 — P3.4 flow-config port (implementer; orchestrator-delegated, recommended options)

- Orchestrator-delegated, recommended option: P3.4 is cut in two slices (P3.4a schema core, 61 files; P3.4b graph/authoring, 13 files), each with its own stage 1/stage 2 commits. Spec: `conversion/specs/P3.4.md`.
- Orchestrator-delegated, recommended option: the zod schemas are rebuilt with a small zod-4 runtime (`chatbotx_core::flow_config::z`, same paths and English messages) from skeletons generated once by `conversion/tools/zod-rust-gen/gen.mts`; closures and functions are hand-ported. Flow data (steps, nodes, edges, template params) stays `serde_json::Value`.
- Orchestrator-delegated, recommended option: `executeJavascript` is removed from `stepTypes`, `actionSteps` and the channel policy step table (follows the user's drop of the step). A stored flow with that step no longer validates; there is no data import, so nothing breaks.

## 2026-11-12 — P3.3 implementer (orchestrator-delegated, recommended option)

- Slice order: the auth repositories first (`user`, `auth-account`, `invitation`, `license`, `mobile-refresh-token`, `oauth-*`, `platform-admin`, `tenant` + default template, `workspace`, `workspace-api-token`, `workspace-member`). `auth-account` is in the first half of the alphabet but had no tests and no P3.2 port; it is in P3.3 slice 1 because auth needs it.
- `logger` spies: a process-wide `tracing` recorder in `crates/db/tests/common/log.rs` (repository code logs on db threads). No new crate dependency.
- New shared test fixtures go to `crates/db/tests/common/repo3.rs`; the P3.2 helper file stays locked.
- Drizzle `.set()/.values()` objects with caller-chosen keys: `repo::patch::Patch`, checked against the model columns.
- Untested repositories get no map rows; `crates/db/tests/repo_p33_smoke.rs` (own test) calls each of their functions once against the schema.
- Stage 1 speed-up: src is written once, then a script replaces the `pub fn` bodies by `todo!("P3.3")` for the stage 1 commit (tests run only against the stubs before the commit; every stage 1 failure was checked to be the `todo!` panic, except slice 2a, see next point).
- **Open for the orchestrator:** `crates/db/tests/repo_trigger_repository.rs` (locked, slice 2a, `504a85889`) seeds two triggers named `Welcome` in one workspace, which violates `Trigger_workspaceId_name_key`; all 6 tests fail in setup. Proposed fix (fixture only): rename trigger 2 to `Welcome root` in `seeded()`. The implementation (`repo::trigger`) is done.
- 2026-11-12 P3.3 slice 2a: orchestrator, recommended option (user standing rule 2026-11-12): fixture-only fix in locked `crates/db/tests/repo_trigger_repository.rs` — trigger 2 renamed "Welcome" → "Welcome root" (the agent-written fixture broke `Trigger_workspaceId_name_key`; the TS test was mocked, so no original fixture exists). Assertions unchanged; 6/6 pass. Rule for agents: in stage 1, check that every failure is the `todo!` panic before the lock.
- Orchestrator-delegated, recommended option (P3.4a stage 2): add the `phonenumber` crate (Rust libphonenumber with Google metadata, +20 small crates) for the call-button E.164 validity check (`parsePhoneNumberFromString(...).isValid()` in the original). The business phone port can reuse it.
- P3.4a stage 2 finding (test bug, locked file): `crates/core/tests/flow_config_authoring_normalize_graph.rs` lines 30–32 expect `json!({"x": 100, …})` (integer) for node 0 but `json!({"x": 100.0 + w, …})` (float) for node 1. `serde_json` treats 508 and 508.0 as different values, so no implementation passes both. The implementation emits whole numbers as integers (JS has one number type). Fix needed in the test (orchestrator): compare `x`/`y` as `f64`, or write `(100.0 + w) as i64`.
- Orchestrator-delegated, recommended option (P3.4b stage 1): the registry-completeness test reads `import.meta.glob("../src/steps/*.ts")` in TypeScript. Rust has no module reflection, so `flow_config::exported_step_schemas()` (new, `step_schema_registry.rs`) lists the 103 exported step schemas, generated once from the TypeScript exports (Execute JavaScript dropped). The id-key scan reads the Rust `steps/` and `nodes/` modules (`("fooId", …)` / `"fooId":` keys); its key set is a subset of the TypeScript scan (the 3 extra TS keys are on the "not a reference" list).
- Orchestrator-delegated, recommended option (P3.4b stage 1): the 50 000-level nesting tests run on a 1 GiB-stack thread, because `serde_json::Value` clone, compare and drop recurse. The walkers keep the 512-level ceiling of the original.
- Orchestrator-delegated, recommended option (P3.4b stage 1): `ReadonlyMap`/`Record` inputs (`ReferenceIdMaps`, `FlowAuthoringContext` maps, `TemplateParameterValues`) are `BTreeMap`s. Iteration is sorted, not insertion order (affects only the order of `unknown` keys and of tied `candidates`).
