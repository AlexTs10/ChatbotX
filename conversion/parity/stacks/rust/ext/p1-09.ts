// Rust-stack implementation of the P1.9 fixture extension
// (`src/fixtures/ext/p1-09.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same columns as `stacks/original/ext/p1-09.ts`
// (dates as epoch ms, `paths` as a JSON array of id strings). Methods are
// added as the P3.6 ports need them; the others throw.

import { DatabaseSync } from "node:sqlite"
import type { P109Fixtures, P109GoogleAdsEventRow } from "../../../src/fixtures/ext/p1-09.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
const PLACE_ID = 6n

let lastMs = 0n
let sequence = 0n
const createId = (): bigint => {
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
    sequence = 0n
  }
  return (now << 14n) | (PLACE_ID << 10n) | sequence
}

const notPorted = (name: string) => (): never => {
  throw new Error(`p1-09 fixture ${name} is not ported to the Rust stack yet`)
}

class SqliteP109Fixtures implements P109Fixtures {
  private db: DatabaseSync | undefined

  constructor(private readonly path: string) {}

  private open(): DatabaseSync {
    if (!this.db) {
      const db = new DatabaseSync(this.path)
      db.exec("PRAGMA busy_timeout = 10000; PRAGMA foreign_keys = ON;")
      this.db = db
    }
    return this.db
  }

  // folderService.create (packages/business/src/folder/service.ts), top level:
  // parentId null, paths [].
  async createFolder(input: { workspaceId: string; name: string; folderType: string }): Promise<{ folderId: string }> {
    const folderId = createId()
    const now = Date.now()
    this.open()
      .prepare(
        `INSERT INTO "Folder" ("id", "createdAt", "updatedAt", "name", "folderType", "parentId", "workspaceId", "paths")
         VALUES (?, ?, ?, ?, ?, NULL, ?, '[]')`,
      )
      .run(folderId, now, now, input.name, input.folderType, BigInt(input.workspaceId))
    return { folderId: folderId.toString() }
  }

  private insertInbox(
    db: DatabaseSync,
    input: { workspaceId: string; name: string; channel: string; sourceId: string },
  ): bigint {
    const inboxId = createId()
    const now = Date.now()
    db.prepare(
      `INSERT INTO "Inbox" ("id", "createdAt", "updatedAt", "name", "channel", "sourceId", "workspaceId", "status")
       VALUES (?, ?, ?, ?, ?, ?, ?, 'connected')`,
    ).run(inboxId, now, now, input.name, input.channel, input.sourceId, BigInt(input.workspaceId))
    return inboxId
  }

