-- P2.6 jobs (conversion/specs/P2.6.md 2.2, 3.2, 4.2): infrastructure tables
-- of `chatbotx-jobs` (no Drizzle origin). Times are unix ms; JSON is TEXT
-- checked with json_valid.

-- One row per job (BullMQ job hash). `job_id` is the custom job id or the
-- auto id; it is unique per queue until retention deletes the row (S2).
CREATE TABLE "jobs" (
  "id"               INTEGER PRIMARY KEY NOT NULL,
  "queue"            TEXT    NOT NULL,
  "name"             TEXT    NOT NULL,
  "job_id"           TEXT    NOT NULL,
  "data"             TEXT    NOT NULL CHECK (json_valid("data")),
  "opts"             TEXT    NOT NULL CHECK (json_valid("opts")),
  "state"            TEXT    NOT NULL CHECK ("state" IN ('waiting', 'delayed', 'active', 'completed', 'failed')),
  "priority"         INTEGER NOT NULL DEFAULT 0 CHECK ("priority" BETWEEN 0 AND 2097152),
  "ready_seq"        INTEGER NOT NULL,
  "run_at_ms"        INTEGER NOT NULL,
  "created_at_ms"    INTEGER NOT NULL,
  "attempts"         INTEGER,
  "attempts_made"    INTEGER NOT NULL DEFAULT 0,
  "attempts_started" INTEGER NOT NULL DEFAULT 0,
  "stalled_count"    INTEGER NOT NULL DEFAULT 0,
  "backoff_kind"     TEXT    CHECK ("backoff_kind" IN ('fixed', 'exponential')),
  "backoff_ms"       INTEGER NOT NULL DEFAULT 0,
  "dedup_id"         TEXT,
  "scheduler_id"     TEXT,
  "lease_owner"      TEXT,
  "lease_until_ms"   INTEGER,
  "processed_at_ms"  INTEGER,
  "finished_at_ms"   INTEGER,
  -- Per-job retention (DECISIONS P2.6), set when the job finishes.
  "keep_count"       INTEGER,
  "keep_until_ms"    INTEGER,
  "return_value"     TEXT    CHECK ("return_value" IS NULL OR json_valid("return_value")),
  "failed_reason"    TEXT,
  "stacktrace"       TEXT    CHECK ("stacktrace" IS NULL OR json_valid("stacktrace")),
  CONSTRAINT "jobs_queue_job_id_key" UNIQUE ("queue", "job_id")
) STRICT;
-- Claim: next waiting job per queue in BullMQ order.
CREATE INDEX "jobs_claim" ON "jobs" ("queue", "priority", "ready_seq") WHERE "state" = 'waiting';
-- Delayed promotion and the timer.
CREATE INDEX "jobs_delayed" ON "jobs" ("run_at_ms") WHERE "state" = 'delayed';
-- Stalled recovery and lease renewal.
CREATE INDEX "jobs_active" ON "jobs" ("lease_owner", "lease_until_ms") WHERE "state" = 'active';
-- Retention trims and counts.
CREATE INDEX "jobs_done" ON "jobs" ("queue", "state", "finished_at_ms") WHERE "state" IN ('completed', 'failed');
-- At most one pending job per scheduler.
CREATE INDEX "jobs_scheduler" ON "jobs" ("queue", "scheduler_id") WHERE "scheduler_id" IS NOT NULL;

-- BullMQ `de:<id>`: the job that owns a deduplication id.
CREATE TABLE "job_dedup" (
  "queue"         TEXT    NOT NULL,
  "dedup_id"      TEXT    NOT NULL,
  "job_id"        TEXT    NOT NULL,
  "expires_at_ms" INTEGER,
  PRIMARY KEY ("queue", "dedup_id")
) STRICT, WITHOUT ROWID;

-- BullMQ `dn:<id>` (keepLastIfActive): the latest add while the owner runs.
CREATE TABLE "job_dedup_next" (
  "queue"    TEXT NOT NULL,
  "dedup_id" TEXT NOT NULL,
  "name"     TEXT NOT NULL,
  "data"     TEXT NOT NULL CHECK (json_valid("data")),
  "opts"     TEXT NOT NULL CHECK (json_valid("opts")),
  "job_id"   TEXT NOT NULL,
  PRIMARY KEY ("queue", "dedup_id")
) STRICT, WITHOUT ROWID;

