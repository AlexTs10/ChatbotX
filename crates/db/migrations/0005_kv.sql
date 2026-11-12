-- P2.7: durable key-value rows that replace Redis keys whose loss on restart
-- changes behaviour (spec conversion/specs/P2.7.md 1.6, 2.9). Owned by
-- crates/kv (`DurableKv`). Reads treat "expiresAtMs" <= now as absent; one
-- sweeper deletes expired rows in batches.

CREATE TABLE "kv_entry" (
  "key"         TEXT    NOT NULL PRIMARY KEY,
  -- 0 string/JSON, 1 hash (JSON object field -> text), 2 list (JSON array of texts)
  "kind"        INTEGER NOT NULL CHECK ("kind" IN (0, 1, 2)),
  "value"       TEXT    NOT NULL,
  -- unix ms; NULL = no expiry
  "expiresAtMs" INTEGER
) STRICT, WITHOUT ROWID;

CREATE INDEX "kv_entry_expiresAtMs_idx" ON "kv_entry" ("expiresAtMs") WHERE "expiresAtMs" IS NOT NULL;
