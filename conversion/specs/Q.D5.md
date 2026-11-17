# Q.D5 — Broadcasts, sequences, contact sequences, smart delay, triggers, email topics

Branch `feat/conversion-d5` (from `conversion` at `0f2f2ad56`). Sources:
`packages/business/src/{broadcast,sequence,contact-sequence,smart-delay,trigger,email-topic}`,
`apps/builder/src/features/{broadcasts,sequences,triggers,email-topics}`,
`apps/worker/src/{schedule,sequence-scheduler,trigger}`. Worker jobs run on the
Q.W1 runtime (`crates/worker`, `specs/Q.W1.md`).

## 1. Slices

| # | Slice | Business | Public routes | /rpc (builder) | State |
|---|-------|----------|---------------|----------------|-------|
| 1 | Email topics | `email_topic.rs` (CRUD, analytics detach on delete) | `emailTopics.*` | `emailTopicsAPI.private*` (4), create / update / delete actions | done |
| 2 | Sequences + steps + folders, contact sequences | `sequence.rs` (CRUD, steps, contact-schedule recalculation), `contact_sequence.rs` (enroll writes the schedule row) | `sequences.*` except `listStepContacts` | `sequencesAPI.listSequencesWorkspaceAuthAPI`, 5 sequence actions, `updateContactSequenceAction` | done |
| 3 | Triggers | `trigger.rs` (CRUD, 50 quota, conditions, settings), `automation_action.rs` (action reference check) | `triggers.*` (zod port schemas in `handlers/triggers.rs`) | create / delete / update / updateSettings actions | done |
| 4 | Broadcasts (business + routes) | | `broadcasts.*` | `broadcastAPIs.*`, broadcast actions | todo |
| 5 | Broadcast worker jobs | | | | todo |
| 6 | Sequence scheduler in the worker (producer / consumer / reconcile, step executor) | | | | todo |
| 7 | Smart delay | | | | todo |
| 8 | Trigger worker | | | | todo |
| 9 | Automated responses actions | | (`/v1/keywords` is D3) | create / update / delete / enable actions | todo |

## 2. Test map

Per test case: `specs/Q.D5-test-map.tsv` (file, line, name, Rust test, status, note).

## 3. Decisions (also in `DECISIONS.md`)

- Sequence schedule: the API writes the SQLite `sequence_schedule` row in the
  same write transaction as the `SequenceDispatch` row
  (`chatbotx_jobs::sequence::schedule_in`) and wakes the producer after the
  commit (`wake_producer`). A cancelled dispatch keeps its schedule row; the
  reconcile cleanup removes it (the producer skips non-pending dispatches).
- Email topic tracking (open / click routes and the tracking service) needs
  the analytics module: deferred to Q.I3 / Q.D11. `sequences.listStepContacts`
  and the private step stats / contacts procedures: deferred to Q.D11.
- No template locks / delete guards (P3.6-D1 rule): the template-lock test
  cases of `sequence-service.test.ts` are `n-a`.
- Service audits: `sequence::update` returns the audit detail (`enabled` /
  `disabled` when only `active` changed, else `updated`; `None` when nothing
  changed); the caller writes it with `record_audit`.
- `createSequenceAction` / `updateSequenceAction`: a taken name answers
  `validationErrors.name = ["Sequence name already exists"]` (the action's
  `returnValidationErrors`).
- Triggers: no explicit condition-cache refresh after a write; the RAM cache
  TTL is 5 s (`events::cache`). The action / condition schemas use the core
  zod port. The reference check runs only for new actions (TS
  `indexesOfNewActions`). `createTriggerAction` maps the quota error to the
  English text "Maximum 50 triggers allowed per workspace".
- `trigger-service-executions.test.ts` (run limiter, stats) moves to slice 8.
- An invalid path id on `/api/v1` answers 422 `invalidRequestData` (the TS
  tests check only that the schema rejects it).

## 4. State

Slices 1–3 done. Gate per commit: fmt, clippy `-D warnings`, `cargo test`
for `chatbotx-business`, `chatbotx-api`, `chatbotx-jobs` (Dell).
