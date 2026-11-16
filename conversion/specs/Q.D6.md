# Q.D6 — AI domains (agents, files/RAG, functions, handover, MCP servers, providers)

Branch `feat/conversion-ai` (from `conversion` at `6c3f6fc37`). Sources:
`packages/business/src/ai-*`, `packages/ai`, `apps/builder/src/features/ai-*`
and `integration-*` (AI providers). The worker side (`apps/worker/src/ai-agent`)
is unit Q.W2: this unit only enqueues jobs with the original payload.

## 1. Slices

| # | Slice | Business | Public routes | /rpc (builder) | State |
|---|-------|----------|---------------|----------------|-------|
| 1 | AI functions + MCP servers | `ai_function.rs`, `ai_mcp_server.rs`, `ai/mcp_client.rs` | `aiFunctions.*`, `aiMcpServers.*` | list procedures, CRUD actions, `validateAIMcpServer` | done |
| 2 | AI agents | `ai_agent.rs`, `ai/models.rs` (model catalog) | `aiAgents.*` | `aiAgentsAPI.listAIAgentsAPI`, create/update/delete actions | done |
| 3 | AI provider integrations | `integration_ai_provider.rs` (key probe, find, autoReply update), `integration_openai_compatible.rs` | `integrations.getAiProvider`, `updateAiProvider` | `update<Provider>Action` (6) | done (connect / disconnect: P3.6-D2) |
| 4 | AI files / RAG | `ai_file.rs` (DB vector scan: `ai_file_embedding::vector_search`) | `aiFiles.*` except `search` | `aiFilesAPI.*`, create / delete / reindex actions | done |
| 5 | AI handover settings + bulk run | `ai_handover.rs` | `aiHandover.*` | `aiHandoverAPIs.*`, save / set / retry actions | done (engine / sweeper: Q.W2) |
| 6 | `packages/ai` provider layer (server part) | `ai/knowledge.rs` (embedding model, query embedding, `searchKnowledgeChunks`, `performFileSearch`, `searchAIFiles`, `isInvalidApiKeyError`), `ai/models.rs` catalog tests | `aiFiles.search` | `searchAIFilesAction` | done (LLM runtime: Q.W2 / Q.D12) |

## 2. Test map

Per test case: `specs/Q.D6-test-map.tsv` (file, line, name, Rust test, status, note).

## 3. Decisions (also in `DECISIONS.md`)

- The AI `/rpc` procedures and actions live in `crates/api/src/rpc_ai.rs`
  (`call(state, path, context, input)`), not wired yet: the `/rpc` procedure
  table is on the unmerged `feat/conversion-p35f`. The wiring is one match arm
  when p35f merges.
- Action results: `create*` / `delete*` return `null`, `update*` the row
  (RPC `{ json, meta }` with Date meta). Validation failures are
  `400 BAD_REQUEST` with `data.validationErrors` (next-safe-action formatted
  errors); `nameAlreadyExists` becomes `<feature> name already exists` at `name`.
- No read caches (`withCache`), no template locks / delete guards and no
  `assertReferencable` for agent tool tokens (P3.6-D1 rule). The AI function
  `triggerFlowId` keeps the workspace flow check.
- MCP client: a small Streamable-HTTP JSON-RPC client on reqwest
  (`initialize`, `tools/list` with cursor paging, `tools/call`; JSON or SSE
  answers; legacy fallback when `initialize` is unknown). `validate` uses the
  `@ai-sdk/mcp` client name and protocol `2025-11-25`; the worker client uses
  `AhaChat-Worker` / `2024-11-05`.
- Bot-field token templates resolve without a cache.
- `triggerFlowId` (`zodBigintAsString`) uses the 18-digit id check.
- `maxOutputTokens` must be an integer (422): the STRICT SQLite column
  refuses a fraction; the original's Postgres integer column fails it later
  with a 500.
- Public `PATCH /v1/ai-agents/{id}` accepts `actionPrompt` / `actionRules`
  (the strict schema is intersected with `{ id }`) and drops them: the public
  API never writes the private AI action fields.
- `aiAgentsAPI.key` is the TanStack query-key helper (frontend only), not a
  procedure.
