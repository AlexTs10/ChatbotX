-- crates/db/migrations/0002_messaging.sql  (P2.4: schema part 2, 65 tables)
-- Generated once from packages/database/drizzle/20261110161124_resource_deletion_log/snapshot.json
-- by the P2.2 rules (as applied in 0001_core.sql), then reviewed by hand. Do not edit after release:
-- the runner stores a checksum. Not created: "MessageShard", "ShardTimeRange" (sharding is not used,
-- DECISIONS P2.2). "ContactOnSmartDelay"."appointmentId" (FK to the part-3 table "Appointment") and
-- its index are added by 0003_rest.sql (P2.5).
-- Needs no app SQL function (no index, CHECK or trigger in this file calls one).

-- Flow
-- "currentVersionId", "draftVersionId": no FK in Postgres either (circular with FlowVersion); kept as plain columns.
CREATE TABLE "Flow" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "active" INTEGER NOT NULL DEFAULT 1 CHECK ("active" IN (0, 1)),
  "enableInInbox" INTEGER NOT NULL DEFAULT 1 CHECK ("enableInInbox" IN (0, 1)),
  "currentVersionId" INTEGER,
  "draftVersionId" INTEGER,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "folderId" INTEGER REFERENCES "Folder" ("id") ON DELETE SET NULL ON UPDATE CASCADE
) STRICT;
CREATE INDEX "Flow_workspaceId_idx" ON "Flow" ("workspaceId");

-- Forward FKs of part-1 tables to "Flow" (P2.3 spec 1.5). ADD COLUMN ... REFERENCES is allowed because the
-- default is NULL (checked on these STRICT tables). The columns become the last columns of their tables.
ALTER TABLE "IntegrationWhatsapp"  ADD COLUMN "handoverResumeFlowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "IntegrationMessenger" ADD COLUMN "welcomeFlowId"        INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "IntegrationMessenger" ADD COLUMN "handoverResumeFlowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "IntegrationInstagram" ADD COLUMN "welcomeFlowId"        INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "IntegrationWebchat"   ADD COLUMN "welcomeFlowId"        INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE;
ALTER TABLE "IntegrationZalo"      ADD COLUMN "fallbackFlowId"       INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE;
CREATE INDEX "IntegrationWhatsapp_handoverResumeFlowId_idx"  ON "IntegrationWhatsapp"  ("handoverResumeFlowId");
CREATE INDEX "IntegrationMessenger_welcomeFlowId_idx"        ON "IntegrationMessenger" ("welcomeFlowId");
CREATE INDEX "IntegrationMessenger_handoverResumeFlowId_idx" ON "IntegrationMessenger" ("handoverResumeFlowId");
CREATE INDEX "IntegrationInstagram_welcomeFlowId_idx"        ON "IntegrationInstagram" ("welcomeFlowId");
CREATE INDEX "IntegrationWebchat_welcomeFlowId_idx"          ON "IntegrationWebchat"   ("welcomeFlowId");
CREATE INDEX "IntegrationZalo_fallbackFlowId_idx"            ON "IntegrationZalo"      ("fallbackFlowId");

-- FlowVersion
-- "nodes", "edges": Postgres jsonb[] -> one JSON array TEXT each; no default (Postgres has none).
CREATE TABLE "FlowVersion" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "flowId" INTEGER NOT NULL REFERENCES "Flow" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "nodes" TEXT NOT NULL CHECK (json_valid("nodes") AND json_type("nodes") = 'array'),
  "edges" TEXT NOT NULL CHECK (json_valid("edges") AND json_type("edges") = 'array'),
  "isDraft" INTEGER NOT NULL CHECK ("isDraft" IN (0, 1)),
  "isLatest" INTEGER NOT NULL DEFAULT 0 CHECK ("isLatest" IN (0, 1)),
  "startNodeId" INTEGER NOT NULL
) STRICT;
CREATE INDEX "FlowVersion_workspaceId_idx" ON "FlowVersion" ("workspaceId");
CREATE INDEX "FlowVersion_flowId_idx" ON "FlowVersion" ("flowId");

-- FlowRun
CREATE TABLE "FlowRun" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "flowId" INTEGER NOT NULL REFERENCES "Flow" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "flowVersionId" INTEGER NOT NULL REFERENCES "FlowVersion" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "conversationId" INTEGER REFERENCES "Conversation" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "FlowRun_conversationId_idx" ON "FlowRun" ("conversationId");
CREATE INDEX "FlowRun_workspaceId_idx" ON "FlowRun" ("workspaceId");
CREATE INDEX "FlowRun_flowId_idx" ON "FlowRun" ("flowId");
CREATE INDEX "FlowRun_flowVersionId_idx" ON "FlowRun" ("flowVersionId");

-- FlowNodeStat
CREATE TABLE "FlowNodeStat" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "flowId" INTEGER NOT NULL,
  "analyticsId" INTEGER NOT NULL,
  "nodeId" TEXT NOT NULL,
  "buttonId" TEXT,
  "contactId" INTEGER NOT NULL,
  "contactInboxId" INTEGER NOT NULL,
  "eventType" TEXT NOT NULL,
  "errorContent" TEXT,
  "occurredAt" INTEGER,
  "seenAt" INTEGER,
  "refId" TEXT,
  "refType" INTEGER
) STRICT;
CREATE INDEX "FlowNodeStat_filter_1_idx" ON "FlowNodeStat" ("workspaceId", "analyticsId", "nodeId", "eventType", "buttonId");
CREATE INDEX "FlowNodeStat_filter_2_idx" ON "FlowNodeStat" ("workspaceId", "analyticsId", "nodeId") WHERE "eventType" = 'seen' AND "seenAt" IS NOT NULL;

-- FlowAnalyticsSession
CREATE TABLE "FlowAnalyticsSession" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "flowId" INTEGER NOT NULL,
  "workspaceId" INTEGER NOT NULL,
  "deletedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "FlowAnalyticsSession_workspaceId_flowId_key" ON "FlowAnalyticsSession" ("flowId", "workspaceId") WHERE "deletedAt" IS NULL;
CREATE INDEX "FlowAnalyticsSession_flowId_idx" ON "FlowAnalyticsSession" ("flowId");

-- Trigger
-- "actions": Postgres jsonb[] -> JSON array TEXT.
CREATE TABLE "Trigger" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "active" INTEGER NOT NULL DEFAULT 1 CHECK ("active" IN (0, 1)),
  "folderId" INTEGER REFERENCES "Folder" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "actions" TEXT NOT NULL CHECK (json_valid("actions") AND json_type("actions") = 'array'),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "Trigger_workspaceId_name_key" ON "Trigger" ("workspaceId", "name");
CREATE INDEX "Trigger_workspaceId_idx" ON "Trigger" ("workspaceId");
CREATE INDEX "Trigger_folderId_idx" ON "Trigger" ("folderId");
CREATE INDEX "Trigger_workspaceId_active_idx" ON "Trigger" ("workspaceId", "active");

-- Condition
-- "operator" varchar(255) -> TEXT + length CHECK.
CREATE TABLE "Condition" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "triggerId" INTEGER REFERENCES "Trigger" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "webhookId" INTEGER REFERENCES "Webhook" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "type" TEXT NOT NULL,
  "sourceId" TEXT,
  "operator" TEXT CHECK ("operator" IS NULL OR length("operator") <= 255),
  "value" TEXT CHECK ("value" IS NULL OR json_valid("value"))
) STRICT;
CREATE INDEX "Condition_type_source_id_idx" ON "Condition" ("type", "sourceId");
CREATE INDEX "Condition_triggerId_idx" ON "Condition" ("triggerId");
CREATE INDEX "Condition_webhookId_idx" ON "Condition" ("webhookId");
CREATE INDEX "Condition_type_sourceId_triggerId_idx" ON "Condition" ("type", "sourceId", "triggerId");
CREATE INDEX "Condition_type_sourceId_webhookId_idx" ON "Condition" ("type", "sourceId", "webhookId");

-- TriggerExecution
CREATE TABLE "TriggerExecution" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "executedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "triggerId" INTEGER NOT NULL REFERENCES "Trigger" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "TriggerExecution_triggerId_contactId_idx" ON "TriggerExecution" ("triggerId", "contactId");
CREATE INDEX "TriggerExecution_workspaceId_idx" ON "TriggerExecution" ("workspaceId");
CREATE INDEX "TriggerExecution_contactId_idx" ON "TriggerExecution" ("contactId");

-- TriggerStat
CREATE TABLE "TriggerStat" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "triggerId" INTEGER NOT NULL REFERENCES "Trigger" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "date" INTEGER NOT NULL,
  "totalContacts" INTEGER NOT NULL DEFAULT 0,
  "successCount" INTEGER NOT NULL DEFAULT 0,
  "failureCount" INTEGER NOT NULL DEFAULT 0,
  "totalExecutions" INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE UNIQUE INDEX "TriggerStat_triggerId_date_key" ON "TriggerStat" ("triggerId", "date");
CREATE INDEX "TriggerStat_triggerId_date_idx" ON "TriggerStat" ("triggerId", "date");
CREATE INDEX "TriggerStat_workspaceId_date_idx" ON "TriggerStat" ("workspaceId", "date");

-- TriggerContactHistory
CREATE TABLE "TriggerContactHistory" (
  "id" INTEGER NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "triggerId" INTEGER NOT NULL REFERENCES "Trigger" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "firstEnteredAt" INTEGER NOT NULL,
  CONSTRAINT "TriggerContactHistory_pkey" PRIMARY KEY ("id", "contactId")
) STRICT;
CREATE INDEX "TriggerContactHistory_triggerId_contactId_idx" ON "TriggerContactHistory" ("triggerId", "contactId");
CREATE INDEX "TriggerContactHistory_contactId_idx" ON "TriggerContactHistory" ("contactId");
CREATE INDEX "TriggerContactHistory_workspaceId_idx" ON "TriggerContactHistory" ("workspaceId");

-- AutomatedResponse
-- "keywords": Postgres text[] queried with @> -> JSON array + json_each (P2.2 1.6).
CREATE TABLE "AutomatedResponse" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "folderId" INTEGER REFERENCES "Folder" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "keywords" TEXT NOT NULL CHECK (json_valid("keywords") AND json_type("keywords") = 'array'),  -- NO DB default (Postgres parity: drizzle .default(sql`[]`) is app-side only). Rust always writes it.
  "conditions" TEXT CHECK ("conditions" IS NULL OR json_valid("conditions")),
  "status" INTEGER NOT NULL CHECK ("status" IN (0, 1)),
  "text" TEXT,
  "flowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "type" TEXT NOT NULL DEFAULT 'inbound' CHECK ("type" IN ('inbound', 'outbound'))
) STRICT;
CREATE INDEX "AutomatedResponse_workspaceId_idx" ON "AutomatedResponse" ("workspaceId");

