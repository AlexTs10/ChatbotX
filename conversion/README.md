# ChatbotX → Lean conversion

This folder is the record of the conversion. The orchestrator prompt is the same in each session.
The session reads `TRACKER.md`, does one unit, updates the tracker, commits, and stops.

- Fork: https://github.com/AlexTs10/ChatbotX (branch `conversion`)
- Upstream: https://github.com/ChatbotXIO/ChatbotX
- Baseline upstream commit: `40e733cee` (main, 2026-11-11)

| File | Content |
|---|---|
| TRACKER.md | Ordered units, status, proof, commit |
| FEATURES.md | Each feature: keep or drop, with the user's decision |
| TEST_MAP.md | Each original test file → class A/B/C → new location → status |
| DECISIONS.md | Dated log of decisions and questions to the user |
| MEMORY.md | RSS measurements after each phase |
| frozen-tests/ | Read-only copy of the original tests + MANIFEST.sha256 (from P0.2) |
| parity/ | Class A black-box harness (frozen after Phase 1) |
| specs/ | Analyst specs, one file for each unit |