- AI key providers (claude, deepseek, gemini, muse, openai, openrouter):
  connect and disconnect go through the connection engine
  (`connectFromCredentials`, `connectionService.disconnect`), which is P3.6-D2
  work. This unit ports the AI part: the key probe (`verify_request` +
  `classify`, reqwest), `getAiProvider` / `updateAiProvider` (autoReply) and the
  six `update<Provider>Action`s. `integration_openai_compatible.rs` has the
  base-URL rules (cloud SSRF check), the models probe and the service
  (connect / update / disconnect) for the engine to call.
- The provider update actions return `null` (the original returns the stored
  row, which carries the encrypted `auth`); an empty patch writes nothing (the
  original's drizzle `.set({})` throws); the OpenAI action ignores its bound
  integration id (one row per workspace).

- AI handover (slice 5): no settings cache (P3.6-D1). The distributed lock and
  the `FOR UPDATE` row locks become one SQLite write transaction: the TS tests
  for races "between the two locks" / "under the lock" are n-a. The bulk-run
  engine, sweeper, claim / page helpers and `refundIfPreviousChunkQueued` are
  worker code (Q.W2). The API enqueues `aiHandoverBulkToggle` on the
  integration queue with job id `ai-handover-bulk-{runId}-chunk-{seq}` and
  payload `{ type, data: { runId, workspaceId } }` (ids as strings, as the
  original's bigint-as-string), attempts 1, `removeOnComplete`,
  `removeOnFail: 100`.
- An ON for a Page with no settings row is refused when the automation does
  not run (`aiHandoverBulkAutomationNotActive`); the TS test passes only because
  `isActiveNow` is mocked. A confirmed change that cannot start its run rolls
  back and then stops the live run in its own transaction, as the original.
- An unknown workspace time zone is read as UTC (the original's
  `formatInTimeZone` throws a `RangeError`, so the request fails with a 500).
- Builder actions: `RpcAiContext.is_super_admin` / `is_support_session` default
  to `false`; the p35f wiring sets them (`with_member`). Refusal and domain-error
  copy is the `en` message (no i18n on the server). `inboxService.disconnect`
  is not in Rust yet; `ai_handover::cancel_live_in` is its hook.

- AI files (slice 4): no template locks / delete guards (P3.6-D1). The
  public `aiFiles.create` takes JSON (`url`) or multipart (`file`, `url`,
  `name`): `Call` has no headers, so a body that is not JSON and starts with a
  `--boundary` line is read as multipart (a small parser, no new crate). The
  API enqueues `processAIFile` on the heavy queue (`{ type, data: { aiFileId,
  runId } }`, job id `heavy-ai-file-<id>` / `heavy-ai-file-<id>-reindex-<run>`,
  `getHeavyJobOptions`). No job queue (`ApiState.jobs = None`) counts as a
  failed enqueue (`enqueueFailed`). The original's no-op `DELETE "AIEmbedding"
  WHERE "id" = <file id>` is not ported (chunks cascade). Search / reindex
  share the kv rate-limit buckets of the original (5 per 10 s); without a kv
  store there is no limit. Action copy is the `en` message.
  `createFromCopiedObject` (template install) is not ported (no templates).

- `packages/ai` (slice 6): this unit ports the parts with a server caller in
  the AI domains: the embedding model (`resolveEmbeddingModel`: OpenAI
  `text-embedding-ada-002`, then Gemini `gemini-embedding-001` at 1536
  dimensions), the query embedding (one reqwest POST per provider behind the
  `Embedder` trait, `ApiState.embedder`; the `ai` SDK retries, 2 after 2 s /
  4 s, without its `Retry-After` reading), `searchKnowledgeChunks`,
  `performFileSearch`, `searchAIFiles`, `isInvalidApiKeyError`, the model
  catalog (slice 2) and the MCP client (slice 1). The LLM runtime
  (`factory`, `stream`, tools / toolset, prompts, `ai-context-service`,
  `muse-transcribe`, `sam`) is ported with its caller, the worker AI agent
  (Q.W2); the summarizer with the WhatsApp call summaries (Q.D12). The
  picker catalogs (`openaiCompatiblePresetConfigs`, knowledge extensions)
  stay TypeScript: the web frontend imports `packages/ai`.
  `aiIntegrationService.invalidateCache` is a cache (P3.6-D1: none).
- A resolve or provider failure inside a search is a plain error (500), as
  the original's unhandled `EmbeddingProviderNotConfiguredError` /
  `APICallError`; `noEmbeddingProvider` (400) is checked first.

## 4. State

- Slices 1–6 committed (see TRACKER). Parity p1-02 / p1-03: see TRACKER.