-- AutomationThrottle
-- Postgres: HASH(workspaceId) x32. SQLite: one table, same PK (DECISIONS P2.2). "claimId" is the only uuid column (app-generated).
CREATE TABLE "AutomationThrottle" (
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactInboxId" INTEGER NOT NULL REFERENCES "ContactInbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "throttleType" TEXT NOT NULL CHECK ("throttleType" IN ('defaultReply')),
  "subjectId" INTEGER NOT NULL,
  "lastTriggeredAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "claimId" TEXT NOT NULL CHECK (length("claimId") = 36),
  CONSTRAINT "AutomationThrottle_pkey" PRIMARY KEY ("workspaceId", "contactInboxId", "throttleType", "subjectId")
) STRICT;
CREATE INDEX "AutomationThrottle_lastTriggeredAt_idx" ON "AutomationThrottle" ("lastTriggeredAt");
CREATE INDEX "AutomationThrottle_contactInboxId_idx" ON "AutomationThrottle" ("contactInboxId");

-- Webhook
CREATE TABLE "Webhook" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "active" INTEGER NOT NULL DEFAULT 1 CHECK ("active" IN (0, 1)),
  "folderId" INTEGER REFERENCES "Folder" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "url" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "Webhook_workspaceId_idx" ON "Webhook" ("workspaceId");
CREATE INDEX "Webhook_folderId_idx" ON "Webhook" ("folderId");
CREATE INDEX "Webhook_workspaceId_active_idx" ON "Webhook" ("workspaceId", "active");

-- WebhookExecution
CREATE TABLE "WebhookExecution" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "executedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "webhookId" INTEGER NOT NULL REFERENCES "Webhook" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "WebhookExecution_webhookId_contactId_key" ON "WebhookExecution" ("webhookId", "contactId");
CREATE INDEX "WebhookExecution_workspaceId_idx" ON "WebhookExecution" ("workspaceId");
CREATE INDEX "WebhookExecution_contactId_idx" ON "WebhookExecution" ("contactId");

-- ExternalWebhook
CREATE TABLE "ExternalWebhook" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "provider" TEXT NOT NULL DEFAULT 'make',
  "event" TEXT NOT NULL,
  "url" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "ExternalWebhook_workspaceId_event_idx" ON "ExternalWebhook" ("workspaceId", "event");
CREATE UNIQUE INDEX "ExternalWebhook_workspaceId_event_url_key" ON "ExternalWebhook" ("workspaceId", "event", "url");

-- InboundWebhook
CREATE TABLE "InboundWebhook" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "active" INTEGER NOT NULL DEFAULT 0 CHECK ("active" IN (0, 1)),
  "token" TEXT NOT NULL,
  "inboxResolution" TEXT CHECK ("inboxResolution" IS NULL OR json_valid("inboxResolution")),
  "identifiers" TEXT NOT NULL CHECK (json_valid("identifiers")),
  "fieldMappings" TEXT NOT NULL CHECK (json_valid("fieldMappings")),
  "actions" TEXT NOT NULL CHECK (json_valid("actions")),
  "sampleJson" TEXT CHECK ("sampleJson" IS NULL OR json_valid("sampleJson")),
  "requestCount" INTEGER NOT NULL DEFAULT 0
) STRICT;
CREATE INDEX "InboundWebhook_workspaceId_idx" ON "InboundWebhook" ("workspaceId");
CREATE UNIQUE INDEX "InboundWebhook_token_key" ON "InboundWebhook" ("token");

-- InboundWebhookRequest
CREATE TABLE "InboundWebhookRequest" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboundWebhookId" INTEGER NOT NULL REFERENCES "InboundWebhook" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "idempotencyKey" TEXT,
  "clientIp" TEXT,
  "userAgent" TEXT,
  "status" TEXT NOT NULL DEFAULT 'received' CHECK ("status" IN ('received', 'processing', 'processed', 'completedWithErrors', 'skipped', 'failed')),
  "claimToken" TEXT,
  "claimedAt" INTEGER,
  "attempts" INTEGER NOT NULL DEFAULT 0,
  "skipReason" TEXT,
  "errorCode" TEXT,
  "errorMessage" TEXT,
  "inboxId" INTEGER REFERENCES "Inbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "contactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "contactInboxId" INTEGER,
  "payload" TEXT NOT NULL CHECK (json_valid("payload")),
  "fieldErrors" TEXT CHECK ("fieldErrors" IS NULL OR json_valid("fieldErrors")),
  "actionResults" TEXT CHECK ("actionResults" IS NULL OR json_valid("actionResults")),
  "processedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "InboundWebhookRequest_inboundWebhookId_idempotencyKey_key" ON "InboundWebhookRequest" ("inboundWebhookId", "idempotencyKey") WHERE "idempotencyKey" IS NOT NULL;
CREATE INDEX "InboundWebhookRequest_inboundWebhookId_createdAt_idx" ON "InboundWebhookRequest" ("inboundWebhookId", "createdAt" DESC);
CREATE INDEX "InboundWebhookRequest_workspaceId_idx" ON "InboundWebhookRequest" ("workspaceId");
CREATE INDEX "InboundWebhookRequest_contactId_idx" ON "InboundWebhookRequest" ("contactId");
CREATE INDEX "InboundWebhookRequest_inboxId_idx" ON "InboundWebhookRequest" ("inboxId");
CREATE INDEX "InboundWebhookRequest_createdAt_received_idx" ON "InboundWebhookRequest" ("createdAt") WHERE "status" = 'received';
CREATE INDEX "InboundWebhookRequest_claimedAt_processing_idx" ON "InboundWebhookRequest" ("claimedAt") WHERE "status" = 'processing';
CREATE INDEX "InboundWebhookRequest_createdAt_idx" ON "InboundWebhookRequest" ("createdAt");

-- Conversation
-- Two partial uniques (sourceId IS NULL / IS NOT NULL): one DM per contact, one thread per (contact, sourceId).
-- Postgres DESC NULLS LAST on "lastActivityAt" = SQLite DESC default (NULLs last).
CREATE TABLE "Conversation" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "botEnabled" INTEGER NOT NULL DEFAULT 1 CHECK ("botEnabled" IN (0, 1)),
  "botResumeAt" INTEGER,
  "archivedAt" INTEGER,
  "additionalAttributes" TEXT CHECK ("additionalAttributes" IS NULL OR json_valid("additionalAttributes")),
  "contactLastReadAt" INTEGER,
  "agentLastReadAt" INTEGER,
  "aiContextLastMessageId" INTEGER,
  "lastActivityAt" INTEGER,
  "followed" INTEGER NOT NULL DEFAULT 0 CHECK ("followed" IN (0, 1)),
  "assignedUserId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "assignedInboxTeamId" INTEGER REFERENCES "InboxTeam" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "sourceId" TEXT,
  "lastStep" TEXT,
  "currentStep" TEXT,
  "adminRepliedAt" INTEGER,
  "contactRepliedAt" INTEGER,
  "awaitingHumanReplySince" INTEGER,
  "awaitingHumanReplyClosedAt" INTEGER,
  "assignedUserAt" INTEGER,
  "responseDueAt" INTEGER,
  "responseSlaBreachingAt" INTEGER,
  "responseSlaTargetSeconds" INTEGER
) STRICT;
CREATE UNIQUE INDEX "Conversation_contactId_sourceId_key" ON "Conversation" ("contactId", "sourceId") WHERE "sourceId" IS NOT NULL;
CREATE UNIQUE INDEX "Conversation_contactId_dm_key" ON "Conversation" ("contactId") WHERE "sourceId" IS NULL;
CREATE INDEX "Conversation_contactId_idx" ON "Conversation" ("contactId");
CREATE INDEX "Conversation_workspaceId_responseDueAt_idx" ON "Conversation" ("workspaceId", "responseDueAt") WHERE "responseDueAt" IS NOT NULL;
CREATE INDEX "Conversation_aiContextLastMessageId_idx" ON "Conversation" ("aiContextLastMessageId");
CREATE INDEX "Conversation_workspaceId_lastActivityAt_id_idx" ON "Conversation" ("workspaceId", "lastActivityAt" DESC, "id" DESC);
CREATE INDEX "Conversation_workspaceId_assignedUserId_lastActivityAt_id_idx" ON "Conversation" ("workspaceId", "assignedUserId", "lastActivityAt" DESC, "id" DESC) WHERE "assignedUserId" IS NOT NULL;
CREATE INDEX "Conversation_ws_team_lastActivity_id_idx" ON "Conversation" ("workspaceId", "assignedInboxTeamId", "lastActivityAt" DESC, "id" DESC) WHERE "assignedInboxTeamId" IS NOT NULL;

-- ConversationParticipant
CREATE TABLE "ConversationParticipant" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "conversationId" INTEGER NOT NULL REFERENCES "Conversation" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "userId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "ConversationParticipant_workspaceId_idx" ON "ConversationParticipant" ("workspaceId");
CREATE INDEX "ConversationParticipant_conversationId_idx" ON "ConversationParticipant" ("conversationId");
CREATE UNIQUE INDEX "ConversationParticipant_conversationId_userId_key" ON "ConversationParticipant" ("conversationId", "userId");

-- Message
-- Postgres: TimescaleDB hypertable (7-day chunks, compression after 30 d) + message sharding. SQLite: one plain table.
-- PK (id, createdAt) -> PK (id): "createdAt" was only in the key because of the hypertable.
-- "Message_source_dedup_idx" keeps createdAt (3 columns): the ON CONFLICT target of the ported repository is
-- (contactInboxId, sourceId, createdAt) (frozen sharded-message-repository.test.ts:90). No FKs (same as Postgres).
-- updatedAt: the Postgres trigger update_updated_at_column becomes Rust code (P2.2 3.4).
CREATE TABLE "Message" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "conversationId" INTEGER NOT NULL,
  "contactInboxId" INTEGER NOT NULL,
  "workspaceId" INTEGER NOT NULL,
  "text" TEXT,
  "contentAttributes" TEXT CHECK ("contentAttributes" IS NULL OR json_valid("contentAttributes")),
  "messageType" TEXT NOT NULL CHECK ("messageType" IN ('incoming', 'outgoing', 'activity')),
  "contentType" TEXT NOT NULL CHECK ("contentType" IN ('text', 'location', 'refLink')),
  "senderType" TEXT NOT NULL CHECK ("senderType" IN ('bot', 'contact', 'system', 'user', 'api')),
  "senderId" INTEGER,
  "sourceId" TEXT,
  "deletedAt" INTEGER,
  "type" TEXT NOT NULL DEFAULT 'message' CHECK ("type" IN ('message', 'comment')),
  "parentId" TEXT,
  "attributes" TEXT CHECK ("attributes" IS NULL OR json_valid("attributes")),
  "sendError" TEXT,
  CONSTRAINT "Message_source_dedup_idx" UNIQUE ("contactInboxId", "sourceId", "createdAt")
) STRICT;
CREATE INDEX "Message_conversation_history_idx" ON "Message" ("conversationId", "createdAt" DESC, "id" DESC);
CREATE INDEX "Message_workspace_created_idx" ON "Message" ("workspaceId", "createdAt" DESC);
CREATE INDEX "Message_contactInboxId_sourceId_createdAt_idx" ON "Message" ("contactInboxId", "sourceId", "createdAt" DESC);
CREATE INDEX "Message_conversationId_type_idx" ON "Message" ("workspaceId", "conversationId", "type", "createdAt" DESC);
CREATE INDEX "Message_parentId_idx" ON "Message" ("workspaceId", "parentId", "type", "createdAt" DESC) WHERE "parentId" IS NOT NULL;
-- SQLite-only (DECISIONS P2.4 #3, spec P2.4 2.3): serves the per-webhook-event "... by source id" updates
-- (WHERE "sourceId" = ? AND "workspaceId" = ?, no contact inbox, no time bound). Postgres had no such index.
CREATE INDEX "Message_workspaceId_sourceId_idx" ON "Message" ("workspaceId", "sourceId") WHERE "sourceId" IS NOT NULL;

-- Attachment
-- Postgres: TimescaleDB hypertable. SQLite: one plain table. PK (id, createdAt) -> PK (id). No FKs (same as Postgres).
-- updatedAt: Rust sets it (Postgres trigger update_updated_at_column).
CREATE TABLE "Attachment" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL,
  "conversationId" INTEGER NOT NULL,
  "messageId" INTEGER NOT NULL,
  "messageCreatedAt" INTEGER NOT NULL,
  "fileType" TEXT NOT NULL CHECK ("fileType" IN ('image', 'video', 'audio', 'gif', 'file')),
  "sourceId" TEXT,
  "mimeType" TEXT NOT NULL,
  "width" INTEGER,
  "height" INTEGER,
  "size" INTEGER NOT NULL DEFAULT 0,
  "thumbnailPath" TEXT,
  "originPath" TEXT NOT NULL,
  "name" TEXT,
  "keepOriginal" INTEGER CHECK ("keepOriginal" IN (0, 1))
) STRICT;
CREATE INDEX "Attachment_message_idx" ON "Attachment" ("messageId", "messageCreatedAt" DESC);
CREATE INDEX "Attachment_workspaceId_createdAt_idx" ON "Attachment" ("workspaceId", "createdAt" DESC);
CREATE INDEX "Attachment_conversationId_idx" ON "Attachment" ("conversationId", "createdAt" DESC);

