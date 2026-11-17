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
| 2 | Heavy queue: `processAIFile` (text extraction, chunkers, embeddings), AI handover bulk run (toggle engine, sweeper, chunk refund) | 2a done (text formats; binary formats left); parity ai-handover 34/34 |
| 3 | Events (analytics, error log, flow, message incl. `message:failed`), webhook, notification, schedule crons, leaderboard, questionnaires + Get User Data, marketing message, low | 3a done: webhook, trigger, events (error log, `message:failed`), notification, retention crons; rest left (see "Left") |
| 4 | `default-worker` tests (16) and the default / template handlers of Rust domains | 4a: blocked-workspace guard; the handlers wait for their domains (no Rust producer) |

## Test map (slice 1)

| TS test file | Cases | Rust | State |
|---|---|---|---|
| `ai-output-length.test.ts` | 6 (13 with `each`) | `ai/text.rs` tests (6) | done |
| `ai-generate-text-handler.test.ts` | 5 | `steps/ai_tests.rs` `ai_generate_text_handler` (5) | done |
| `ai-message-history-builders.test.ts` | 1 (2 with `each`) | `ai_message_history_builders` (1, the `aiGenerateText` path; the agent path shares `step_messages`) | done |
| `ai-generate-text-agent-openai-compatible.test.ts` | 4 | `ai_generate_text_agent` (3; the `sentByUserId` hand-over case is the variables context, `variables.rs`) | done |
| `ai-agent-runner-openai-compatible.test.ts` | 8 | `ai_generate_text_agent` (provider order, disabled row fallback) + `ai/llm.rs` tests; the v7 tool-error callback / tool-stats / first-chunk timeout cases are n-a (no streaming callbacks; non-streamed steps) | partial |
| `ai-delete-message-history.test.ts` | 7 | `ai_delete_message_history` (4: complete-history fallback + cache delete, no `lastMessageAt`, no message, DM reset); the storage-error / marker-failure mock cases are n-a (no typed storage errors) | partial |
| `ai-error-log-provider.test.ts` | 2 | `events_tests.rs` (1) | done |
| `agent-tool-stats.test.ts` | 4 | — | n-a (logging-only collector of the SDK callbacks) |

## Test map (slice 2)

| TS test file | Cases | Rust | State |
|---|---|---|---|
| `chunkers.test.ts` | 25 (with `each`) | `ai/chunk.rs` tests (20) | done |
| `text-extractor.test.ts` | 22 | `ai/extract.rs` tests (13: legacy text output, CSV tables, BOM, Windows-1252 reject, Markdown, budgets); PDF / XLSX cases wait for the binary parsers | partial |
| `process-ai-file.test.ts` | 25 | `ai_file.rs` tests (13, on a DB and a local file store); the lock / call-order mock cases are covered by the DB state | partial |
| `process-pending-embeddings.test.ts` | 4 | `ai_file.rs` tests (5); the invalid-key ErrorLog row goes through `events::log_provider_errors` | done |
| `mark-embedding-chunk-failed.test.ts` | 6 | `ai_file::handle_ai_agent` (last attempt marks the chunk `error`) | partial (no own test) |
| `ai-handover-bulk-toggle.test.ts` | 57 | `ai_handover.rs` tests (12: claiming, preconditions, integration unavailable, resuming, reconcile after the end) | partial: the batch / quota / token cases need the thread control (Q.D12 / Q.I1b) |
| `scan-ai-handover-bulk-runs.test.ts` | 15 | `ai_handover.rs` tests (4: exhausted runs, dispatch, refund of a queued chunk, enqueue failure, reconcile without cursor) | partial: no distributed lock (n-a) |

## Test map (slice 3)

