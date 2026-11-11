# Tracker

Status values: `todo`, `in_progress`, `blocked` (reason required), `done`, `dropped`.
Only one unit can be `in_progress` at a time. Units for Phase 1 and later are added when Phase 0 is done.

| ID | Phase | Unit | Status | Depends on | Tests (A/B/C) | Proof | Commit |
|----|-------|------|--------|------------|---------------|-------|--------|
| P0.1 | 0 | Fork, clone, `conversion` branch, empty tracker files | done | - | - | See "Proof P0.1" | commit `conversion(P0.1)` in git log |
| P0.2 | 0 | Copy all tests to `frozen-tests/` + `MANIFEST.sha256` | done | P0.1 | - | See "Proof P0.2" | commit `conversion(P0.2)` |
| P0.3 | 0 | Run original suite (`pnpm install && pnpm test`), record baseline, mark `baseline-fail` | todo | P0.2 | - | | |
| P0.4 | 0 | Feature inventory in `FEATURES.md` (LOC + test count); keep/drop already decided (see DECISIONS 2026-11-11: drop Execute JavaScript only) | todo | P0.1 | - | | |
| P0.5 | 0 | Classify every test file A/B/C in `TEST_MAP.md`; mark dropped-feature tests `dropped` | todo | P0.2, P0.3, P0.4 | - | | |
| P0.6 | 0 | Measure RSS of original stack (`docker stats --no-stream`) into `MEMORY.md` | todo | P0.1 | - | | |

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
