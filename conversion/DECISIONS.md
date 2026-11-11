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
