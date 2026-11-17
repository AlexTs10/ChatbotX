# Q.W3 — worker flow-step handlers

Branch `feat/conversion-steps`. Module `crates/worker/src/steps/` on the Q.W1
runner: `flow::run_step_handler` sends every step type that Q.W1 does not
handle to `steps::run`. One step per `sendFlow` job, as in Q.W1.

## Slices

| # | Slice | State |
|---|-------|-------|
| 1 | Basic-bot steps: `condition`; contact (tags, custom field set / clear, notes, email flags, delete, block, sequences, broadcast subscription); conversation (archive, assign, auto assign with the on-shift check, unassign, read / unread with the channel receipt, follow, bot on / off, typing, thread control); smart delays (`wait`, `followUp`, `splitTraffic`, the `resumeWait` / `resumeFollowUp` integration jobs). Worker DB fixtures (`steps/test_support.rs`) | in progress |
| 2a | `pipeline` step (`runPipelineFlowAction`, realtime `pipelineItemChanged`); tool steps `countCharacters`, `formatDate`, `generateCode`, `getDataFromJson` (`steps/tools.rs`) | done |
| 2b | Small steps: copy text, call phone number, find places, challenge, handover response, messenger user menu | todo |
| 3 | Steps of domains that are not in Rust: interface + test double, or skip (list below) | todo |
| 4 | The mock-call tests of `send-flow-step` (81), `send-message-handler` (52), `default-worker` (16) on the fixtures | todo |

## Test map

| TS test file | Cases | Rust (`crates/worker/src/steps/tests.rs` module) | State |
|---|---|---|---|
| `condition-handler.test.ts` | 10 | `condition_handler` (7: the 3 mock-call cases for snapshot refresh are n-a until Q.I1) | done |
| `contact-tag-steps.test.ts` | 13 | `contact_tag_steps` (4; event / analytics-scope cases n-a until Q.W2) | done |
| `set-custom-field-step.test.ts` | 9 | `custom_field_steps` (6; event-scope cases n-a until Q.W2) | done |
| `clear-custom-field-step.test.ts` | 2 | `custom_field_steps` | done |
| `auto-assign-step-handler.test.ts` | 6 | `auto_assign_step_handler` (7) | done |
| `mark-conversation-read-state-steps.test.ts` | 9 | `mark_conversation_read_state_steps` (7) | done |
| `thread-control-step.test.ts` | 9 | `thread_control_step` (1: every channel answers "not supported" until Q.D12; the 8 channel cases are n-a) | partial |
| `follow-up-step.test.ts` | 6 | `smart_delay_steps` | done |
| `resume-follow-up.test.ts` | 5 | `smart_delay_steps` | done |
| `resume-wait.test.ts` | 6 | `smart_delay_steps` | done |
| `smart-delay-resume-job.test.ts` | 9 | `smart_delay_steps` (the scanner cases wait for Q.W2) | partial |
| `start-external-step-handlers.test.ts` | 2 | `start_and_split_steps` | done |
| `send-flow-job.test.ts` | 3 | — | todo |
| `pipeline-step.test.ts` | 6 (19 with `each`) | `pipeline_step` (2: real rows; the action rules are `crates/business/tests/pipeline_flow_actions.rs`; the mock-call `each` cases and the "variables load once" case are n-a) | done |
| `count-characters-step.test.ts` | 5 | `tool_steps` (3) | done |
| `format-date-step.test.ts` | 9 | `tool_steps` (1 test, 5 cases) | done |
| `get-data-from-json-step.test.ts` | 5 | `tool_steps` (2) | done |

Mock-call assertions become assertions on DB rows and recorded jobs
(`test_support::Jobs`) or channel calls (`channel::RecordingChannel`).

## Decisions

- Unported and unknown step types stay an implicit success with a warning
  (Q.W1). `executeJavascript` stays dropped: a warning "not supported" and
  success.
- Contact, tag, custom-field and conversation events (trigger, analytics),
  SLA tracking and cache invalidation are not sent here; they come with the
  event workers (Q.W2). `ponytail:` comments mark each place.
- `threadControl` answers "not supported" on every channel and follows the
  error state until the channel units (Q.D12, Q.I1) give the channel calls.
- The on-shift check (`listOnShiftUserIds`) lives in the worker
  (`steps/working_hours.rs`) until the working-hours domain (Q.D9).
- The condition step uses the stored profile snapshot; the Instagram
  follower refresh waits for Q.I1.
- `scanSmartDelay` (rows due in more than 5 minutes) waits for Q.W2; only
  near deadlines are enqueued at once.
- `changeGamificationScore` (leaderboards, Q.D13) is not ported.
- `pipeline`: a variable-resolver failure keeps the text as written (the
  original rethrows); the write's flow-trigger events wait for Q.W2.
- Tool steps: `getDataFromJson` paths use `readJsonPath` (reads the
  dot-prop forms the builder emits); `generateCode` draws from the OS
  random source (`math_random`) instead of faker; an invalid `formatDate`
  input fails the job (date-fns-tz throws a `RangeError`).
- `callApi` (external request service), `evaluateDecision` (decision
  service), `questionnaires`, `getUserData`, coupons wait for their domain
  units and stay implicit successes.

## State

Slices 1 and 2a: 134 worker tests pass (Dell; 94 before Q.W3); fmt, clippy -D warnings clean.

## Steps that wait for other units

| Step(s) | Unit |
|---|---|
| AI steps (generate text, agent, analyze / edit image, extract data, text to speech, delete history) | Q.D6 |
| ads conversion, Meta conversions, custom audience, Google Ads, lead ads | Q.I4 / ads unit |
| email / CRM (ActiveCampaign, Drip, GetResponse, Klaviyo, Mailchimp, MailerLite, Moosend, SendGrid, send email, Make, n8n) | Q.I3 |
| spreadsheet steps | Q.I4 |
| appointment scheduling | Q.D9 |
| ecommerce (Shopify, WooCommerce, coupons, order payment) | Q.D8 |
| thread control channel calls | Q.D12 |
| `changeGamificationScore` | Q.D13 |
