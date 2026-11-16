# Q.W1 — worker core

Branch `feat/conversion-worker`. New crate `crates/worker` (`chatbotx-worker`),
started by `chatbotx-server` (`chatbotx_worker::start`).

## Slices

| # | Slice | State |
|---|-------|-------|
| 1 | Channel send interface (`channel::Channel`, `Channels`, test double `RecordingChannel`), webchat + API channel senders, `chat` queue (send / edit / delete / like / hide / typing / flow step / chat message), `integration` queue `sendFlow` runner, server wiring (shared `JobStore`, `Shutdown`, `*_WORKER_CONCURRENCY`) | done (be107bde8) |
| 2 | `default` queue: Meta catalog import / submit / check (`meta_catalog/`: Graph client, item mapping, run / import state, `importPage`, the error sanitizer and safe log) | done |
| 3 | `prepareOutgoing` / `resolveInboundProducts` (18 tests, `packages/business/__tests__/product-message-service.test.ts`) and the `sendProducts` / `sendCollection` flow steps | todo |

## Channel send interface (for Q.I1–Q.I4)

`chatbotx_worker::channel::Channel` (one object per channel name, registered
with `Channels::with(name, Arc<dyn Channel>)`). Each method has a default that
fails with "Channel handler {group}.{name} is not available for {channel}"
(the SDK's missing handler):

| SDK handler | Rust method | Input | Output |
|---|---|---|---|
| `message.sendMessage` | `send_message` | `SendMessage { contact, message, quick_replies, metadata, send_from }` | `SendResult { message_ids, sent_count }` |
| `message.sendFlowStep` | `send_flow_step` | `SendFlowStep { contact, flow_id, flow_version_id, step, product_message, quick_replies, metadata, rich_response, send_from, comment_anchor, message }` | `SendResult` |
| `comment.sendComment` / `sendPrivateReply` | `send_comment` / `send_private_reply` | `SendMessage` | `SendResult` |
| `comment.deleteComment` / `editComment` / `likeComment` / `hideComment` | same names | ids + values | `()` |
| `conversation.sendTyping` | `send_typing` | `SendTyping` | `()` |
| `deliveryReceipts` | `delivery_receipts()` | — | `bool` |

`ChannelCtx` gives the channel its integration row (`integration_id`, `auth`
JSON), the workspace, the `Db` and the realtime handle. Errors are
`SendFailure::Channel(ChannelError { message, category, code, http_status_code })`
(`channel::Category`: retryable, permanent or unknown, as the SDK `ChannelErrorCategory`) or `SendFailure::Other(text)`.
`Channels::resolve` finds the row in the channel's `Integration*` table by
`inboxId`.

## Test map

| TS test file | Cases | Rust | State |
|---|---|---|---|
| `apps/worker/__tests__/will-send-retry.test.ts` | 4 | `channel.rs` tests | done |
| `apps/worker/__tests__/stored-step-content.test.ts` | 5 | `send_flow_step.rs` tests | done |
| `apps/worker/__tests__/flow-execution-key.test.ts` | 3 | `integration.rs` tests | done |
| `apps/worker/__tests__/integration-channel-origin.test.ts` | 7 | `integration.rs` tests (6; the webhook-context case is n-a: no async-local context in Rust) | done |
| `apps/worker/__tests__/integration-routing.test.ts` | 13 | `integration.rs` tests | done |
| `apps/worker/__tests__/integration-job-context.test.ts` | 6 | — | n-a until the orphaned-integration cleanup (Q.I*) |
| `apps/worker/__tests__/send-flow-step.test.ts` | 81 | — | todo (needs the DB VariableSources, media copy, product steps) |
| `apps/worker/__tests__/send-message-handler.test.ts` | 52 | — | todo |
| `apps/worker/__tests__/default-worker.test.ts` | 16 | — | todo (slice 2) |
| `apps/worker/src/default/handlers/meta-catalog/__tests__/safe-error-log.test.ts` | 2 | `meta_catalog/mod.rs` tests | done (the `err.stack` checks become "no secret in the log details": no stack in Rust) |
| `integrations/meta-catalog/__tests__/remote-product.test.ts` | 11 (19 with `each`) | `meta_catalog/items.rs` tests | done |
| `integrations/meta-catalog/__tests__/mapper.test.ts` | 10 (31 with `each`) | `meta_catalog/items.rs` tests | done |
| `integrations/meta-catalog/__tests__/retailer-id.test.ts` | 8 (10 with `each`) | `meta_catalog/items.rs` tests | done |
| `integrations/meta-catalog/__tests__/items-batch.test.ts` | 6 | `meta_catalog/sync.rs` (2 own `parseBatchResults` tests) | partial: HTTP-mock cases not ported |
| `apps/worker/__tests__/meta-catalog-{import,sync}.test.ts` | 32 | — | todo (service-mock tests; covered by parity p1-14 `22`) |
| `packages/business/__tests__/product-message-service.test.ts` | 18 | — | todo (slice 3) |

## Decisions

- New crate `chatbotx-worker` holds the handlers and the `Channel` trait; the
  channel units depend on it.
- Webchat and API channel outgoing senders are in this crate as reference
  implementations (parity p1-11 flow send and API callback). Q.I2 keeps the
  inbound side and may move them.
- The handlers re-read the conversation and contact inbox by id (D3 job
  payloads carry reduced objects).
- An unported `chat` / `integration` / `default` action fails unrecoverably
  ("… action is not defined: X"), so a job is kept and visible, not lost.
- An unported flow step handler counts as an implicit success (as a missing
  TS handler) with a warning log.
- The flow execution key is `<jobId>:<row id>` (the row id replaces BullMQ's
  `job.timestamp`; it is fixed across retries and differs when a job id is reused).
- `flowNodeExecuted` activity is recorded only for a numeric node id (the
  `referenceId` column is an integer).
- Not ported yet (marked `ponytail:` in code): blocked-workspace guard, bot
  quota gate, connection send gate, thread-control gate, `message:failed` and
  analytics events (Q.W2), comment moderation log, the media copy into storage,
  `{{variable}}` resolution (spintax only), quick-reply arm / clear,
  `statusBranches`, broadcast `resetContactForResume`, getUserData branch deferral.
- Meta catalog: the worker has its own Graph client (`x-business-use-case-usage`,
  GET retries 3 times with a backoff up to 1 s). `toPublicErrorMessage` /
  `sanitizePublicText` are ported in `meta_catalog/mod.rs`; the key / value
  quote back-references are spelled out as alternatives (`regex` has none).
  The `catalog_synced` audit is not sent: a worker job has no user, so the
  original drops it too. `logProviderError` is not ported (ponytail).
- Workers share the API's `JobStore` (in-process wake-ups). `Shutdown` drains
  the workers on SIGINT / SIGTERM (`WORKER_SHUTDOWN_TIMEOUT_MS`), then the
  HTTP server and the realtime hub stop.

## State

Slice 1 committed (be107bde8): fmt, clippy -D warnings, 30 tests (25 worker + 5 server) pass.
Slice 2: 73 worker tests pass; parity p1-14 `22-meta-catalog` 6/6.

Parity (`run-rust.sh`, Dell, after slice 1):

| Suite | Pass | Fail | Cause of the failures |
|---|---|---|---|
| p1-06 08-messages | 7 | 0 | (sendFlow case now green) |
| p1-11 40-messages-core | 13 | 1 | media library upload `mediaFileId` (media unit; 501 notPorted). Worker flow send and API-channel callback now green |
| p1-14 22-meta-catalog | 6 | 0 | (after slice 2) |
| p1-08 31-mm-campaigns | 0 | 22 | no Rust seed module `stacks/rust/ext/p1-08.ts` and the marketing-message routes / worker (not Q.W1) |