  // Same rows as stacks/original/ext/p1-09.ts (Inbox + IntegrationInstagram + Connection).
  async createInstagramChannel(input: {
    workspaceId: string
    igId: string
    pageId: string
    name: string
    username: string
    type: "instagram" | "facebook"
  }): Promise<{ integrationId: string; inboxId: string }> {
    const db = this.open()
    db.exec("BEGIN IMMEDIATE")
    try {
      const inboxId = this.insertInbox(db, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "instagram",
        sourceId: input.igId,
      })
      const integrationId = createId()
      const now = Date.now()
      const auth = {
        authType: "oauth2",
        clientId: "parity-client",
        clientSecret: "parity-secret",
        redirectUrl: "http://app/integrations/instagram/callback",
        version: "v23.0",
        tokens: { accessToken: `ig-token-${input.igId}` },
        metadata: { igId: input.igId, pageId: input.pageId, version: "v23.0" },
      }
      db.prepare(
        `INSERT INTO "IntegrationInstagram"
           ("id", "createdAt", "updatedAt", "auth", "igId", "pageId", "name", "username", "workspaceId", "inboxId",
            "type", "conversationStarters", "persistentMenus")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, '[]', '[]')`,
      ).run(
        integrationId,
        now,
        now,
        JSON.stringify(auth),
        input.igId,
        input.pageId,
        input.name,
        input.username,
        BigInt(input.workspaceId),
        inboxId,
        input.type,
      )
      db.prepare(
        `INSERT INTO "Connection"
           ("id", "createdAt", "updatedAt", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId",
            "displayName", "status", "connectedAt")
         VALUES (?, ?, ?, ?, ?, 'channel', 'instagram', ?, ?, ?, 'connected', ?)`,
      ).run(
        createId(),
        now,
        now,
        BigInt(input.workspaceId),
        input.type === "facebook" ? "instagramFacebook" : "instagram",
        inboxId,
        input.igId,
        input.name,
        now,
      )
      db.exec("COMMIT")
      return { integrationId: String(integrationId), inboxId: String(inboxId) }
    } catch (error) {
      db.exec("ROLLBACK")
      throw error
    }
  }

  async createInbox(input: {
    workspaceId: string
    name: string
    channel: string
    sourceId: string
  }): Promise<{ inboxId: string }> {
    return { inboxId: String(this.insertInbox(this.open(), input)) }
  }

  async addWorkspaceMember(input: { workspaceId: string; userId: string }): Promise<{ memberId: string }> {
    const memberId = createId()
    this.open()
      .prepare(
        `INSERT INTO "WorkspaceMember"
           ("id", "workspaceId", "userId", "role", "permissions", "notificationTypes", "notificationChannels")
         VALUES (?, ?, ?, 'agent', ?, ?, ?)`,
      )
      .run(
        memberId,
        BigInt(input.workspaceId),
        BigInt(input.userId),
        JSON.stringify({
          superAdmin: false,
          analytics: true,
          flows: true,
          contacts: true,
          onlyAssignedContacts: false,
          emailAndPhone: true,
          broadcast: true,
          ecommerce: true,
        }),
        JSON.stringify({ notifyAdmin: true, newMessageToHuman: true, newOrder: true }),
        JSON.stringify({ messenger: true, email: true, telegram: true, browser: true }),
      )
    return { memberId: String(memberId) }
  }
  // Same rows as stacks/original/ext/p1-09.ts (Integration + IntegrationGoogleAds + Connection).
  async createGoogleAdsConnection(
    input: Parameters<P109Fixtures["createGoogleAdsConnection"]>[0],
  ): Promise<{ integrationGoogleAdsId: string }> {
    const db = this.open()
    const integrationId = createId()
    const integrationGoogleAdsId = createId()
    const now = Date.now()
    const ms = (iso: string | null) => (iso === null ? null : Date.parse(iso))
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
    const reason: Record<string, string | null> = {
      connected: null,
      degraded: "verify_failed",
      needs_reauth: "token_revoked",
      paused: "manual",
      disconnected: "manual",
    }
    db.exec("BEGIN")
    try {
      db.prepare(`INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (?, ?, 'googleAds')`).run(
        integrationId,
        BigInt(input.workspaceId),
      )
      db.prepare(
        `INSERT INTO "IntegrationGoogleAds"
           ("id", "workspaceId", "integrationId", "auth", "customerId", "descriptiveName", "currencyCode",
            "conversionCustomerId", "acceptedCustomerDataTerms", "conversionActions",
            "conversionActionsSyncedAt", "setupError", "setupErrorAt")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(
        integrationGoogleAdsId,
        BigInt(input.workspaceId),
        integrationId,
        JSON.stringify(auth),
        input.customerId,
        input.descriptiveName,
        input.currencyCode,
        input.conversionCustomerId,
        input.acceptedCustomerDataTerms === null ? null : input.acceptedCustomerDataTerms ? 1 : 0,
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
        ms(input.conversionActionsSyncedAt),
        input.setupError,
        input.setupError ? now : null,
      )
      db.prepare(
        `INSERT INTO "Connection"
           ("id", "createdAt", "updatedAt", "workspaceId", "provider", "kind", "channel", "integrationId", "sourceId",
            "displayName", "status", "statusReason", "connectedAt", "disconnectedAt")
         VALUES (?, ?, ?, ?, 'googleAds', 'integration', NULL, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(
        createId(),
        now,
        now,
        BigInt(input.workspaceId),
        integrationId,
        input.customerId,
        input.descriptiveName ?? input.customerId,
        input.connectionStatus,
        reason[input.connectionStatus] ?? null,
        now,
        input.connectionStatus === "disconnected" ? now : null,
      )
      db.exec("COMMIT")
    } catch (error) {
      db.exec("ROLLBACK")
      throw error
    }
    return { integrationGoogleAdsId: integrationGoogleAdsId.toString() }
  }

  async setGoogleAdsSettings(input: { workspaceId: string; settings: unknown }): Promise<void> {
    const now = Date.now()
    this.open()
      .prepare(
        `INSERT INTO "GoogleAdsSettings" ("id", "createdAt", "updatedAt", "workspaceId", "settings") VALUES (?, ?, ?, ?, ?)
         ON CONFLICT ("workspaceId") DO UPDATE SET "settings" = excluded."settings", "updatedAt" = excluded."updatedAt"`,
      )
      .run(createId(), now, now, BigInt(input.workspaceId), JSON.stringify(input.settings))
  }

  async insertGoogleAdsEvents(input: {
    workspaceId: string
    integrationGoogleAdsId: string | null
    customerId: string
    rows: P109GoogleAdsEventRow[]
  }): Promise<{ ids: string[] }> {
    const db = this.open()
    const ids: string[] = []
    const ms = (iso: string | null | undefined) => (iso === null || iso === undefined ? null : Date.parse(iso))
    const insert = db.prepare(
      `INSERT INTO "GoogleAdsConversionEvent"
         ("id", "createdAt", "updatedAt", "workspaceId", "integrationGoogleAdsId", "customerId", "conversionCustomerId",
          "conversionActionId", "conversionActionName", "conversionActionCategory", "lookbackWindowDays",
          "channel", "source", "scopeId", "clickIdType", "clickId", "googleClickReceivedAt", "occurredAt",
          "value", "currency", "transactionId", "options", "uploadMethod", "status", "attempt",
          "requestId", "sentAt", "error", "failureStage", "processingStatus", "processingAttempts")
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 30, ?, 'flowStep', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?, ?, 0)`,
    )
    db.exec("BEGIN")
    try {
      for (const row of input.rows) {
        const id = createId()
        ids.push(id.toString())
        const now = Date.now()
        const occurredAt = ms(row.occurredAt)
        insert.run(
          id,
          now,
          now,
          BigInt(input.workspaceId),
          input.integrationGoogleAdsId === null ? null : BigInt(input.integrationGoogleAdsId),
          input.customerId,
          input.customerId,
          row.conversionActionId,
          row.conversionActionName,
          row.conversionActionCategory,
          row.channel,
          `flow-${id}`,
          row.clickIdType,
          row.clickId,
          occurredAt,
          occurredAt,
          row.value ?? null,
          row.currency ?? null,
          row.transactionId,
          row.options === undefined || row.options === null ? null : JSON.stringify(row.options),
          row.uploadMethod,
          row.status,
          row.requestId ?? null,
          ms(row.sentAt),
          row.error ?? null,
          row.failureStage ?? null,
          row.processingStatus ?? null,
        )
      }
      db.exec("COMMIT")
    } catch (error) {
      db.exec("ROLLBACK")
      throw error
    }
    return { ids }
  }

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const create = (): P109Fixtures =>
  new SqliteP109Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
