# Q.W2 — worker rest

Branch `feat/conversion-w2` (from `conversion` at `1da18811c`). Crate
`crates/worker` (Q.W1 runtime, Q.W3 step registry). Order of the user
brief: (1) AI runtime + AI steps, (2) heavy queue (`processAIFile`, AI
handover bulk run), (3) events / webhook / notification / schedule /
leaderboard / questionnaires / marketing message / low, (4) the
`default-worker` tests and the default / template job handlers whose
domain is in Rust.

## Slices

| # | Slice | State |
|---|-------|-------|
| 1 | LLM runtime (`crates/worker/src/ai/`): provider wire formats (OpenAI chat for OpenAI / DeepSeek / OpenRouter / Muse / OpenAI-compatible, Anthropic messages, Gemini `generateContent`) with the tool-step loop, model factory (`normalizeAIModelId`, Muse transform, `nearai` reasoning strip), toolset (knowledge-file search, AI functions, MCP tools), prompt guards, channel length instruction, AI context summary + summarizer; steps `aiGenerateText`, `aiGenerateTextAgent`, `aiDeleteMessageHistory` | 1a done |
| 2 | Heavy queue: `processAIFile` (text extraction, chunkers, embeddings), AI handover bulk run (toggle engine, sweeper, chunk refund) | todo |
| 3 | Events (analytics, error log, flow, message incl. `message:failed`), webhook, notification, schedule crons, leaderboard, questionnaires + Get User Data, marketing message, low | todo |
| 4 | `default-worker` tests (16) and the default / template handlers of Rust domains | todo |

## Test map (slice 1)

| TS test file | Cases | Rust | State |
|---|---|---|---|
| `ai-output-length.test.ts` | 6 (13 with `each`) | `ai/text.rs` tests (6) | done |
| `ai-generate-text-handler.test.ts` | 5 | `steps/ai_tests.rs` `ai_generate_text_handler` (5) | done |
| `ai-message-history-builders.test.ts` | 1 (2 with `each`) | `ai_message_history_builders` (1, the `aiGenerateText` path; the agent path shares `step_messages`) | done |
| `ai-generate-text-agent-openai-compatible.test.ts` | 4 | `ai_generate_text_agent` (3; the `sentByUserId` hand-over case is the variables context, `variables.rs`) | done |
| `ai-agent-runner-openai-compatible.test.ts` | 8 | `ai_generate_text_agent` (provider order, disabled row fallback) + `ai/llm.rs` tests; the v7 tool-error callback / tool-stats / first-chunk timeout cases are n-a (no streaming callbacks; non-streamed steps) | partial |
| `ai-delete-message-history.test.ts` | 7 | `ai_delete_message_history` (4: complete-history fallback + cache delete, no `lastMessageAt`, no message, DM reset); the storage-error / marker-failure mock cases are n-a (no typed storage errors) | partial |
| `ai-error-log-provider.test.ts` | 2 | — | slice 3 (ErrorLog) |
| `agent-tool-stats.test.ts` | 4 | — | n-a (logging-only collector of the SDK callbacks) |

## Decisions

- Model calls are one non-streamed HTTP request per step (`ai::llm`). The
  original streams but never sends a partial answer in these steps
  (`sendParts: false`); the final text is the same (`textStream` = step
  texts joined; the agent joins steps with a paragraph break as
  `toStepSeparatedTextStream`).
- OpenAI uses the chat completions API (the SDK's default is Responses);
  reasoning models (`o*`, `gpt-5*`) get `max_completion_tokens` and no
  temperature, as `@ai-sdk/openai` does. Anthropic gets `max_tokens` 4096
  when the step sets none.
- The transport is the `AiHttp` trait (`Deps.ai`), so the tests script the
  provider answers; the embedder is the business `HttpEmbedder`.
- The AI context (`aiContextStore`, Redis hash, 24 h) is a `DurableKv`
  JSON value under the same key `ai:ctx:v1:<conversationId>` with the
  marker and the summary. `appendHistory` and the summarize job are not
  ported: the summary is rebuilt when the AI marker changes or the 24 h TTL
  ends.
- No per-conversation lock around the history marker reset (the SQLite
  writer serializes the writes).
- System tools (`sys:` handoff, document / image / URL readers, web
  search) come with the automated response (they need its context
  getter); the flow steps pass none, as the original.
- `logStepProviderError` (ErrorLog rows) waits for the error-log worker
  (slice 3).

## State

Slice 1a (LLM runtime + 3 AI steps): 196 worker tests pass (Dell; 168 before); fmt, clippy -D warnings clean.
Left in slice 1: `aiAnalyzeImage`, `aiEditImage` / `aiGenerateImage` / `aiSegmentImage` / text to speech (heavy queue), `aiExtractData`, the automated response (`processAutomatedResponse`, rich replies, system tools, global router), AI agent actions / handoff re-entry, comment AI reply / moderation, story automations, the `aiAgent` queue jobs (`processPendingEmbedding`, `summarizeConversation`, conversation sources).
