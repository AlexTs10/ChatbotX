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
| 4 | Broadcasts (business + routes) | `broadcast/` (`mod` reads + transitions, `write` create / draft / schedule / clone / resend, `audience` count / preview / calendar, `recurrence` arm / disarm / ledger / preview, `plan_policy`), `template_params.rs` | `broadcasts.*` except `listContacts` and the 4 recurrence-history routes | `privatePreviewBroadcastRecurrenceAPI`, `privateListBroadcastOptionsAPI`, `privateListBroadcastTemplateDetailsAPI`; 11 actions (all but `setBroadcastRunSkippedAction`) | done |
| 4b | Broadcast recurrence history | `broadcast/history.rs` (skip / restore, run groups, occurrences, recipient stats) | `skipOccurrence`, `restoreOccurrence`, `listRunGroups`, `listOccurrences` | `privateListBroadcastRunGroupsAPI`, `privateListBroadcastOccurrencesAPI`, `setBroadcastRunSkippedAction` | done |
| 5a | Broadcast worker jobs: enqueue, prepare, send, reconcile, finalize, purge, draft purge | `broadcast/send.rs`, `broadcast/audience.rs` (`AudienceWalk`); `crates/worker/src/schedule` (`schedule` queue worker, broadcast schedulers) | worker part of `broadcast/service.ts` | — | done |
| 5b | Recurring broadcast jobs: `enqueueRecurringBroadcasts`, `materializeRecurringBroadcasts` (`materializeOccurrence`, `skipOccurrence`) | `broadcast/materialize.rs`; `crates/worker/src/schedule/recurring.rs`; `Enqueuer::backlog_count` | materialize part of `broadcast/recurrence.service.ts` | — | done |
| 6 | Sequence scheduler in the worker (producer / consumer / reconcile, step executor) | | | | done |
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
- Broadcast plan policy: `resolveForWorkspace` is always unrestricted (the
  owner plan identity is not ported; the community edition is unrestricted
  in the original too). The SQLite single writer is the activation lock.
- Broadcast draft activation reads the prepare job state from the `jobs`
  table of the same database; the purge job goes to the schedule queue.
- `broadcasts.listContacts`, `privateGetBatchBroadcastStatsAPI` and
  `privateListBroadcastContactsAPI` read the delivery analytics: Q.D11.
- Run-group and occurrence `stats` are `ContactOnBroadcast` counts in the
  main database (`history::batch_stats`, `stats_by_occurrence_bucket`).
- Audience preview avatar: resolved against the row's own contact inbox.
- An invalid path id on `/api/v1` answers 422 `invalidRequestData` (the TS
  tests check only that the schema rejects it).
- Broadcast worker jobs (slice 5a): the `schedule` queue worker starts with
  the broadcast jobs and registers only their schedulers; other schedule
  jobs fail unrecoverable until Q.W2. Cron locks are process-local mutexes
  (per database); the dispatch-window lease is the durable kv CAS. Hand-off
  and purge run one at a time. `isBlockedWorkspace` = the workspace row
  is gone or scheduled for deletion (cloud `ownerBlocked` waits for the
  user-quota port). The chat template send handlers are Q.I1.
- Recurring broadcast jobs (slice 5b): one item at a time (one SQLite
  writer); the backlog check is `Enqueuer::backlog_count`; plan capacity
  resolves unrestricted until the quota service is ported.
- Sequence scheduler (slice 6): `chatbotx_worker::sequence::start` runs the
  producer, the `sequenceScheduler` consumer and the reconcile loops on the
  SQLite schedule, each stopped by the shutdown coordinator; the
  `integration` worker handles `sendSequenceFlow`. One `runFlowNode` per
  contact inbox at a time (one SQLite writer).

## 4. State

Slices 1–3 done (`641f599f2`). Slice 4 done (`8b3cfe662`), 4b done (`e67dd7857`), 5a done (`59388ee17`), 5b done (`ff0bf948b`, gate: 2215 tests pass), 6 done (gate: 683 jobs/worker/server tests pass). Gate per commit: fmt, clippy `-D warnings`, `cargo test`
for `chatbotx-business`, `chatbotx-api`, `chatbotx-jobs`, `chatbotx-worker` (Dell).

Parity (Rust stack, Dell, 2026-11-17): `p1-04/11-broadcasts` and
`p1-08/04-email-topics` 27 of 29 pass — the 2 failures are `listContacts`
("Operation not ported yet", Q.D11). `p1-04/12-broadcasts-recurrence` 12 of
12 pass after 5b (the scheduler run is materialized by the worker).