-- MessageCleanup
CREATE TABLE "MessageCleanup" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL,
  "contactId" INTEGER NOT NULL,
  "contactInboxId" INTEGER NOT NULL,
  "inboxId" INTEGER NOT NULL,
  "sourceId" TEXT NOT NULL,
  "conversationIds" TEXT NOT NULL CHECK (json_valid("conversationIds")),
  "sinceTime" INTEGER,
  "deletedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "status" TEXT NOT NULL DEFAULT 'pending' CHECK ("status" IN ('pending', 'processing', 'completed', 'failed')),
  "attempts" INTEGER NOT NULL DEFAULT 0,
  "lastError" TEXT,
  "processedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "MessageCleanup_inboxId_sourceId_key" ON "MessageCleanup" ("inboxId", "sourceId");
CREATE INDEX "MessageCleanup_status_createdAt_idx" ON "MessageCleanup" ("status", "createdAt");
CREATE INDEX "MessageCleanup_workspaceId_idx" ON "MessageCleanup" ("workspaceId");

-- ChannelPost
CREATE TABLE "ChannelPost" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "channel" TEXT NOT NULL,
  "integrationId" INTEGER NOT NULL,
  "sourceAccountId" TEXT NOT NULL,
  "externalPostId" TEXT NOT NULL,
  "caption" TEXT,
  "mediaType" TEXT,
  "thumbnail" TEXT,
  "permalink" TEXT,
  "publishedAt" INTEGER,
  "metadataFetchedAt" INTEGER,
  "metadataAttemptedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "ChannelPost_workspaceId_channel_externalPostId_key" ON "ChannelPost" ("workspaceId", "channel", "externalPostId");
CREATE INDEX "ChannelPost_workspaceId_sortAt_id_idx" ON "ChannelPost" ("workspaceId", COALESCE("publishedAt", "createdAt") DESC, "id" DESC);
CREATE INDEX "ChannelPost_inboxId_idx" ON "ChannelPost" ("inboxId");

-- CoexistSyncRun
CREATE TABLE "CoexistSyncRun" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL,
  "integrationId" INTEGER NOT NULL,
  "channel" TEXT NOT NULL CHECK ("channel" IN ('whatsapp', 'messenger', 'instagram')),
  "type" TEXT NOT NULL DEFAULT 'coexist' CHECK ("type" IN ('coexist', 'contact_scan')),
  "status" TEXT NOT NULL DEFAULT 'init' CHECK ("status" IN ('init', 'running', 'succeeded', 'failed', 'partial', 'waiting')),
  "triggerSource" TEXT NOT NULL,
  "startedAt" INTEGER,
  "finishedAt" INTEGER,
  "lastHeartbeatAt" INTEGER,
  "totalScan" INTEGER NOT NULL DEFAULT 0,
  "currentScan" INTEGER NOT NULL DEFAULT 0,
  "currentStep" TEXT,
  "lastSyncedAt" INTEGER,
  "currentPageNumber" INTEGER NOT NULL DEFAULT 0,
  "importedContactCount" INTEGER NOT NULL DEFAULT 0,
  "importedMessageCount" INTEGER NOT NULL DEFAULT 0,
  "skippedCount" INTEGER NOT NULL DEFAULT 0,
  "failedCount" INTEGER NOT NULL DEFAULT 0,
  "attempts" INTEGER NOT NULL DEFAULT 0,
  "currentError" TEXT,
  "lastPhase" INTEGER,
  "lastChunkOrder" INTEGER,
  "syncProgress" INTEGER NOT NULL DEFAULT 0,
  "messengerSyncPhase" TEXT NOT NULL DEFAULT 'contacts' CHECK ("messengerSyncPhase" IN ('contacts', 'messages')),
  "pendingPatches" TEXT CHECK ("pendingPatches" IS NULL OR json_valid("pendingPatches")),
  "claimToken" TEXT,
  "scanFromAt" INTEGER,
  "requestedByUserId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "resumeCursor" TEXT
) STRICT;
CREATE INDEX "CoexistSyncRun_workspace_idx" ON "CoexistSyncRun" ("workspaceId");
CREATE INDEX "CoexistSyncRun_integration_idx" ON "CoexistSyncRun" ("integrationId");
CREATE INDEX "CoexistSyncRun_active_idx" ON "CoexistSyncRun" ("status", "lastHeartbeatAt") WHERE status IN ('init', 'running');
CREATE INDEX "CoexistSyncRun_integration_resume_idx" ON "CoexistSyncRun" ("integrationId", "channel", "startedAt" DESC) WHERE status IN ('succeeded', 'partial');
CREATE UNIQUE INDEX "CoexistSyncRun_integration_init_uq" ON "CoexistSyncRun" ("integrationId", "channel") WHERE status = 'init' AND type = 'coexist';
CREATE UNIQUE INDEX "CoexistSyncRun_contact_scan_active_uq" ON "CoexistSyncRun" ("integrationId") WHERE type = 'contact_scan' AND status IN ('init', 'running');
CREATE INDEX "CoexistSyncRun_contact_scan_due_idx" ON "CoexistSyncRun" ("type", "status", "createdAt") WHERE type = 'contact_scan';

-- WhatsappCoexistStaging
CREATE TABLE "WhatsappCoexistStaging" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "phoneNumberId" TEXT NOT NULL,
  "payload" TEXT NOT NULL CHECK (json_valid("payload")),
  "payloadHash" TEXT NOT NULL,
  "processedAt" INTEGER,
  "parseFailedAt" INTEGER
) STRICT;
CREATE INDEX "WhatsappCoexistStaging_phoneNumberId_idx" ON "WhatsappCoexistStaging" ("phoneNumberId");
CREATE UNIQUE INDEX "WhatsappCoexistStaging_phone_hash_uq" ON "WhatsappCoexistStaging" ("phoneNumberId", "payloadHash");
CREATE INDEX "WhatsappCoexistStaging_processedAt_idx" ON "WhatsappCoexistStaging" ("processedAt") WHERE "processedAt" IS NOT NULL;
CREATE INDEX "WhatsappCoexistStaging_parseFailedAt_idx" ON "WhatsappCoexistStaging" ("parseFailedAt") WHERE "parseFailedAt" IS NOT NULL;

-- WhatsappCall
CREATE TABLE "WhatsappCall" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "wacid" TEXT,
  "attemptId" TEXT,
  "direction" TEXT NOT NULL CHECK ("direction" IN ('userInitiated', 'businessInitiated')),
  "status" TEXT NOT NULL DEFAULT 'ringing' CHECK ("status" IN ('ringing', 'accepted', 'rejected', 'completed', 'failed')),
  "outcome" TEXT CHECK ("outcome" IN ('completed', 'failed', 'rejected', 'canceled')),
  "startedAt" INTEGER,
  "endedAt" INTEGER,
  "durationSeconds" INTEGER,
  "messageId" INTEGER,
  "lastError" TEXT,
  "answeredByUserId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "initiatedByUserId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "recordingRequested" INTEGER CHECK ("recordingRequested" IN (0, 1)),
  "recordingFailureReason" TEXT,
  "recordingPath" TEXT,
  "recordedAt" INTEGER,
  "transcript" TEXT,
  "transcribedAt" INTEGER,
  "transcriptSegments" TEXT CHECK ("transcriptSegments" IS NULL OR json_valid("transcriptSegments")),
  "aiSummary" TEXT CHECK ("aiSummary" IS NULL OR json_valid("aiSummary")),
  "aiSummarizedAt" INTEGER,
  "aiSummaryProvider" TEXT,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactInboxId" INTEGER NOT NULL REFERENCES "ContactInbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "conversationId" INTEGER NOT NULL REFERENCES "Conversation" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "WhatsappCall_wacid_key" ON "WhatsappCall" ("wacid") WHERE "wacid" IS NOT NULL;
CREATE UNIQUE INDEX "WhatsappCall_attemptId_key" ON "WhatsappCall" ("attemptId") WHERE "attemptId" IS NOT NULL;
CREATE INDEX "WhatsappCall_workspaceId_idx" ON "WhatsappCall" ("workspaceId");
CREATE INDEX "WhatsappCall_conversationId_idx" ON "WhatsappCall" ("conversationId");
CREATE INDEX "WhatsappCall_contactInboxId_idx" ON "WhatsappCall" ("contactInboxId");
CREATE INDEX "WhatsappCall_workspaceId_createdAt_id_idx" ON "WhatsappCall" ("workspaceId", "createdAt" DESC, "id" DESC);
CREATE INDEX "WhatsappCall_contactInboxId_createdAt_idx" ON "WhatsappCall" ("contactInboxId", "createdAt" DESC);
CREATE INDEX "WhatsappCall_ringing_createdAt_idx" ON "WhatsappCall" ("createdAt") WHERE "status" = 'ringing';
CREATE INDEX "WhatsappCall_resumableRinging_idx" ON "WhatsappCall" ("workspaceId", "createdAt" DESC) WHERE "status" = 'ringing' AND "wacid" IS NOT NULL AND "answeredByUserId" IS NULL;
CREATE INDEX "WhatsappCall_recording_inboxId_recordedAt_idx" ON "WhatsappCall" ("inboxId", "recordedAt") WHERE "recordingPath" IS NOT NULL;
CREATE UNIQUE INDEX "WhatsappCall_pendingOutbound_key" ON "WhatsappCall" ("inboxId", "contactInboxId") WHERE "direction" = 'businessInitiated' AND "status" IN ('ringing', 'accepted');

