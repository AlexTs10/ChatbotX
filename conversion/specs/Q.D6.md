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
| 4 | AI files / RAG | — | `aiFiles.*` | — | todo |
| 5 | AI handover settings + bulk run | `ai_handover.rs` | `aiHandover.*` | `aiHandoverAPIs.*`, save / set / retry actions | in progress |
| 6 | `packages/ai` provider layer | — | — | — | todo |

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

## 4. State

- Slices 1–3 committed (see TRACKER). Slice 5 in progress; 4 and 6 todo.
