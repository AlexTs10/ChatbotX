// Postgres implementation of the P1.9 fixture extension (original stack).
// Writes the same logical rows the app writes; the row source is cited per
// method. Setup only; assertions stay black-box.
//
// Layout: one factory per section (A flows/folders, C ig/googleAds/inboxTeams;
// part B, inbound webhooks, needs no fixture), merged by `create()`.
//
// Encryption (where a section needs it): AES-256-GCM, 12-byte IV, 16-byte
// tag, hex fields, {v:1, kid, iv, text, tag} (packages/encryption/src/encryption.ts),
// key = ENCRYPTION_KEY of stacks/original/original.env.

import { createCipheriv, randomBytes } from "node:crypto"
import { readFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import pg from "pg"
import type {
  P109Fixtures,
  P109FlowsExt,
  P109IgExt,
} from "../../../src/fixtures/ext/p1-09.ts"

const here = path.dirname(fileURLToPath(import.meta.url))
const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
/** App processes use place ids 1-3; fixtures 15 (core), 8-12, 14; P1.9 uses 7. */
const PLACE_ID = 7n

let lastMs = 0n
let sequence = 0n
export const createId = (): string => {
  let now = BigInt(Date.now() - SNOWFLAKE_EPOCH_MS)
  if (now <= lastMs) {
    sequence += 1n
    if (sequence >= 1024n) {
      lastMs += 1n
      sequence = 0n
    }
    now = lastMs
  } else {
    lastMs = now
    sequence = BigInt(randomBytes(1)[0] ?? 0)
  }
  return ((now << 14n) | (PLACE_ID << 10n) | sequence).toString()
}

const readEnvFile = (): Record<string, string> => {
  const text = readFileSync(path.resolve(here, "../original.env"), "utf8")
  const values: Record<string, string> = {}
  for (const line of text.split("\n")) {
    const match = /^([A-Z0-9_]+)=(.*)$/.exec(line.trim())
    if (match?.[1]) {
      values[match[1]] = match[2] ?? ""
    }
  }
  return values
}

const env = readEnvFile()
const ENCRYPTION_KEY = Buffer.from(env.ENCRYPTION_KEY ?? "", "hex")
const ENCRYPTION_KEY_ID = env.ENCRYPTION_KEY_ID || "default"

/** encryptUtils.encryptObject of the app. */
export const encryptObject = (value: unknown) => {
  if (ENCRYPTION_KEY.length !== 32) {
    throw new Error("p1-09 fixtures: ENCRYPTION_KEY must be 32 bytes of hex")
  }
  const iv = randomBytes(12)
  const cipher = createCipheriv("aes-256-gcm", ENCRYPTION_KEY, iv, { authTagLength: 16 })
  const text = Buffer.concat([cipher.update(JSON.stringify(value), "utf8"), cipher.final()])
  return {
    v: 1,
    kid: ENCRYPTION_KEY_ID,
    iv: iv.toString("hex"),
    text: text.toString("hex"),
    tag: cipher.getAuthTag().toString("hex"),
  }
}

export type Db = {
  pool: pg.Pool
  transaction<T>(fn: (client: pg.PoolClient) => Promise<T>): Promise<T>
}

const openDb = (connectionString: string): Db => {
  const pool = new pg.Pool({ connectionString, max: 2, idleTimeoutMillis: 1_000, allowExitOnIdle: true })
  return {
    pool,
    async transaction(fn) {
      const client = await pool.connect()
      try {
        await client.query("BEGIN")
        const result = await fn(client)
        await client.query("COMMIT")
        return result
      } catch (error) {
        await client.query("ROLLBACK")
        throw error
      } finally {
        client.release()
      }
    },
  }
}

// ---------------------------------------------------------------------------
// Section A: flows + folders
// ---------------------------------------------------------------------------

const flowsSection = (db: Db): P109FlowsExt => ({
  // folderService.create (packages/business/src/folder/service.ts), top level:
  // parentId null, paths [].
  async createFolder(input) {
    const folderId = createId()
    await db.pool.query(
      `INSERT INTO "Folder" ("id", "name", "folderType", "parentId", "workspaceId", "paths")
       VALUES ($1, $2, $3, NULL, $4, '{}')`,
      [folderId, input.name, input.folderType, input.workspaceId],
    )
    return { folderId }
  },
})

// ---------------------------------------------------------------------------
// Section C: igComments, igStories, googleAds, inboxTeams
// ---------------------------------------------------------------------------

// Row sources (baseline 40e733cee), copied from the P1.5 / P1.7 fixtures:
// - Inbox: inboxService.create (packages/database/src/schema/inbox.ts).
// - IntegrationInstagram + Connection: the Instagram connect flow
//   (schema/integration-instagram.ts, schema/connection.ts); auth is plain
//   JSON (oauth2 value of integrations/instagram*/src/schema.ts).
// - WorkspaceMember (agent): workspace invitation accept
//   (schema/workspace-member.ts), default agent permissions.
// - Integration + IntegrationGoogleAds + Connection: the connection engine
//   (packages/business/src/connection/store-bindings.ts
//   makeWorkspaceIntegrationBinding, auth plain JSON) and the Google Ads setup
//   service (integration-google-ads/service.ts refreshSetup: conversion
//   customer, actions cache, sync time, setup error).
// - GoogleAdsSettings: googleAdsSettingsRepository.upsertSettings.
// - GoogleAdsConversionEvent: recordConversion + delivery
//   (packages/business/src/google-ads/record-conversion.ts, delivery.ts).

const AGENT_PERMISSIONS = {
  superAdmin: false,
  analytics: true,
  flows: true,
  contacts: true,
  onlyAssignedContacts: false,
  emailAndPhone: true,
  broadcast: true,
  ecommerce: true,
}
const MEMBER_NOTIFICATION_TYPES = { notifyAdmin: true, newMessageToHuman: true, newOrder: true }
const MEMBER_NOTIFICATION_CHANNELS = { messenger: true, email: true, telegram: true, browser: true }

const GOOGLE_ADS_STATUS_REASON: Record<string, string | null> = {
  connected: null,
  degraded: "verify_failed",
  needs_reauth: "token_revoked",
  paused: "manual",
  disconnected: "manual",
}

const igSection = (db: Db): P109IgExt => {
  const insertInbox = async (
    client: pg.PoolClient | pg.Pool,
    input: { workspaceId: string; name: string; channel: string; sourceId: string },
  ): Promise<string> => {
    const inboxId = createId()
    await client.query(
      `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status")
       VALUES ($1, $2, $3, $4, $5, 'connected')`,
      [inboxId, input.name, input.channel, input.sourceId, input.workspaceId],
    )
    return inboxId
  }

  return {
    async createInstagramChannel(input) {
      return db.transaction(async (client) => {
        const inboxId = await insertInbox(client, {
          workspaceId: input.workspaceId,
          name: input.name,
          channel: "instagram",
          sourceId: input.igId,
        })
        const integrationId = createId()
        const auth = {
          authType: "oauth2",
          clientId: "parity-client",
          clientSecret: "parity-secret",
          redirectUrl: "http://app/integrations/instagram/callback",
          version: "v23.0",
          tokens: { accessToken: `ig-token-${input.igId}` },
          metadata: { igId: input.igId, pageId: input.pageId, version: "v23.0" },
        }
        await client.query(
          `INSERT INTO "IntegrationInstagram"
             ("id", "auth", "igId", "pageId", "name", "username", "workspaceId", "inboxId", "type",
              "conversationStarters", "persistentMenus")
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, '{}'::jsonb[], '{}'::jsonb[])`,
          [
            integrationId,
            JSON.stringify(auth),
            input.igId,
            input.pageId,
            input.name,
            input.username,
            input.workspaceId,
            inboxId,
            input.type,
          ],
        )
        await client.query(
          `INSERT INTO "Connection"
             ("id", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId", "displayName", "status", "connectedAt")
           VALUES ($1, $2, $3, 'channel', 'instagram', $4, $5, $6, 'connected', now())`,
          [
            createId(),
            input.workspaceId,
            input.type === "facebook" ? "instagramFacebook" : "instagram",
            inboxId,
            input.igId,
            input.name,
          ],
        )
        return { integrationId, inboxId }
      })
    },

    async createInbox(input) {
      return { inboxId: await insertInbox(db.pool, input) }
    },

    async addWorkspaceMember(input) {
      const memberId = createId()
      await db.pool.query(
        `INSERT INTO "WorkspaceMember"
           ("id", "workspaceId", "userId", "role", "permissions", "notificationTypes", "notificationChannels")
         VALUES ($1, $2, $3, 'agent', $4, $5, $6)`,
        [
          memberId,
          input.workspaceId,
          input.userId,
          JSON.stringify(AGENT_PERMISSIONS),
          JSON.stringify(MEMBER_NOTIFICATION_TYPES),
          JSON.stringify(MEMBER_NOTIFICATION_CHANNELS),
        ],
      )
      return { memberId }
    },

    async createGoogleAdsConnection(input) {
      return db.transaction(async (client) => {
        const integrationId = createId()
        const integrationGoogleAdsId = createId()
        await client.query(
          `INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES ($1, $2, 'googleAds')`,
          [integrationId, input.workspaceId],
        )
        const auth = {
          authType: "oauth2",
          clientId: "parity-google-ads-client",
          clientSecret: "parity-google-ads-secret",
          redirectUrl: "http://app/integrations/googleAds/callback",
          tokens: {
            accessToken: "parity-google-ads-access",
            refreshToken: "parity-google-ads-refresh",
            expiresAt: "2099-01-01T00:00:00.000Z",
          },
          metadata: input.uploadMethod ? { uploadMethod: input.uploadMethod } : {},
        }
        await client.query(
          `INSERT INTO "IntegrationGoogleAds"
             ("id", "workspaceId", "integrationId", "auth", "customerId", "descriptiveName", "currencyCode",
              "conversionCustomerId", "acceptedCustomerDataTerms", "conversionActions",
              "conversionActionsSyncedAt", "setupError", "setupErrorAt")
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)`,
          [
            integrationGoogleAdsId,
            input.workspaceId,
            integrationId,
            JSON.stringify(auth),
            input.customerId,
            input.descriptiveName,
            input.currencyCode,
            input.conversionCustomerId,
            input.acceptedCustomerDataTerms,
            input.conversionActions === null
              ? null
              : JSON.stringify(
                  input.conversionActions.map((action) => ({
                    id: action.id,
                    resourceName: `customers/${input.customerId}/conversionActions/${action.id}`,
                    name: action.name,
                    category: action.category,
                    status: action.status,
                    countingType: action.countingType,
                    clickThroughLookbackWindowDays: 30,
                    ...(action.attributionModel === undefined ? {} : { attributionModel: action.attributionModel }),
                  })),
                ),
            input.conversionActionsSyncedAt,
            input.setupError,
            input.setupError ? new Date().toISOString() : null,
          ],
        )
        await client.query(
          `INSERT INTO "Connection"
             ("id", "workspaceId", "provider", "kind", "channel", "integrationId", "sourceId", "displayName",
              "status", "statusReason", "connectedAt", "disconnectedAt")
           VALUES ($1, $2, 'googleAds', 'integration', NULL, $3, $4, $5, $6, $7, now(), $8)`,
          [
            createId(),
            input.workspaceId,
            integrationId,
            input.customerId,
            input.descriptiveName ?? input.customerId,
            input.connectionStatus,
            // Connection_status_reason_check: a reason exactly when not connected.
            GOOGLE_ADS_STATUS_REASON[input.connectionStatus],
            input.connectionStatus === "disconnected" ? new Date().toISOString() : null,
          ],
        )
        return { integrationGoogleAdsId }
      })
    },

    async setGoogleAdsSettings(input) {
      await db.pool.query(
        `INSERT INTO "GoogleAdsSettings" ("id", "workspaceId", "settings") VALUES ($1, $2, $3)
         ON CONFLICT ("workspaceId") DO UPDATE SET "settings" = EXCLUDED."settings", "updatedAt" = now()`,
        [createId(), input.workspaceId, JSON.stringify(input.settings)],
      )
    },

    async insertGoogleAdsEvents(input) {
      const ids: string[] = []
      await db.transaction(async (client) => {
        for (const row of input.rows) {
          const id = createId()
          ids.push(id)
          await client.query(
            `INSERT INTO "GoogleAdsConversionEvent"
               ("id", "workspaceId", "integrationGoogleAdsId", "customerId", "conversionCustomerId",
                "conversionActionId", "conversionActionName", "conversionActionCategory", "lookbackWindowDays",
                "channel", "source", "scopeId", "clickIdType", "clickId", "googleClickReceivedAt", "occurredAt",
                "value", "currency", "transactionId", "options", "uploadMethod", "status", "attempt",
                "requestId", "sentAt", "error", "failureStage", "processingStatus", "processingAttempts")
             VALUES ($1, $2, $3, $4, $4, $5, $6, $7, 30, $8, 'flowStep', $9, $10, $11, $12, $12,
                     $13, $14, $15, $16, $17, $18, 1, $19, $20, $21, $22, $23, 0)`,
            [
              id,
              input.workspaceId,
              input.integrationGoogleAdsId,
              input.customerId,
              row.conversionActionId,
              row.conversionActionName,
              row.conversionActionCategory,
              row.channel,
              `flow-${id}`,
              row.clickIdType,
              row.clickId,
              row.occurredAt,
              row.value ?? null,
              row.currency ?? null,
              row.transactionId,
              row.options === undefined || row.options === null ? null : JSON.stringify(row.options),
              row.uploadMethod,
              row.status,
              row.requestId ?? null,
              row.sentAt ?? null,
              row.error ?? null,
              row.failureStage ?? null,
              row.processingStatus ?? null,
            ],
          )
        }
      })
      return { ids }
    },
  }
}

export const create = (): P109Fixtures => {
  const db = openDb(process.env.PARITY_PG_URL ?? "postgresql://chatbotx:secretkey@postgres:5432/chatbotx")
  return {
    ...flowsSection(db),
    ...igSection(db),
    close: () => db.pool.end(),
  }
}
