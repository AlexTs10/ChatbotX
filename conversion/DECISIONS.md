# Decisions

Dated log of each decision and each question to the user. Newest entry at the bottom.

## 2026-11-11 — P0.1 setup

- The user has READ access only to `ChatbotXIO/ChatbotX`. So the work is on a fork: `AlexTs10/ChatbotX` (`origin`). `upstream` points to `ChatbotXIO/ChatbotX`.
- Clone location: `~/Desktop/ChatbotX-lean`. The old clone at `~/Desktop/ChatbotX` (tracks upstream `main`) is not changed.
- Baseline: upstream `main` at `40e733cee` (2026-11-11).
- The orchestrator prompt says "about 2,779 test files". On the baseline commit, `git ls-files | grep -E '\.(test|spec)\.(ts|tsx|js|jsx|mts|cts)$'` gives **3,100** files. P0.2 must freeze the real set (and also test helpers/fixtures that tests import), not the old number.
- The manifest check (`shasum -a 256 -c MANIFEST.sha256`) is not possible in P0.1, because `frozen-tests/` is empty until P0.2. From P0.2, it runs before each commit.
- Upstream `main` moved to `f371c5c5a` (1 commit) after the clone. The baseline stays `40e733cee`. Do not merge upstream into `conversion` without a new decision.