| TS test file | Cases | Rust | State |
|---|---|---|---|
| `condition-evaluator.test.ts` | all | `condition_tests.rs` | done |
| `webhook-executor-payloads.test.ts`, `webhook-pipeline-payload.test.ts`, `webhook-payload-contact-inbox-leak.test.ts` | all | `webhook_tests.rs` (payload builders, leak check, tag scope, contact fallback) | done |
| `webhook-matcher-catalog.test.ts`, `webhook-matcher-datetime.test.ts` | all | `webhook_tests.rs` `catalog_matching`, `datetime_matching` | done ("logs and skips a webhook whose conditions cannot be evaluated" is n-a: no mock throw) |
| `record-provider-error-log.test.ts`, `write-error-log.test.ts` | all | `events_tests.rs` (recording emitter, DB writer, end-to-end `message:failed` on the SQLite bus) | done (stack cases n-a: no JS stacks) |
| `message-failed-error-stack.test.ts` | all | — | n-a (no JS stacks) |
| `push-notification-content.test.ts`, `send-push-expo.test.ts`, `expo-push-enabled-env.test.ts` | 10 + 9 + 5 | `notification_tests.rs` | done (token check and purged workspace: see Decisions) |
| `notify-new-order.test.ts` | all | — | todo (order domain has no Rust producer) |
| `trigger-action-executor-*.test.ts`, `trigger-action-scope*.test.ts`, `trigger-executor-*.test.ts`, `action-executor-contact-inbox-attribution.test.ts`, `trigger-worker-contact-inbox-propagation.test.ts` | all | `trigger_tests.rs` (DB outcomes: matching, history + stats, run limit, skip reasons, sequences, custom fields, conversation state, flow starts on the threaded inbox, Decision self-loop) | partial: Meta CAPI / Google Ads / Google Sheets / tag-sync / ads-conversion cases wait for their units; mock call-count cases are the memoized `Scope`; inbound-webhook source cases belong to the inbound webhook unit; audit-context cases n-a |
| `trigger-action-executor-send-google-ads-conversion.test.ts`, `trigger-action-executor-send-meta-capi-event.test.ts`, `trigger-n8n-handler.test.ts`, `evaluate-conversion-trigger.test.ts` | all | — | todo (integrations) |
| `datetime-trigger-evaluator.test.ts` | all | — | todo (datetime scanners) |
| `purge-*` cron handlers | — | `purge.rs` test (retention cut-off + each purge's SQL) | done |
| `default-worker.test.ts` | 16 | `default_queue.rs` `blocked_workspaces_skip_the_job` (the guard); the handler cases wait for their domains | partial |

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

- AI hand-over bulk run: no Rust thread control yet (Messenger hand-over
  protocol, Q.D12 / Q.I1b), so a run takes the original's
  "integration unavailable" path: the eligible threads after the cursor are
  counted skipped and the run fails `integrationUnavailable`, then the Page
  is reconciled. Claim, stop reasons, in-flight recovery, total count and
  the finish are the original.
- The sweeper reads the previous chunk's state from the `jobs` table (same
  SQLite file as the job store) instead of `queue.getJob().getState()`. No
  distributed lock: one process runs the schedule queue.
- The `schedule` queue worker registers only the ported crons
  (`schedule::PORTED`, today `scanAiHandoverBulkRuns`, every minute at
  second 36), so an unported cron never wakes the process.
- `processAIFile`: an in-process per-file lock replaces the Redis lock.
  Text formats are read as the original (plain text, CSV as a table, Markdown,
  RTF, XML, VTT, properties); HTML and the prose reading of Markdown use a
  reduced `html-to-text` / `remove-markdown`. PDF, DOCX, XLSX, PPTX, PPT,
  EPUB and e-mail need parser crates: until then such a file fails the run
  `fileUnreadable` (not read as garbage). Chunk lengths are UTF-16 units, as
  JavaScript, so chunk ids match.
- The heavy queue serves only `processAIFile`, the aiAgent queue only
  `processPendingEmbedding`; other actions fail unrecoverable (nothing in
  Rust enqueues them yet).