-- WhatsappCallPermission
CREATE TABLE "WhatsappCallPermission" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "response" TEXT NOT NULL CHECK ("response" IN ('accept', 'reject')),
  "isPermanent" INTEGER NOT NULL DEFAULT 0 CHECK ("isPermanent" IN (0, 1)),
  "expiresAt" INTEGER,
  "respondedAt" INTEGER NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactInboxId" INTEGER NOT NULL REFERENCES "ContactInbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "WhatsappCallPermission_contactInboxId_key" ON "WhatsappCallPermission" ("contactInboxId");
CREATE INDEX "WhatsappCallPermission_workspaceId_idx" ON "WhatsappCallPermission" ("workspaceId");

-- SavedReply
CREATE TABLE "SavedReply" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "shortcut" TEXT NOT NULL,
  "text" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "SavedReply_workspaceId_idx" ON "SavedReply" ("workspaceId");

-- SavedReplyImage
CREATE TABLE "SavedReplyImage" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "savedReplyId" INTEGER NOT NULL REFERENCES "SavedReply" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "mediaLibraryFileId" INTEGER NOT NULL REFERENCES "MediaLibraryFile" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "position" INTEGER NOT NULL
) STRICT;
CREATE INDEX "SavedReplyImage_savedReplyId_idx" ON "SavedReplyImage" ("savedReplyId");
CREATE INDEX "SavedReplyImage_mediaLibraryFileId_idx" ON "SavedReplyImage" ("mediaLibraryFileId");
CREATE INDEX "SavedReplyImage_workspaceId_idx" ON "SavedReplyImage" ("workspaceId");
CREATE UNIQUE INDEX "SavedReplyImage_savedReplyId_mediaLibraryFileId_key" ON "SavedReplyImage" ("savedReplyId", "mediaLibraryFileId");

-- MediaLibraryFile
-- In part 2 (P2.2 section 2) because "SavedReplyImage" cascades from it.
CREATE TABLE "MediaLibraryFile" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "path" TEXT NOT NULL,
  "mimeType" TEXT NOT NULL,
  "size" INTEGER NOT NULL,
  "isFavourite" INTEGER NOT NULL DEFAULT 0 CHECK ("isFavourite" IN (0, 1)),
  "lastAccessedAt" INTEGER,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "folderId" INTEGER REFERENCES "MediaLibraryFolder" ("id") ON DELETE SET NULL ON UPDATE CASCADE
) STRICT;
CREATE INDEX "MediaLibraryFile_workspaceId_idx" ON "MediaLibraryFile" ("workspaceId");
CREATE INDEX "MediaLibraryFile_folderId_idx" ON "MediaLibraryFile" ("folderId");

-- MediaLibraryFolder
CREATE TABLE "MediaLibraryFolder" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "MediaLibraryFolder_workspaceId_idx" ON "MediaLibraryFolder" ("workspaceId");

-- ResponseSlaPolicy
CREATE TABLE "ResponseSlaPolicy" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "enabled" INTEGER NOT NULL DEFAULT 1 CHECK ("enabled" IN (0, 1)),
  "firstResponseTargetSeconds" INTEGER NOT NULL,
  "nextResponseTargetSeconds" INTEGER,
  "breachingThresholdPercent" INTEGER NOT NULL DEFAULT 25,
  CONSTRAINT "ResponseSlaPolicy_workspaceId_key" UNIQUE ("workspaceId"),
  CONSTRAINT "ResponseSlaPolicy_targets_check" CHECK ("firstResponseTargetSeconds" > 0 AND ("nextResponseTargetSeconds" IS NULL OR "nextResponseTargetSeconds" > 0)),
  CONSTRAINT "ResponseSlaPolicy_threshold_check" CHECK ("breachingThresholdPercent" BETWEEN 1 AND 99)
) STRICT;

-- WorkingHours
-- UNIQUE (id, workspaceId) is the parent key of the composite FKs from WorkingHoursMember / WorkingHoursDateOverride.
CREATE TABLE "WorkingHours" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  CONSTRAINT "WorkingHours_id_workspaceId_key" UNIQUE ("id", "workspaceId")
) STRICT;
CREATE INDEX "WorkingHours_workspaceId_idx" ON "WorkingHours" ("workspaceId");

-- WorkingHoursInterval
CREATE TABLE "WorkingHoursInterval" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workingHoursId" INTEGER NOT NULL REFERENCES "WorkingHours" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "weekday" INTEGER NOT NULL,
  "startMinute" INTEGER NOT NULL,
  "endMinute" INTEGER NOT NULL,
  CONSTRAINT "WorkingHoursInterval_weekday_check" CHECK ("weekday" BETWEEN 0 AND 6),
  CONSTRAINT "WorkingHoursInterval_minutes_check" CHECK (0 <= "startMinute" AND "startMinute" < "endMinute" AND "endMinute" <= 1440)
) STRICT;
CREATE INDEX "WorkingHoursInterval_workingHoursId_idx" ON "WorkingHoursInterval" ("workingHoursId");

-- WorkingHoursMember
CREATE TABLE "WorkingHoursMember" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workingHoursId" INTEGER NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "userId" INTEGER NOT NULL REFERENCES "User" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT "WorkingHoursMember_workspaceId_userId_key" UNIQUE ("workspaceId", "userId"),
  CONSTRAINT "WorkingHoursMember_workingHoursId_workspaceId_fkey" FOREIGN KEY ("workingHoursId", "workspaceId") REFERENCES "WorkingHours" ("id", "workspaceId") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "WorkingHoursMember_workingHoursId_idx" ON "WorkingHoursMember" ("workingHoursId");

-- WorkingHoursDateOverride
-- "date" (Postgres date, mode string) -> TEXT YYYY-MM-DD.
-- Composite FK (workingHoursId, workspaceId): with "workingHoursId" NULL the FK is not checked (MATCH SIMPLE, same as Postgres).
CREATE TABLE "WorkingHoursDateOverride" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workingHoursId" INTEGER,
  "date" TEXT NOT NULL CHECK ("date" GLOB '[0-9][0-9][0-9][0-9]-[0-1][0-9]-[0-3][0-9]'),
  "name" TEXT,
  "ranges" TEXT NOT NULL CHECK (json_valid("ranges")),
  CONSTRAINT "WorkingHoursDateOverride_workingHoursId_workspaceId_fkey" FOREIGN KEY ("workingHoursId", "workspaceId") REFERENCES "WorkingHours" ("id", "workspaceId") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "WorkingHoursDateOverride_workspace_date_key" ON "WorkingHoursDateOverride" ("workspaceId", "date") WHERE "workingHoursId" IS NULL;
CREATE UNIQUE INDEX "WorkingHoursDateOverride_schedule_date_key" ON "WorkingHoursDateOverride" ("workingHoursId", "date") WHERE "workingHoursId" IS NOT NULL;
CREATE INDEX "WorkingHoursDateOverride_workspaceId_date_idx" ON "WorkingHoursDateOverride" ("workspaceId", "date");

-- Broadcast
CREATE TABLE "Broadcast" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "flowId" INTEGER REFERENCES "Flow" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "integrationWhatsappId" INTEGER REFERENCES "IntegrationWhatsapp" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "integrationMessengerId" INTEGER REFERENCES "IntegrationMessenger" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "templateId" INTEGER,
  "templateData" TEXT CHECK ("templateData" IS NULL OR json_valid("templateData")),
  "targetMode" TEXT NOT NULL DEFAULT 'channel',
  "status" TEXT NOT NULL CHECK ("status" IN ('scheduled', 'sent', 'sending', 'cancelled', 'draft', 'failed', 'recurring')),
  "schedulesType" TEXT NOT NULL CHECK ("schedulesType" IN ('now', 'future', 'recurring')),
  "schedulesAt" INTEGER NOT NULL,
  "contactFilter" TEXT CHECK ("contactFilter" IS NULL OR json_valid("contactFilter")),
  "subaction" TEXT NOT NULL,
  "channel" TEXT NOT NULL,
  "contactCount" INTEGER,
  "handoffCompletedAt" INTEGER,
  "deletedAt" INTEGER,
  "resumeCount" INTEGER NOT NULL DEFAULT 0,
  "audienceRangeStart" INTEGER,
  "audienceRangeEnd" INTEGER,
  "sendRatePerMinute" INTEGER,
  "parentBroadcastId" INTEGER REFERENCES "Broadcast" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "occurrenceAt" INTEGER,
  CONSTRAINT "Broadcast_occurrence_pair_check" CHECK ("parentBroadcastId" IS NULL OR "occurrenceAt" IS NOT NULL)
) STRICT;
CREATE INDEX "Broadcast_workspaceId_idx" ON "Broadcast" ("workspaceId");
CREATE INDEX "Broadcast_workspaceId_active_idx" ON "Broadcast" ("workspaceId", "channel") WHERE "status" IN ('scheduled', 'sending') AND "deletedAt" IS NULL;
CREATE INDEX "Broadcast_flowId_idx" ON "Broadcast" ("flowId");
CREATE INDEX "Broadcast_channel_idx" ON "Broadcast" ("channel");
CREATE INDEX "Broadcast_schedulesAt_idx" ON "Broadcast" ("schedulesAt");
CREATE INDEX "Broadcast_status_idx" ON "Broadcast" ("status");
CREATE INDEX "Broadcast_deletedAt_idx" ON "Broadcast" ("deletedAt") WHERE "deletedAt" IS NOT NULL;
CREATE UNIQUE INDEX "Broadcast_parentBroadcastId_occurrenceAt_key" ON "Broadcast" ("parentBroadcastId", "occurrenceAt");

-- BroadcastTarget
CREATE TABLE "BroadcastTarget" (
  "broadcastId" INTEGER NOT NULL REFERENCES "Broadcast" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "inboxId" INTEGER NOT NULL REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "flowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "templateId" INTEGER,
  "templateData" TEXT CHECK ("templateData" IS NULL OR json_valid("templateData")),
  CONSTRAINT "BroadcastTarget_pkey" PRIMARY KEY ("broadcastId", "inboxId")
) STRICT;
CREATE INDEX "BroadcastTarget_inboxId_idx" ON "BroadcastTarget" ("inboxId");
CREATE INDEX "BroadcastTarget_flowId_idx" ON "BroadcastTarget" ("flowId");

-- BroadcastRecurrence
CREATE TABLE "BroadcastRecurrence" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "broadcastId" INTEGER NOT NULL REFERENCES "Broadcast" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "rule" TEXT NOT NULL CHECK (json_valid("rule")),
  "nextRunAt" INTEGER,
  "accountedThroughAt" INTEGER,
  "occurrenceCount" INTEGER NOT NULL DEFAULT 0,
  "lastOccurrenceAt" INTEGER,
  "pausedAt" INTEGER,
  "endedAt" INTEGER
) STRICT;
CREATE UNIQUE INDEX "BroadcastRecurrence_broadcastId_key" ON "BroadcastRecurrence" ("broadcastId");
CREATE INDEX "BroadcastRecurrence_workspaceId_idx" ON "BroadcastRecurrence" ("workspaceId");
CREATE INDEX "BroadcastRecurrence_due_idx" ON "BroadcastRecurrence" ("nextRunAt", "id") WHERE "nextRunAt" IS NOT NULL;

