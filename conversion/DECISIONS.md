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