- Slice 3 (details in DECISIONS 2026-11-17 "Q.W2 slice 3"): outgoing
  webhooks re-check `assert_public_url` (DoH) on every hop, at most 5
  hops, no signing (the original has none); the blocked-workspace check
  reads the row; the event buses run on `SqliteEventStore` (notify, no
  polling); Expo push over HTTPS; the trigger run limiter is in-process;
  retention crons without `distributedLock`.

## State

Slice 1a (LLM runtime + 3 AI steps): 196 worker tests pass (Dell; 168 before); fmt, clippy -D warnings clean.
Slice 2a (AI hand-over bulk engine + sweeper, schedule worker, `processAIFile` for text formats, `processPendingEmbedding`): 263 worker + 7 server tests pass (Dell; 196 before); fmt, clippy -D warnings clean. Parity (Rust stack, e4d85bcc3): p1-03 ai-handover 34/34 (30/34 before); p1-02 ai-files 1/16: each failing case stops in its setup at `PUT /v1/integrations/ai/openai` ("Operation not ported yet", AI key connect P3.6-D2, not in `conversion` at 1da18811c), so the worker part is not reached; rerun after the P3.6-D2 merge.
Left in slice 2: PDF / DOCX / XLSX / PPTX / PPT / EPUB / e-mail extraction (parser crates), the other heavy actions (image edit / generate / segment, speech, analyze image, extract text, optimize stored image), the bulk thread-control batches (Q.D12 / Q.I1b).
Slice 3a + 4a (webhook queue, condition evaluator, trigger queue, events worker: error log + `message:failed`, notification queue, 8 retention crons, default-queue blocked guard): 305 worker + 7 server tests pass (Dell; 263 before); fmt, clippy -D warnings clean. Parity not run for this slice (no parity suite covers the outgoing webhook / trigger / push paths without the Trigger and Webhook CRUD, Q.D5 / P1.17).
Left in slice 1: `aiAnalyzeImage`, `aiEditImage` / `aiGenerateImage` / `aiSegmentImage` / text to speech (heavy queue), `aiExtractData`, the automated response (`processAutomatedResponse`, rich replies, system tools, global router), AI agent actions / handoff re-entry, comment AI reply / moderation, story automations, the `aiAgent` queue jobs (`processPendingEmbedding`, `summarizeConversation`, conversation sources).

## Left (next runs, brief order)

1. Slice 3 rest: the datetime scanners (`evaluateTriggers` / `evaluateDateTimeWebhooks` crons, `datetime-trigger-evaluator`) and the execution cleanups (`cleanupTriggers`, `cleanupWebhookExecutions`); the analytics listeners (broadcast / sequence / flow / comment-automation ops, MAC, active hourly, contact blocked, ads replied; analytics package); trigger integration actions (Meta CAPI, Google Ads, Google Sheets) and the tag-sync / ads-conversion fan-out; `notifyNewOrder` (order domain); leaderboard queue (leaderboard business domain, ~3.3k TS lines); questionnaires + Get User Data answers, `low` queue and marketing message (inbound-message / Messenger channel paths, Q.I*); the other schedule crons (broadcasts, marketing messages, token refresh, tenants, partitions, ...: their domains are not in Rust).
2. Slice 4 rest: the default / template handlers (bulk tag, appointment reminders, welcome message sync, pipeline stage removal, template worker) when their domains get a Rust producer (today the API runs them in the request or not at all).
3. Slice 1 rest: `aiAnalyzeImage`, `aiExtractData`, heavy image / speech steps, the automated response (`processAutomatedResponse`, system tools, global router), AI agent actions / handoff re-entry, comment AI reply / moderation, story automations, `summarizeConversation`, conversation sources.
4. Slice 2 rest: PDF / DOCX / XLSX / PPTX / PPT / EPUB / e-mail extraction (parser crates, Cargo.lock on the Mac), the other heavy actions, the bulk thread-control batches (with Q.D12 / Q.I1b). Parity p1-02 ai-files after the Q.D2c merge (`/v1/integrations/ai/{provider}`).