-- BroadcastRecurrenceOccurrence
CREATE TABLE "BroadcastRecurrenceOccurrence" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "recurrenceId" INTEGER NOT NULL REFERENCES "BroadcastRecurrence" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "localSlot" TEXT NOT NULL,
  "occurrenceAt" INTEGER NOT NULL,
  "outcome" TEXT NOT NULL CHECK ("outcome" IN ('created', 'skipped', 'missed')),
  "reason" TEXT CHECK ("reason" IN ('catchUpSuperseded', 'paused', 'draft', 'blockedOwner', 'planLimit', 'noTargets', 'undeliverable', 'excludedDate')),
  "childBroadcastId" INTEGER REFERENCES "Broadcast" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "materializedAt" INTEGER NOT NULL,
  "supersededCount" INTEGER NOT NULL DEFAULT 1
) STRICT;
CREATE UNIQUE INDEX "BroadcastRecurrenceOccurrence_recurrenceId_localSlot_occurrenceAt_key" ON "BroadcastRecurrenceOccurrence" ("recurrenceId", "localSlot", "occurrenceAt");
CREATE INDEX "BroadcastRecurrenceOccurrence_recurrenceId_occurrenceAt_idx" ON "BroadcastRecurrenceOccurrence" ("recurrenceId", "occurrenceAt" DESC);
CREATE INDEX "BroadcastRecurrenceOccurrence_workspaceId_idx" ON "BroadcastRecurrenceOccurrence" ("workspaceId");
CREATE INDEX "BroadcastRecurrenceOccurrence_childBroadcastId_idx" ON "BroadcastRecurrenceOccurrence" ("childBroadcastId");

-- ContactOnBroadcast
-- Postgres: HASH(broadcastId) x64. SQLite: one table, same PK and PK name (DECISIONS P2.2).
-- "isRead": STORED generated column, same expression (booleans -> 0/1).
CREATE TABLE "ContactOnBroadcast" (
  "broadcastId" INTEGER NOT NULL REFERENCES "Broadcast" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactInboxId" INTEGER NOT NULL REFERENCES "ContactInbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "conversationId" INTEGER NOT NULL REFERENCES "Conversation" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "sent" INTEGER NOT NULL DEFAULT 0 CHECK ("sent" IN (0, 1)),
  "seenAt" INTEGER,
  "deliveredAt" INTEGER,
  "clickedAt" INTEGER,
  "failedAt" INTEGER,
  "errorContent" TEXT,
  "isRead" INTEGER GENERATED ALWAYS AS (CASE WHEN "seenAt" IS NULL THEN 0 WHEN "deliveredAt" IS NULL THEN 0 ELSE "seenAt" >= "deliveredAt" END) STORED,
  CONSTRAINT "ContactsOnBroadcast_pkey" PRIMARY KEY ("broadcastId", "contactId")
) STRICT;
CREATE INDEX "idx_contact_on_broadcast_contact_id" ON "ContactOnBroadcast" ("contactId");
CREATE INDEX "ContactOnBroadcast_conversationId_idx" ON "ContactOnBroadcast" ("conversationId");
CREATE INDEX "ContactOnBroadcast_contactInboxId_idx" ON "ContactOnBroadcast" ("contactInboxId");
CREATE INDEX "idx_contact_on_broadcast_is_read" ON "ContactOnBroadcast" ("isRead");
CREATE INDEX "ContactOnBroadcast_unsent_order_idx" ON "ContactOnBroadcast" ("broadcastId", "contactInboxId") WHERE "sent" = 0 AND "failedAt" IS NULL;

-- Sequence
CREATE TABLE "Sequence" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "folderId" INTEGER REFERENCES "Folder" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "active" INTEGER NOT NULL DEFAULT 1 CHECK ("active" IN (0, 1)),
  "subscribers" INTEGER NOT NULL DEFAULT 0,
  "messages" INTEGER NOT NULL DEFAULT 0,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "Sequence_folderId_idx" ON "Sequence" ("folderId");
CREATE UNIQUE INDEX "Sequence_workspaceId_name_key" ON "Sequence" ("workspaceId", "name");

-- SequenceStep
CREATE TABLE "SequenceStep" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "order" INTEGER NOT NULL,
  "delayDays" INTEGER NOT NULL,
  "delayMinutes" INTEGER NOT NULL DEFAULT 0,
  "delayUnit" TEXT,
  "specificDateTime" INTEGER,
  "isActive" INTEGER NOT NULL DEFAULT 1 CHECK ("isActive" IN (0, 1)),
  "anytime" INTEGER NOT NULL DEFAULT 1 CHECK ("anytime" IN (0, 1)),
  "sendTimeStart" TEXT,
  "sendTimeEnd" TEXT,
  "sendDays" TEXT DEFAULT '["monday","tuesday","wednesday","thursday","friday","saturday","sunday"]',
  "flowId" INTEGER REFERENCES "Flow" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "sequenceId" INTEGER NOT NULL REFERENCES "Sequence" ("id") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE INDEX "SequenceStep_sequenceId_idx" ON "SequenceStep" ("sequenceId");
CREATE INDEX "SequenceStep_flowId_idx" ON "SequenceStep" ("flowId");

-- SequenceDispatch
-- Postgres: HASH(workspaceId) x64, PK (id, workspaceId). SQLite: one table, PK (id).
-- "isRead": STORED generated column, same expression. "SequenceDispatch_id_idx" is kept for parity (redundant with the rowid).
CREATE TABLE "SequenceDispatch" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "runAtMs" INTEGER NOT NULL,
  "bucket" INTEGER NOT NULL DEFAULT 0,
  "status" TEXT NOT NULL DEFAULT 'pending',
  "idempotencyKey" TEXT NOT NULL,
  "attempt" INTEGER NOT NULL DEFAULT 0,
  "lastError" TEXT,
  "lockedAt" INTEGER,
  "lockOwner" TEXT,
  "completedAt" INTEGER,
  "deliveredAt" INTEGER,
  "seenAt" INTEGER,
  "clickedAt" INTEGER,
  "failedAt" INTEGER,
  "errorContent" TEXT,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "sequenceId" INTEGER NOT NULL REFERENCES "Sequence" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactInboxId" INTEGER NOT NULL REFERENCES "ContactInbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "stepId" INTEGER NOT NULL REFERENCES "SequenceStep" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "enrollmentId" INTEGER NOT NULL,
  "isRead" INTEGER GENERATED ALWAYS AS (CASE WHEN "seenAt" IS NULL THEN 0 WHEN "deliveredAt" IS NULL THEN 0 ELSE "seenAt" >= "deliveredAt" END) STORED,
  CONSTRAINT "SequenceDispatch_enrollment_workspace_fkey" FOREIGN KEY ("enrollmentId", "workspaceId") REFERENCES "ContactOnSequence" ("id", "workspaceId") ON DELETE CASCADE ON UPDATE CASCADE
) STRICT;
CREATE UNIQUE INDEX "SequenceDispatch_idempotencyKey_key" ON "SequenceDispatch" ("idempotencyKey", "workspaceId");
CREATE INDEX "SequenceDispatch_id_idx" ON "SequenceDispatch" ("id");
CREATE INDEX "SequenceDispatch_pending_runAtMs_idx" ON "SequenceDispatch" ("runAtMs") WHERE "status" = 'pending';
CREATE INDEX "SequenceDispatch_pending_bucket_runAtMs_idx" ON "SequenceDispatch" ("bucket", "runAtMs") WHERE "status" = 'pending';
CREATE INDEX "SequenceDispatch_status_runAtMs_idx" ON "SequenceDispatch" ("status", "runAtMs");
CREATE INDEX "SequenceDispatch_workspaceId_status_runAtMs_idx" ON "SequenceDispatch" ("workspaceId", "status", "runAtMs");
CREATE INDEX "SequenceDispatch_enrollmentId_workspaceId_idx" ON "SequenceDispatch" ("enrollmentId", "workspaceId");
CREATE INDEX "SequenceDispatch_workspace_sequence_step_id_idx" ON "SequenceDispatch" ("workspaceId", "sequenceId", "stepId", "id");
CREATE INDEX "SequenceDispatch_bucket_status_runAtMs_idx" ON "SequenceDispatch" ("bucket", "status", "runAtMs");
CREATE INDEX "SequenceDispatch_terminal_updatedAt_idx" ON "SequenceDispatch" ("updatedAt") WHERE "status" IN ('completed', 'failed', 'canceled');
CREATE INDEX "SequenceDispatch_contactId_idx" ON "SequenceDispatch" ("contactId");
CREATE INDEX "SequenceDispatch_contactInboxId_idx" ON "SequenceDispatch" ("contactInboxId");
CREATE INDEX "SequenceDispatch_sequenceId_idx" ON "SequenceDispatch" ("sequenceId");
CREATE INDEX "SequenceDispatch_stepId_idx" ON "SequenceDispatch" ("stepId");

-- ContactOnSequence
-- Postgres: HASH(workspaceId) x64, PK (id, workspaceId). SQLite: one table, PK (id).
-- SQLite-only UNIQUE (id, workspaceId): parent key of the composite FK from "SequenceDispatch" (SQLite needs a
-- UNIQUE index on the parent columns).
CREATE TABLE "ContactOnSequence" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "enrolledAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "completedAt" INTEGER,
  "currentStep" INTEGER NOT NULL DEFAULT 0,
  "status" TEXT,
  "nextRunAt" INTEGER,
  "lastStepId" INTEGER,
  "nextStepId" INTEGER,
  "lockedAt" INTEGER,
  "lockOwner" TEXT,
  "lastError" TEXT,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "sequenceId" INTEGER NOT NULL REFERENCES "Sequence" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  CONSTRAINT "ContactOnSequence_id_workspaceId_key" UNIQUE ("id", "workspaceId")
) STRICT;
CREATE INDEX "ContactsOnSequence_sequenceId_idx" ON "ContactOnSequence" ("sequenceId");
CREATE INDEX "ContactsOnSequence_contactId_idx" ON "ContactOnSequence" ("contactId");
CREATE INDEX "ContactsOnSequence_workspaceId_idx" ON "ContactOnSequence" ("workspaceId");
CREATE INDEX "ContactsOnSequence_status_nextRunAt_idx" ON "ContactOnSequence" ("status", "nextRunAt");
CREATE INDEX "ContactsOnSequence_workspaceId_status_nextRunAt_idx" ON "ContactOnSequence" ("workspaceId", "status", "nextRunAt");
CREATE UNIQUE INDEX "ContactsOnSequence_contactId_sequenceId_workspaceId_key" ON "ContactOnSequence" ("contactId", "sequenceId", "workspaceId");