-- `upsertJobScheduler`: one row per scheduler (cron pattern or `every`).
CREATE TABLE "job_schedulers" (
  "queue"          TEXT    NOT NULL,
  "scheduler_id"   TEXT    NOT NULL,
  "pattern"        TEXT,
  "every_ms"       INTEGER,
  "template_name"  TEXT    NOT NULL,
  "template_data"  TEXT    NOT NULL CHECK (json_valid("template_data")),
  "next_run_at_ms" INTEGER NOT NULL,
  "iteration"      INTEGER NOT NULL DEFAULT 0,
  CHECK (("pattern" IS NULL) <> ("every_ms" IS NULL)),
  PRIMARY KEY ("queue", "scheduler_id")
) STRICT, WITHOUT ROWID;

-- Counters: `ready_seq` (FIFO key, grows), `head_seq` (stalled requeue at
-- the head, shrinks), `job_id` (auto job ids).
CREATE TABLE "job_meta" (
  "k" TEXT PRIMARY KEY NOT NULL,
  "v" INTEGER NOT NULL
) STRICT, WITHOUT ROWID;
INSERT INTO "job_meta" ("k", "v") VALUES ('ready_seq', 0), ('head_seq', 0), ('job_id', 0);

-- The sequence schedule and retry sets (Redis ZSETs `seq:dispatch:{b}:*`).
CREATE TABLE "sequence_schedule" (
  "kind"        TEXT    NOT NULL CHECK ("kind" IN ('schedule', 'retry')),
  "dispatch_id" INTEGER NOT NULL,
  "bucket"      INTEGER NOT NULL CHECK ("bucket" BETWEEN 0 AND 255),
  "at_ms"       INTEGER NOT NULL,
  PRIMARY KEY ("kind", "dispatch_id")
) STRICT, WITHOUT ROWID;
CREATE INDEX "sequence_schedule_due" ON "sequence_schedule" ("kind", "bucket", "at_ms");
-- The producer timer: the next due entry of any bucket.
CREATE INDEX "sequence_schedule_at" ON "sequence_schedule" ("at_ms");

-- Event bus streams (Redis Streams with one consumer group each).
CREATE TABLE "event_group" (
  "stream"     TEXT NOT NULL,
  "group_name" TEXT NOT NULL,
  PRIMARY KEY ("stream", "group_name")
) STRICT, WITHOUT ROWID;

CREATE TABLE "event_entry" (
  "id"              INTEGER PRIMARY KEY NOT NULL,
  "stream"          TEXT    NOT NULL,
  "type"            TEXT    NOT NULL,
  "payload"         TEXT    NOT NULL CHECK (json_valid("payload")),
  "ts_ms"           INTEGER NOT NULL,
  "delivered_at_ms" INTEGER,
  "delivery_count"  INTEGER NOT NULL DEFAULT 0,
  "consumer"        TEXT,
  "last_error"      TEXT
) STRICT;
-- Acked entries are deleted, so the table holds unacked entries only: one
-- index serves the read of new entries and the `max_len` cap.
CREATE INDEX "event_entry_stream" ON "event_entry" ("stream", "id");
CREATE INDEX "event_entry_pending" ON "event_entry" ("stream", "delivered_at_ms") WHERE "delivered_at_ms" IS NOT NULL;

CREATE TABLE "event_dead_letter" (
  "id"             INTEGER PRIMARY KEY NOT NULL,
  "stream"         TEXT    NOT NULL,
  "type"           TEXT    NOT NULL,
  "payload"        TEXT    NOT NULL,
  "original_id"    TEXT    NOT NULL,
  "delivery_count" INTEGER NOT NULL,
  "error"          TEXT    NOT NULL,
  "ts_ms"          INTEGER NOT NULL
) STRICT;
CREATE INDEX "event_dead_letter_stream" ON "event_dead_letter" ("stream", "id");
