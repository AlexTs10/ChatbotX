-- Q.P7-V1: the ConnectSession purpose CHECK gets `meta_capi`, so a Meta CAPI
-- connect no longer shares (and cancels) the `messaging_ads` session. SQLite
-- cannot alter a CHECK: the table is rebuilt (the runner turns foreign keys
-- off around each migration).
CREATE TABLE "ConnectSession_new" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "provider" TEXT NOT NULL,
  "purpose" TEXT NOT NULL,
  "targetConnectionId" INTEGER REFERENCES "Connection" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "actorUserId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "actorTokenId" INTEGER REFERENCES "WorkspaceApiToken" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "platformOwnerId" TEXT,
  "originHost" TEXT,
  "returnUrl" TEXT,
  "stateNonceHash" TEXT NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'pending',
  "step" TEXT NOT NULL DEFAULT 'authorize',
  "nextAction" TEXT CHECK ("nextAction" IS NULL OR json_valid("nextAction")),
  "encryptedAuth" TEXT CHECK ("encryptedAuth" IS NULL OR json_valid("encryptedAuth")),
  "targets" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("targets")),
  "targetClaims" TEXT NOT NULL DEFAULT '{}' CHECK (json_valid("targetClaims")),
  "resultConnectionIds" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("resultConnectionIds") AND json_type("resultConnectionIds") = 'array'),
  "results" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("results")),
  "errorCode" TEXT,
  "expiresAt" INTEGER NOT NULL,
  "consumedAt" INTEGER,
  CONSTRAINT "ConnectSession_actor_at_most_one" CHECK ((("actorUserId" IS NOT NULL) + ("actorTokenId" IS NOT NULL)) <= 1),
  CONSTRAINT "ConnectSession_status_check" CHECK ("status" IN ('pending', 'authorized', 'awaiting_selection', 'completed', 'failed', 'expired', 'cancelled')),
  CONSTRAINT "ConnectSession_purpose_check" CHECK ("purpose" IN ('connect', 'reconnect', 'facebook_ads', 'messaging_ads', 'lead_ads', 'meta_catalog', 'meta_capi')),
  CONSTRAINT "ConnectSession_errorCode_check" CHECK ("errorCode" IN ('state_mismatch', 'expired', 'provider_denied', 'exchange_failed', 'provider_error', 'no_candidates', 'already_connected', 'quota_exceeded', 'trial_expired', 'internal_error')),
  CONSTRAINT "ConnectSession_errorCode_terminal_failure_check" CHECK ("errorCode" IS NULL OR "status" IN ('failed', 'expired', 'cancelled')),
  CONSTRAINT "ConnectSession_terminal_consumedAt_check" CHECK (("status" IN ('completed', 'failed', 'expired', 'cancelled')) = ("consumedAt" IS NOT NULL)),
  CONSTRAINT "ConnectSession_terminal_clears_encryptedAuth_check" CHECK ("status" NOT IN ('completed', 'failed', 'expired', 'cancelled') OR "encryptedAuth" IS NULL),
  CONSTRAINT "ConnectSession_active_reconnect_requires_target_check" CHECK ("purpose" <> 'reconnect' OR "status" IN ('completed', 'failed', 'expired', 'cancelled') OR "targetConnectionId" IS NOT NULL)
) STRICT;
INSERT INTO "ConnectSession_new" ("id", "createdAt", "updatedAt", "workspaceId", "provider", "purpose", "targetConnectionId", "actorUserId", "actorTokenId", "platformOwnerId", "originHost", "returnUrl", "stateNonceHash", "status", "step", "nextAction", "encryptedAuth", "targets", "targetClaims", "resultConnectionIds", "results", "errorCode", "expiresAt", "consumedAt")
  SELECT "id", "createdAt", "updatedAt", "workspaceId", "provider", "purpose", "targetConnectionId", "actorUserId", "actorTokenId", "platformOwnerId", "originHost", "returnUrl", "stateNonceHash", "status", "step", "nextAction", "encryptedAuth", "targets", "targetClaims", "resultConnectionIds", "results", "errorCode", "expiresAt", "consumedAt" FROM "ConnectSession";
DROP TABLE "ConnectSession";
ALTER TABLE "ConnectSession_new" RENAME TO "ConnectSession";
CREATE INDEX "ConnectSession_workspaceId_idx" ON "ConnectSession" ("workspaceId");
CREATE INDEX "ConnectSession_expiresAt_idx" ON "ConnectSession" ("expiresAt");
CREATE INDEX "ConnectSession_consumedAt_idx" ON "ConnectSession" ("consumedAt") WHERE "consumedAt" IS NOT NULL;
CREATE UNIQUE INDEX "ConnectSession_stateNonceHash_key" ON "ConnectSession" ("stateNonceHash");