-- ContactOnSmartDelay
-- "appointmentId" (FK to "Appointment", P2.5) and "ContactOnSmartDelay_appointmentId_idx" are added by 0003_rest.sql.
CREATE TABLE "ContactOnSmartDelay" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "flowId" INTEGER NOT NULL,
  "flowVersionId" INTEGER,
  "contactInboxId" INTEGER NOT NULL,
  "conversationId" INTEGER NOT NULL REFERENCES "Conversation" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "nodeId" TEXT,
  "stepId" TEXT,
  "metadata" TEXT CHECK ("metadata" IS NULL OR json_valid("metadata")),
  "type" TEXT NOT NULL CHECK ("type" IN ('waitNode', 'followUp', 'quickReplyFollowUp', 'stepInputTimeout')),
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "triggerAt" INTEGER NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'pending' CHECK ("status" IN ('pending', 'scheduled', 'completed', 'failed', 'canceled'))
) STRICT;
CREATE INDEX "ContactOnSmartDelay_workspaceId_flowId_contactInboxId_stepId_idx" ON "ContactOnSmartDelay" ("workspaceId", "flowId", "contactInboxId", "stepId");
CREATE INDEX "ContactOnSmartDelay_status_triggerAt_idx" ON "ContactOnSmartDelay" ("status", "triggerAt");
CREATE INDEX "ContactOnSmartDelay_conversationId_idx" ON "ContactOnSmartDelay" ("conversationId");
CREATE UNIQUE INDEX "ContactOnSmartDelay_followUp_active_key" ON "ContactOnSmartDelay" ("workspaceId", "contactInboxId", "flowId", "stepId") WHERE "status" NOT IN ('completed', 'failed', 'canceled') AND "type" = 'followUp';
CREATE UNIQUE INDEX "ContactOnSmartDelay_quickReplyFollowUp_active_key" ON "ContactOnSmartDelay" ("workspaceId", "contactInboxId", "flowId", "nodeId") WHERE "status" NOT IN ('completed', 'failed', 'canceled') AND "type" = 'quickReplyFollowUp';
CREATE UNIQUE INDEX "ContactOnSmartDelay_stepInputTimeout_active_key" ON "ContactOnSmartDelay" ("workspaceId", "conversationId") WHERE "status" NOT IN ('completed', 'failed', 'canceled') AND "type" = 'stepInputTimeout';

-- Template
-- BEFORE DELETE trigger "Template_stamp_installations_source_deleted": end of this file.
CREATE TABLE "Template" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "tenantId" INTEGER NOT NULL REFERENCES "Tenant" ("id") ON DELETE RESTRICT ON UPDATE CASCADE,
  "name" TEXT NOT NULL,
  "description" TEXT,
  "imageUrl" TEXT,
  "publisherName" TEXT,
  "youtubeVideoId" TEXT,
  "testLink" TEXT,
  "selection" TEXT NOT NULL CHECK (json_valid("selection")),
  "payload" TEXT NOT NULL CHECK (json_valid("payload")),
  "categoryCounts" TEXT NOT NULL CHECK (json_valid("categoryCounts")),
  "formatVersion" INTEGER NOT NULL,
  "shareToken" TEXT NOT NULL,
  "shareEnabled" INTEGER NOT NULL DEFAULT 0 CHECK ("shareEnabled" IN (0, 1)),
  "shareExpiresAt" INTEGER,
  "defaultPermissions" TEXT NOT NULL CHECK (json_valid("defaultPermissions")),
  "createInstallFolder" INTEGER NOT NULL DEFAULT 0 CHECK ("createInstallFolder" IN (0, 1)),
  "defaultAutoUpdate" INTEGER NOT NULL DEFAULT 0 CHECK ("defaultAutoUpdate" IN (0, 1)),
  "installCount" INTEGER NOT NULL DEFAULT 0,
  "version" INTEGER NOT NULL DEFAULT 1,
  "payloadHash" TEXT,
  "createdBy" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "deletedAt" INTEGER
) STRICT;
CREATE INDEX "Template_workspaceId_idx" ON "Template" ("workspaceId");
CREATE INDEX "Template_tenantId_idx" ON "Template" ("tenantId");
CREATE UNIQUE INDEX "Template_shareToken_key" ON "Template" ("shareToken");

-- TemplateVersion
CREATE TABLE "TemplateVersion" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "templateId" INTEGER NOT NULL REFERENCES "Template" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "version" INTEGER NOT NULL,
  "payload" TEXT NOT NULL CHECK (json_valid("payload")),
  "payloadHash" TEXT NOT NULL,
  "formatVersion" INTEGER NOT NULL,
  "changes" TEXT CHECK ("changes" IS NULL OR json_valid("changes"))
) STRICT;
CREATE UNIQUE INDEX "TemplateVersion_templateId_version_key" ON "TemplateVersion" ("templateId", "version");
CREATE INDEX "TemplateVersion_workspaceId_idx" ON "TemplateVersion" ("workspaceId");

-- TemplateInstallation
-- Partial indexes with column-to-column comparisons are copied as-is.
CREATE TABLE "TemplateInstallation" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "templateId" INTEGER REFERENCES "Template" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "templateName" TEXT NOT NULL,
  "sourceWorkspaceId" INTEGER NOT NULL,
  "formatVersion" INTEGER NOT NULL,
  "status" TEXT NOT NULL DEFAULT 'pending' CHECK ("status" IN ('pending', 'installing', 'completed', 'partial', 'failed', 'uninstalled')),
  "permissions" TEXT NOT NULL CHECK (json_valid("permissions")),
  "warnings" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("warnings")),
  "warningCount" INTEGER NOT NULL DEFAULT 0,
  "errorMessage" TEXT,
  "resourceCount" INTEGER NOT NULL DEFAULT 0,
  "installFolderId" INTEGER REFERENCES "Folder" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "autoUpdate" INTEGER NOT NULL DEFAULT 0 CHECK ("autoUpdate" IN (0, 1)),
  "sourceUpdatedAt" INTEGER,
  "installedBy" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "installedByKind" TEXT NOT NULL DEFAULT 'apiToken',
  "installSource" TEXT NOT NULL DEFAULT 'manual' CHECK ("installSource" IN ('manual', 'tenantDefault')),
  "completedAt" INTEGER,
  "stagingGeneration" INTEGER NOT NULL DEFAULT 0,
  "stagedCleanedGeneration" INTEGER NOT NULL DEFAULT -1,
  "stagedCleanupClaimedAt" INTEGER,
  "pendingPublishEffects" TEXT CHECK ("pendingPublishEffects" IS NULL OR json_valid("pendingPublishEffects")),
  "claimToken" TEXT,
  "claimedAt" INTEGER,
  "attempts" INTEGER NOT NULL DEFAULT 0,
  "errorCode" TEXT,
  "sourceTemplateId" INTEGER,
  "successfulInstallEvents" INTEGER NOT NULL DEFAULT 0,
  "sourceDeletedAt" INTEGER,
  "inboxBindings" TEXT CHECK ("inboxBindings" IS NULL OR json_valid("inboxBindings")),
  "publisherRef" TEXT NOT NULL DEFAULT '',
  "publisherUninstallGrant" INTEGER NOT NULL DEFAULT 0,
  "uninstalledAt" INTEGER,
  "uninstalledByKind" TEXT,
  "uninstallCounts" TEXT CHECK ("uninstallCounts" IS NULL OR json_valid("uninstallCounts")),
  "uninstallState" TEXT NOT NULL DEFAULT 'none' CHECK ("uninstallState" IN ('none', 'pending', 'running', 'done', 'partial', 'failed', 'skipped')),
  "uninstallRequestedByKind" TEXT,
  "uninstallRequestedBy" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "uninstallRequestedAt" INTEGER,
  "uninstallAttempt" INTEGER NOT NULL DEFAULT 0,
  "uninstallClaimToken" TEXT,
  "uninstallClaimedAt" INTEGER,
  "uninstallErrorCode" TEXT,
  "uninstallOperationId" INTEGER REFERENCES "TemplateUninstallOperation" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "sourceVersion" INTEGER NOT NULL DEFAULT 1,
  "lastSyncedAt" INTEGER
) STRICT;
CREATE INDEX "TemplateInstallation_workspaceId_idx" ON "TemplateInstallation" ("workspaceId");
CREATE INDEX "TemplateInstallation_templateId_idx" ON "TemplateInstallation" ("templateId");
CREATE INDEX "TemplateInstallation_workspaceId_status_idx" ON "TemplateInstallation" ("workspaceId", "status");
CREATE UNIQUE INDEX "TemplateInstallation_workspaceId_sourceTemplateId_active_key" ON "TemplateInstallation" ("workspaceId", "sourceTemplateId") WHERE "status" IN ('pending', 'installing');
CREATE INDEX "TemplateInstallation_sourceTemplateId_idx" ON "TemplateInstallation" ("sourceTemplateId");
CREATE INDEX "TemplateInstallation_autoUpdate_sourceTemplateId_sourceVersion_idx" ON "TemplateInstallation" ("sourceTemplateId", "sourceVersion") WHERE "autoUpdate" = 1 AND "status" IN ('completed', 'partial') AND "sourceDeletedAt" IS NULL;
CREATE INDEX "TemplateInstallation_templateId_publisherRef_idx" ON "TemplateInstallation" ("templateId", "publisherRef");
CREATE INDEX "TemplateInstallation_uninstallRequestedBy_idx" ON "TemplateInstallation" ("uninstallRequestedBy");
CREATE INDEX "TemplateInstallation_uninstallOperationId_idx" ON "TemplateInstallation" ("uninstallOperationId");
CREATE INDEX "TemplateInstallation_uninstallState_claimedAt_idx" ON "TemplateInstallation" ("uninstallState", "uninstallClaimedAt") WHERE "uninstallState" IN ('pending', 'running');
CREATE INDEX "TemplateInstallation_pendingPublishEffects_updatedAt_idx" ON "TemplateInstallation" ("updatedAt") WHERE "pendingPublishEffects" IS NOT NULL;
CREATE INDEX "TemplateInstallation_stagedCleanupOwed_updatedAt_idx" ON "TemplateInstallation" ("updatedAt") WHERE (("status" = 'failed' AND "stagedCleanedGeneration" < "stagingGeneration") OR ("status" <> 'failed' AND "stagedCleanedGeneration" < "stagingGeneration" - 1));
CREATE INDEX "TemplateInstallation_status_claimedAt_idx" ON "TemplateInstallation" ("status", "claimedAt");

-- TemplateInstallationSync
CREATE TABLE "TemplateInstallationSync" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "installationId" INTEGER NOT NULL REFERENCES "TemplateInstallation" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "status" TEXT NOT NULL DEFAULT 'pending' CHECK ("status" IN ('pending', 'running', 'completed', 'partial', 'failed', 'skipped', 'cancelled')),
  "trigger" TEXT NOT NULL CHECK ("trigger" IN ('autoUpdate', 'manual', 'reinstall')),
  "requestedVersion" INTEGER NOT NULL,
  "idempotencyKey" TEXT,
  "requestFingerprint" TEXT,
  "requestedBy" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "requestedByKind" TEXT NOT NULL DEFAULT 'apiToken',
  "claimToken" TEXT,
  "claimedAt" INTEGER,
  "attempts" INTEGER NOT NULL DEFAULT 0,
  "errorCode" TEXT,
  "fromVersion" INTEGER,
  "appliedVersion" INTEGER,
  "appliedHash" TEXT,
  "counts" TEXT CHECK ("counts" IS NULL OR json_valid("counts")),
  "warnings" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("warnings")),
  "warningSummary" TEXT CHECK ("warningSummary" IS NULL OR json_valid("warningSummary")),
  "startedAt" INTEGER,
  "finishedAt" INTEGER
) STRICT;
CREATE INDEX "TemplateInstallationSync_workspaceId_idx" ON "TemplateInstallationSync" ("workspaceId");
CREATE INDEX "TemplateInstallationSync_requestedBy_idx" ON "TemplateInstallationSync" ("requestedBy");
CREATE INDEX "TemplateInstallationSync_installationId_idx" ON "TemplateInstallationSync" ("installationId");
CREATE UNIQUE INDEX "TemplateInstallationSync_auto_active_key" ON "TemplateInstallationSync" ("installationId", "requestedVersion") WHERE "trigger" = 'autoUpdate' AND "status" IN ('pending', 'running');
CREATE UNIQUE INDEX "TemplateInstallationSync_idempotency_key" ON "TemplateInstallationSync" ("installationId", "idempotencyKey") WHERE "idempotencyKey" IS NOT NULL;
CREATE INDEX "TemplateInstallationSync_status_claimedAt_idx" ON "TemplateInstallationSync" ("status", "claimedAt");

-- TemplateInstalledResource
CREATE TABLE "TemplateInstalledResource" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "installationId" INTEGER NOT NULL REFERENCES "TemplateInstallation" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "category" TEXT NOT NULL CHECK ("category" IN ('flows', 'products', 'aiFunctions', 'aiAgents', 'calendars', 'webchats', 'keywords', 'entryPointLinks', 'triggers', 'fbCommentAutomations', 'settings', 'sequences', 'aiMcpServers', 'aiFiles', 'questionnaires', 'couponTopics', 'pipelines', 'customFields', 'tags', 'productCategories')),
  "resourceKind" TEXT NOT NULL,
  "resourceId" INTEGER NOT NULL,
  "sourceResourceId" TEXT NOT NULL,
  "wasExisting" INTEGER NOT NULL DEFAULT 0 CHECK ("wasExisting" IN (0, 1)),
  "allowEdit" INTEGER NOT NULL DEFAULT 1 CHECK ("allowEdit" IN (0, 1)),
  "allowDelete" INTEGER NOT NULL DEFAULT 1 CHECK ("allowDelete" IN (0, 1)),
  "detachedAt" INTEGER,
  "syncedHash" TEXT,
  "syncedVersion" INTEGER,
  "uninstallOutcome" TEXT,
  "uninstallEffects" TEXT CHECK ("uninstallEffects" IS NULL OR json_valid("uninstallEffects")),
  "uninstallEffectsAttempts" INTEGER NOT NULL DEFAULT 0,
  "uninstallFrozenFrom" TEXT CHECK ("uninstallFrozenFrom" IS NULL OR json_valid("uninstallFrozenFrom")),
  "uninstallDrainStartedAt" INTEGER
) STRICT;
CREATE INDEX "TemplateInstalledResource_active_lookup_idx" ON "TemplateInstalledResource" ("workspaceId", "resourceKind", "resourceId") WHERE "wasExisting" = 0 AND "detachedAt" IS NULL;
CREATE UNIQUE INDEX "TemplateInstalledResource_active_source_key" ON "TemplateInstalledResource" ("installationId", "resourceKind", "sourceResourceId") WHERE "detachedAt" IS NULL;
CREATE INDEX "TemplateInstalledResource_pending_effects_idx" ON "TemplateInstalledResource" ("uninstallEffectsAttempts", "id") WHERE "uninstallEffects" IS NOT NULL;
CREATE INDEX "TemplateInstalledResource_installationId_idx" ON "TemplateInstalledResource" ("installationId");
CREATE INDEX "TemplateInstalledResource_workspaceId_resourceKind_resourceId_idx" ON "TemplateInstalledResource" ("workspaceId", "resourceKind", "resourceId");

-- TemplateResourceBackup
CREATE TABLE "TemplateResourceBackup" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "installationId" INTEGER NOT NULL REFERENCES "TemplateInstallation" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "syncId" INTEGER NOT NULL REFERENCES "TemplateInstallationSync" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "installedResourceId" INTEGER NOT NULL REFERENCES "TemplateInstalledResource" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "resourceKind" TEXT NOT NULL,
  "resourceId" INTEGER NOT NULL,
  "formatVersion" INTEGER NOT NULL,
  "content" TEXT NOT NULL CHECK (json_valid("content")),
  "byteSize" INTEGER NOT NULL
) STRICT;
CREATE INDEX "TemplateResourceBackup_workspaceId_idx" ON "TemplateResourceBackup" ("workspaceId");
CREATE INDEX "TemplateResourceBackup_installationId_idx" ON "TemplateResourceBackup" ("installationId");
CREATE INDEX "TemplateResourceBackup_syncId_idx" ON "TemplateResourceBackup" ("syncId");
CREATE INDEX "TemplateResourceBackup_installedResourceId_idx" ON "TemplateResourceBackup" ("installedResourceId");

-- TemplateUninstallOperation
CREATE TABLE "TemplateUninstallOperation" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "templateId" INTEGER REFERENCES "Template" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "publisherRef" TEXT NOT NULL,
  "requestedByKind" TEXT NOT NULL,
  "requestedBy" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "state" TEXT NOT NULL DEFAULT 'pending' CHECK ("state" IN ('none', 'pending', 'running', 'done', 'partial', 'failed', 'skipped')),
  "counts" TEXT CHECK ("counts" IS NULL OR json_valid("counts")),
  "auditLogId" TEXT NOT NULL,
  "noticeDeliveredAt" INTEGER,
  "finishedAt" INTEGER
) STRICT;
CREATE INDEX "TemplateUninstallOperation_workspaceId_idx" ON "TemplateUninstallOperation" ("workspaceId");
CREATE INDEX "TemplateUninstallOperation_templateId_idx" ON "TemplateUninstallOperation" ("templateId");
CREATE INDEX "TemplateUninstallOperation_requestedBy_idx" ON "TemplateUninstallOperation" ("requestedBy");
CREATE INDEX "TemplateUninstallOperation_notice_idx" ON "TemplateUninstallOperation" ("state", "noticeDeliveredAt");

-- TenantDefaultTemplate
-- PK "tenantId" (also the FK to "Tenant") -> INTEGER PRIMARY KEY.
CREATE TABLE "TenantDefaultTemplate" (
  "tenantId" INTEGER PRIMARY KEY NOT NULL REFERENCES "Tenant" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "templateId" INTEGER NOT NULL REFERENCES "Template" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "keyHash" TEXT NOT NULL,
  "encryptedKey" TEXT NOT NULL CHECK (json_valid("encryptedKey")),
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER))
) STRICT;
CREATE INDEX "TenantDefaultTemplate_templateId_idx" ON "TenantDefaultTemplate" ("templateId");

-- TenantDefaultTemplateEvent
CREATE TABLE "TenantDefaultTemplateEvent" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "tenantId" INTEGER NOT NULL REFERENCES "Tenant" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "action" TEXT NOT NULL CHECK ("action" IN ('set', 'cleared')),
  "templateId" INTEGER NOT NULL,
  "actorUserId" INTEGER REFERENCES "User" ("id") ON DELETE SET NULL ON UPDATE CASCADE
) STRICT;
CREATE INDEX "TenantDefaultTemplateEvent_tenantId_createdAt_idx" ON "TenantDefaultTemplateEvent" ("tenantId", "createdAt" DESC);
CREATE INDEX "TenantDefaultTemplateEvent_actorUserId_idx" ON "TenantDefaultTemplateEvent" ("actorUserId");

-- WhatsappMessageTemplate
CREATE TABLE "WhatsappMessageTemplate" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "integrationWhatsappId" INTEGER NOT NULL REFERENCES "IntegrationWhatsapp" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "sourceId" TEXT NOT NULL,
  "language" TEXT NOT NULL,
  "category" TEXT NOT NULL,
  "status" TEXT NOT NULL,
  "components" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("components"))
) STRICT;
CREATE UNIQUE INDEX "WhatsappMessageTemplate_integrationWhatsappId_sourceId_key" ON "WhatsappMessageTemplate" ("integrationWhatsappId", "sourceId");

-- MessengerMessageTemplate
CREATE TABLE "MessengerMessageTemplate" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "integrationMessengerId" INTEGER NOT NULL REFERENCES "IntegrationMessenger" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "sourceId" TEXT NOT NULL,
  "language" TEXT NOT NULL,
  "category" TEXT NOT NULL,
  "status" TEXT NOT NULL,
  "parameterFormat" TEXT NOT NULL DEFAULT 'POSITIONAL',
  "components" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("components"))
) STRICT;
CREATE UNIQUE INDEX "MessengerMessageTemplate_integrationMessengerId_sourceId_key" ON "MessengerMessageTemplate" ("integrationMessengerId", "sourceId");

-- WhatsappFlow
CREATE TABLE "WhatsappFlow" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "integrationWhatsappId" INTEGER NOT NULL REFERENCES "IntegrationWhatsapp" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "sourceId" TEXT NOT NULL,
  "status" TEXT NOT NULL,
  "categories" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("categories")),
  "validationErrors" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("validationErrors")),
  "completedCount" INTEGER NOT NULL DEFAULT 0,
  "screens" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("screens"))
) STRICT;
CREATE UNIQUE INDEX "WhatsappFlow_integrationWhatsappId_sourceId_key" ON "WhatsappFlow" ("integrationWhatsappId", "sourceId");

-- CommentAutomation
-- "excludeKeywords" default ARRAY[] -> '[]'.
CREATE TABLE "CommentAutomation" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "folderId" INTEGER REFERENCES "Folder" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "type" TEXT NOT NULL DEFAULT 'messenger' CHECK ("type" IN ('messenger', 'instagram', 'instagramFacebook', 'threads', 'tiktok')),
  "inboxId" INTEGER REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "isActive" INTEGER NOT NULL DEFAULT 1 CHECK ("isActive" IN (0, 1)),
  "startTime" TEXT,
  "endTime" TEXT,
  "repliesCount" INTEGER NOT NULL DEFAULT 0,
  "sentCount" INTEGER NOT NULL DEFAULT 0,
  "deliveredCount" INTEGER NOT NULL DEFAULT 0,
  "seenCount" INTEGER NOT NULL DEFAULT 0,
  "clickedCount" INTEGER NOT NULL DEFAULT 0,
  "failedCount" INTEGER NOT NULL DEFAULT 0,
  "missedCount" INTEGER NOT NULL DEFAULT 0,
  "moderatedCount" INTEGER NOT NULL DEFAULT 0,
  "post" TEXT NOT NULL DEFAULT '{"type":"all","value":[]}' CHECK (json_valid("post")),
  "privateReply" TEXT NOT NULL DEFAULT '{"type":"text","value":""}' CHECK (json_valid("privateReply")),
  "publicReply" TEXT NOT NULL DEFAULT '{"type":"none","value":null}' CHECK (json_valid("publicReply")),
  "includeKeywords" TEXT NOT NULL DEFAULT '{"type":"all","value":[]}' CHECK (json_valid("includeKeywords")),
  "excludeKeywords" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("excludeKeywords") AND json_type("excludeKeywords") = 'array'),
  "excludeKeywordsType" TEXT NOT NULL DEFAULT 'contain' CHECK ("excludeKeywordsType" IN ('equal', 'contain')),
  "options" TEXT NOT NULL DEFAULT '{"replyToNewContactsOnly":false,"replyOncePerUserPerPost":false,"likeUserComment":false,"replyToUsersWhoCommentedOnOtherPosts":true,"ignoreCommentReplies":true,"trackUserTags":false}' CHECK (json_valid("options")),
  "hideComments" TEXT NOT NULL DEFAULT '{"all":false,"hasPhoneNumber":false,"hasImage":false,"hasVideo":false,"hasLink":false,"hasKeywords":false,"hasGif":false,"hasEmoji":false,"keywords":[],"showCommentsAfter":"none"}' CHECK (json_valid("hideComments")),
  "replyAfter" TEXT NOT NULL DEFAULT '{"type":"immediately","value":0}' CHECK (json_valid("replyAfter"))
) STRICT;
CREATE INDEX "CommentAutomation_workspaceId_idx" ON "CommentAutomation" ("workspaceId");
CREATE INDEX "CommentAutomation_folderId_idx" ON "CommentAutomation" ("folderId");
CREATE INDEX "CommentAutomation_inboxId_idx" ON "CommentAutomation" ("inboxId");

-- CommentAutomationEvent
-- Postgres index key "occurredAt" DESC NULLS FIRST: SQLite indexes cannot state NULLS FIRST; the column is NOT NULL, so DESC is the same order.
CREATE TABLE "CommentAutomationEvent" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "automationId" INTEGER NOT NULL REFERENCES "CommentAutomation" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "contactInboxId" INTEGER REFERENCES "ContactInbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "postId" TEXT NOT NULL,
  "commentId" TEXT NOT NULL,
  "commentText" TEXT,
  "replyChannel" TEXT NOT NULL CHECK ("replyChannel" IN ('public', 'private')),
  "replyType" TEXT NOT NULL CHECK ("replyType" IN ('AIAgent', 'text', 'flow', 'none')),
  "replyText" TEXT,
  "status" TEXT NOT NULL CHECK ("status" IN ('sent', 'failed')),
  "deliveredAt" INTEGER,
  "seenAt" INTEGER,
  "clickedAt" INTEGER,
  "failedAt" INTEGER,
  "errorDetail" TEXT,
  "httpCode" TEXT,
  "occurredAt" INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX "CommentAutomationEvent_dedup_idx" ON "CommentAutomationEvent" ("automationId", "commentId", "replyChannel");
CREATE INDEX "CommentAutomationEvent_automation_occurredAt_idx" ON "CommentAutomationEvent" ("workspaceId", "automationId", "occurredAt" DESC);
CREATE INDEX "CommentAutomationEvent_contactId_idx" ON "CommentAutomationEvent" ("contactId");
CREATE INDEX "CommentAutomationEvent_contactInboxId_idx" ON "CommentAutomationEvent" ("contactInboxId");
CREATE INDEX "CommentAutomationEvent_private_unseen_idx" ON "CommentAutomationEvent" ("contactInboxId") WHERE "replyChannel" = 'private' AND "deliveredAt" IS NOT NULL AND "seenAt" IS NULL;
CREATE INDEX "CommentAutomationEvent_failed_createdAt_idx" ON "CommentAutomationEvent" ("createdAt") WHERE "status" = 'failed';

-- CommentAutomationMiss
-- Index key "occurredAt" DESC NULLS FIRST -> DESC (NOT NULL column, same order).
CREATE TABLE "CommentAutomationMiss" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "automationId" INTEGER NOT NULL REFERENCES "CommentAutomation" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "contactInboxId" INTEGER REFERENCES "ContactInbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "postId" TEXT NOT NULL,
  "commentId" TEXT NOT NULL,
  "commentText" TEXT,
  "reason" TEXT NOT NULL CHECK ("reason" IN ('outsideSchedule', 'postNotMatched', 'commentIsReply', 'keywordsNotMatched', 'mentionCountNotMatched', 'noImageAttachment', 'contactNotNew', 'alreadyRepliedOnPost', 'engagedOnOtherPost', 'moderated')),
  "occurredAt" INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX "CommentAutomationMiss_dedup_idx" ON "CommentAutomationMiss" ("automationId", "commentId");
CREATE INDEX "CommentAutomationMiss_automation_occurredAt_idx" ON "CommentAutomationMiss" ("workspaceId", "automationId", "occurredAt" DESC);
CREATE INDEX "CommentAutomationMiss_contactId_idx" ON "CommentAutomationMiss" ("contactId");
CREATE INDEX "CommentAutomationMiss_contactInboxId_idx" ON "CommentAutomationMiss" ("contactInboxId");

-- CommentAutomationReply
CREATE TABLE "CommentAutomationReply" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "automationId" INTEGER NOT NULL REFERENCES "CommentAutomation" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "contactId" INTEGER NOT NULL REFERENCES "Contact" ("id") ON DELETE CASCADE ON UPDATE NO ACTION,
  "postId" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE NO ACTION
) STRICT;
CREATE UNIQUE INDEX "CommentAutomationReply_dedup_idx" ON "CommentAutomationReply" ("automationId", "contactId", "postId");
CREATE INDEX "CommentAutomationReply_contactId_idx" ON "CommentAutomationReply" ("contactId");
CREATE INDEX "CommentAutomationReply_workspaceId_idx" ON "CommentAutomationReply" ("workspaceId");

-- CommentModerationLog
-- Index key "occurredAt" DESC NULLS FIRST -> DESC (NOT NULL column, same order). "categories" default '{}' -> '[]'.
CREATE TABLE "CommentModerationLog" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "automationId" INTEGER NOT NULL REFERENCES "CommentAutomation" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "contactId" INTEGER REFERENCES "Contact" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "contactInboxId" INTEGER REFERENCES "ContactInbox" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "postId" TEXT NOT NULL,
  "commentId" TEXT NOT NULL,
  "commentText" TEXT,
  "source" TEXT NOT NULL CHECK ("source" IN ('rule', 'ai')),
  "rule" TEXT CHECK ("rule" IN ('all', 'phoneNumber', 'link', 'keyword', 'image', 'video', 'gif', 'emoji')),
  "matchedKeyword" TEXT,
  "categories" TEXT NOT NULL DEFAULT '[]' CHECK (json_valid("categories") AND json_type("categories") = 'array'),
  "reason" TEXT,
  "action" TEXT NOT NULL CHECK ("action" IN ('hide', 'delete')),
  "status" TEXT NOT NULL DEFAULT 'pending' CHECK ("status" IN ('pending', 'done', 'failed')),
  "errorDetail" TEXT,
  "provider" TEXT,
  "modelId" TEXT,
  "occurredAt" INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX "CommentModerationLog_dedup_idx" ON "CommentModerationLog" ("automationId", "commentId");
CREATE INDEX "CommentModerationLog_workspace_automation_occurredAt_idx" ON "CommentModerationLog" ("workspaceId", "automationId", "occurredAt" DESC);
CREATE INDEX "CommentModerationLog_contactId_idx" ON "CommentModerationLog" ("contactId") WHERE "contactId" IS NOT NULL;
CREATE INDEX "CommentModerationLog_contactInboxId_idx" ON "CommentModerationLog" ("contactInboxId") WHERE "contactInboxId" IS NOT NULL;

-- IgStoryAutomation
CREATE TABLE "IgStoryAutomation" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "name" TEXT NOT NULL,
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "folderId" INTEGER REFERENCES "Folder" ("id") ON DELETE SET NULL ON UPDATE CASCADE,
  "type" TEXT NOT NULL CHECK ("type" IN ('instagram', 'instagramFacebook')),
  "trigger" TEXT NOT NULL DEFAULT 'storyReply' CHECK ("trigger" IN ('storyReply', 'storyMention')),
  "inboxId" INTEGER REFERENCES "Inbox" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "isActive" INTEGER NOT NULL DEFAULT 1 CHECK ("isActive" IN (0, 1)),
  "startTime" TEXT,
  "endTime" TEXT,
  "repliesCount" INTEGER NOT NULL DEFAULT 0,
  "story" TEXT NOT NULL DEFAULT '{"type":"all","value":[]}' CHECK (json_valid("story")),
  "reply" TEXT NOT NULL DEFAULT '{"type":"none","value":null}' CHECK (json_valid("reply")),
  "includeKeywords" TEXT NOT NULL DEFAULT '{"type":"all","value":[]}' CHECK (json_valid("includeKeywords"))
) STRICT;
CREATE INDEX "IgStoryAutomation_workspaceId_idx" ON "IgStoryAutomation" ("workspaceId");
CREATE INDEX "IgStoryAutomation_folderId_idx" ON "IgStoryAutomation" ("folderId");
CREATE INDEX "IgStoryAutomation_inboxId_idx" ON "IgStoryAutomation" ("inboxId");

-- MessengerWelcomeMessageFlow
CREATE TABLE "MessengerWelcomeMessageFlow" (
  "id" INTEGER PRIMARY KEY NOT NULL,
  "createdAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "updatedAt" INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
  "workspaceId" INTEGER NOT NULL REFERENCES "Workspace" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "flowId" INTEGER NOT NULL,
  "integrationMessengerId" INTEGER NOT NULL REFERENCES "IntegrationMessenger" ("id") ON DELETE CASCADE ON UPDATE CASCADE,
  "pageId" TEXT NOT NULL,
  "metaFlowId" TEXT NOT NULL,
  "flowVersionId" INTEGER,
  "status" TEXT NOT NULL DEFAULT 'current'
) STRICT;
CREATE INDEX "MessengerWelcomeMessageFlow_workspaceId_flowId_idx" ON "MessengerWelcomeMessageFlow" ("workspaceId", "flowId");
CREATE INDEX "MessengerWelcomeMessageFlow_integrationMessengerId_idx" ON "MessengerWelcomeMessageFlow" ("integrationMessengerId");
CREATE UNIQUE INDEX "MessengerWelcomeMessageFlow_current_key" ON "MessengerWelcomeMessageFlow" ("flowId", "integrationMessengerId") WHERE "status" = 'current';

-- ===================================================================================
-- Trigger: port of "Template_stamp_installations_source_deleted" (function
-- template_stamp_installations_source_deleted, migration 20261103090100_template_install_core).
-- BEFORE DELETE, same body; Postgres now() -> unix ms. The FK action ("TemplateInstallation"."templateId"
-- ON DELETE SET NULL) runs after this trigger, as in Postgres.
-- ===================================================================================
CREATE TRIGGER "Template_stamp_installations_source_deleted" BEFORE DELETE ON "Template" BEGIN
  UPDATE "TemplateInstallation"
     SET "sourceDeletedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)
   WHERE "sourceTemplateId" = OLD."id"
     AND "sourceDeletedAt" IS NULL;
END;
