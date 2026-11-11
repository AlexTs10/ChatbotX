// Postgres implementation of the P1.8 fixture extension (original stack).
// Writes the same logical rows the app writes; see the comment per method.
// Setup only; assertions stay black-box (HTTP, fake upstream records, files).
//
// Row sources (baseline 40e733cee):
// - Inbox + IntegrationMessenger: the Messenger channel connect path
//   (inboxService.create + the store binding's insertRow,
//   packages/business/src/connection/store-bindings-channels.ts). `auth` is
//   plain jsonb (packages/database/src/schema/integration-messenger.ts), shape
//   MessengerAuthValue (integrations/messenger/src/schema.ts).
// - Folder: folderService.create (packages/business/src/folder/service.ts),
//   no parent, paths [].
// - ErrorLog: the `error-log:recorded` event-bus writer
//   (packages/business/src/error-log/service.ts toEntry → the stream consumer).

import { randomBytes } from "node:crypto"
import pg from "pg"
import type {
  CreatedMessengerChannel,
  ErrorLogRow,
  FolderType,
  OrderRow,
  P108Fixtures,
} from "../../../src/fixtures/ext/p1-08.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
/** App processes use place ids 1-3; core fixtures 15; P1.2/P1.4 14; P1.3 12; P1.5 11; P1.8 uses 8. */
const PLACE_ID = 8n

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

/** `messagingProfileDefaults()` of store-bindings-channels.ts (conversationStarters, persistentMenus). */
const PROFILE_DEFAULTS_SQL = `'[]'::jsonb, '[]'::jsonb`

class PostgresP108Fixtures implements P108Fixtures {
  private readonly pool: pg.Pool

  constructor(connectionString: string) {
    this.pool = new pg.Pool({ connectionString, max: 2, idleTimeoutMillis: 1_000, allowExitOnIdle: true })
  }

  async transaction<T>(fn: (client: pg.PoolClient) => Promise<T>): Promise<T> {
    const client = await this.pool.connect()
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
  }

  query(text: string, values: unknown[] = []) {
    return this.pool.query(text, values)
  }

  async createMessengerChannel(input: {
    workspaceId: string
    pageId: string
    name: string
    accessToken?: string
    withAppCredentials?: boolean
  }): Promise<CreatedMessengerChannel> {
    const accessToken = input.accessToken ?? `page-token-${input.pageId}`
    const auth = {
      authType: "oauth2",
      ...(input.withAppCredentials === false
        ? {}
        : { clientId: "parity-client", clientSecret: "parity-secret" }),
      redirectUrl: "http://app/integrations/messenger/callback",
      version: "v23.0",
      tokens: { accessToken },
      metadata: { pageId: input.pageId, pageName: input.name, version: "v23.0" },
    }
    return await this.transaction(async (client) => {
      const inboxId = createId()
      await client.query(
        `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId")
         VALUES ($1, $2, 'messenger', $3, $4)`,
        [inboxId, input.name, input.pageId, input.workspaceId],
      )
      const integrationId = createId()
      await client.query(
        `INSERT INTO "IntegrationMessenger"
           ("id", "auth", "pageId", "name", "workspaceId", "inboxId", "conversationStarters", "persistentMenus", "personas")
         VALUES ($1, $2, $3, $4, $5, $6, ${PROFILE_DEFAULTS_SQL}, '[]'::jsonb)`,
        [integrationId, JSON.stringify(auth), input.pageId, input.name, input.workspaceId, inboxId],
      )
      return { integrationId, inboxId, accessToken }
    })
  }

  async createFolder(input: { workspaceId: string; name: string; folderType: FolderType }): Promise<{ folderId: string }> {
    const folderId = createId()
    await this.pool.query(
      `INSERT INTO "Folder" ("id", "name", "folderType", "parentId", "workspaceId", "paths")
       VALUES ($1, $2, $3, NULL, $4, '{}')`,
      [folderId, input.name, input.folderType, input.workspaceId],
    )
    return { folderId }
  }

  async insertErrorLogs(input: { workspaceId: string; rows: ErrorLogRow[] }): Promise<{ ids: string[] }> {
    const ids: string[] = []
    await this.transaction(async (client) => {
      for (const row of input.rows) {
        const id = createId()
        ids.push(id)
        await client.query(
          `INSERT INTO "ErrorLog"
             ("id", "action", "detail", "httpCode", "workspaceId", "contactId", "sourceId", "stackTrace", "createdAt", "updatedAt")
           VALUES ($1, $2, $3, $4, $5, NULL, $6, $7, $8, $8)`,
          [
            id,
            row.action,
            row.detail,
            row.httpCode,
            input.workspaceId,
            row.sourceId ?? null,
            row.stackTrace ?? null,
            row.createdAt.toISOString(),
          ],
        )
      }
    })
    return { ids }
  }

  async createContact(input: {
    workspaceId: string
    inboxId: string
    channel: string
    sourceId: string
    firstName: string
  }): Promise<{ contactId: string; contactInboxId: string; conversationId: string }> {
    const contactId = createId()
    const contactInboxId = createId()
    const conversationId = createId()
    await this.transaction(async (client) => {
      await client.query(
        `INSERT INTO "Contact" ("id", "workspaceId", "firstName", "emailVerified", "emailOptIn")
         VALUES ($1, $2, $3, false, true)`,
        [contactId, input.workspaceId, input.firstName],
      )
      await client.query(
        `INSERT INTO "ContactInbox"
           ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId", "consecutiveFailedReply")
         VALUES ($1, $2, $2, $3, $4, 'direct', $5, 0)`,
        [contactInboxId, contactId, input.inboxId, input.channel, input.sourceId],
      )
      await client.query(
        `INSERT INTO "Conversation" ("id", "workspaceId", "contactId", "followed", "botEnabled")
         VALUES ($1, $2, $3, false, true)`,
        [conversationId, input.workspaceId, contactId],
      )
    })
    return { contactId, contactInboxId, conversationId }
  }

  async setCustomFieldValue(input: { contactId: string; customFieldId: string; value: string }): Promise<void> {
    await this.pool.query(
      `INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value")
       VALUES ($1, $2, $3, $4)
       ON CONFLICT ("contactId", "customFieldId") DO UPDATE SET "value" = EXCLUDED."value", "updatedAt" = now()`,
      [createId(), input.contactId, input.customFieldId, input.value],
    )
  }

  /**
   * Order + OrderItem rows with the columns `orderService.place`
   * (packages/business/src/order/service.ts) fills for a cash-on-delivery API
   * order without discounts, taxes, shipping or service fee: subtotal = total.
   */
  async insertOrders(input: { workspaceId: string; orders: OrderRow[] }): Promise<{ ids: string[] }> {
    const ids: string[] = []
    await this.transaction(async (client) => {
      for (const order of input.orders) {
        const orderId = createId()
        ids.push(orderId)
        const createdAt = order.createdAt.toISOString()
        await client.query(
          `INSERT INTO "Order"
             ("id", "createdAt", "updatedAt", "workspaceId", "number", "source", "status", "paymentMethod", "paymentStatus",
              "currency", "subtotal", "discountTotal", "shippingTotal", "serviceFeeTotal", "taxTotal", "total",
              "cancelledAt")
           VALUES ($1, $2, $2, $3, $4, 'api', $5, 'cod', 'unpaid', $6, $7, 0, 0, 0, 0, $7, $8)`,
          [
            orderId,
            createdAt,
            input.workspaceId,
            order.number,
            order.status,
            order.currency,
            order.total,
            order.status === "cancelled" ? createdAt : null,
          ],
        )
        for (const item of order.items) {
          await client.query(
            `INSERT INTO "OrderItem"
               ("id", "createdAt", "updatedAt", "orderId", "workspaceId", "productId", "name", "vendorName",
                "unitPrice", "quantity", "discountPercent", "taxPercent", "discountTotal", "taxTotal", "lineTotal")
             VALUES ($1, $2, $2, $3, $4, NULL, $5, $6, $7, $8, 0, 0, 0, 0, $9)`,
            [
              createId(),
              createdAt,
              orderId,
              input.workspaceId,
              item.name,
              item.vendorName,
              item.unitPrice,
              item.quantity,
              item.lineTotal,
            ],
          )
        }
      }
    })
    return { ids }
  }

  // ---- p1-08 marketing messages: begin ----
  // Row sources (baseline 40e733cee):
  // - PlatformCredential (user-owned, type messenger):
  //   platformCredentialService.upsertForUser
  //   (packages/business/src/platform-credential/service.ts): value =
  //   encryptUtils.encryptObject(config, "user:<userId>:messenger:false").
  // - FacebookMarketingMessagesAuth: facebookMarketingMessagesService.upsertAuth
  //   (packages/business/src/facebook-marketing-messages/service.ts): auth =
  //   encryptUtils.encryptObject(facebookAdsAuthValue) (no AAD).
  // - FacebookMarketingMessage: facebookMarketingMessageRepository.create +
  //   the delivery state machine's transitions
  //   (packages/database/src/schema/facebook-marketing-message.ts).
  // - FacebookMarketingMessageAudience: the audience upload service
  //   (packages/business/src/facebook-marketing-message-audience/upload.service.ts)
  //   + markReady.
  // - FacebookMarketingMessageRecipient: the token snapshot + dispatch
  //   (packages/database/src/schema/facebook-marketing-message-recipient.ts).
  // Encryption: AES-256-GCM, 12-byte IV, 16-byte tag, hex fields,
  // {v:1, kid, iv, text, tag[, aad]} (packages/encryption/src/encryption.ts),
  // key = ENCRYPTION_KEY of stacks/original/original.env.

  private async mmEncryptObject(value: unknown, aad?: string) {
    const { createCipheriv, randomBytes: random } = await import("node:crypto")
    const { readFileSync } = await import("node:fs")
    const path = await import("node:path")
    const { fileURLToPath } = await import("node:url")
    const envFile = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../original.env")
    const env: Record<string, string> = {}
    for (const line of readFileSync(envFile, "utf8").split("\n")) {
      const match = /^([A-Z0-9_]+)=(.*)$/.exec(line.trim())
      if (match?.[1]) {
        env[match[1]] = match[2] ?? ""
      }
    }
    const key = Buffer.from(env.ENCRYPTION_KEY ?? "", "hex")
    if (key.length !== 32) {
      throw new Error("p1-08 fixtures: ENCRYPTION_KEY must be 32 bytes of hex")
    }
    const iv = random(12)
    const cipher = createCipheriv("aes-256-gcm", key, iv, { authTagLength: 16 })
    if (aad !== undefined) {
      cipher.setAAD(Buffer.from(aad, "utf8"))
    }
    const text = Buffer.concat([cipher.update(JSON.stringify(value), "utf8"), cipher.final()])
    return {
      v: 1,
      kid: env.ENCRYPTION_KEY_ID || "default",
      iv: iv.toString("hex"),
      text: text.toString("hex"),
      tag: cipher.getAuthTag().toString("hex"),
      ...(aad !== undefined ? { aad } : {}),
    }
  }

  async createMessengerAppCredential(input: {
    userId: string
    clientId: string
    clientSecret: string
    version: string
  }): Promise<{ credentialId: string }> {
    const verifyToken = `parity-verify-${input.clientId}`
    const config = { clientId: input.clientId, version: input.version, verifyToken, clientSecret: input.clientSecret }
    const publicConfig = { clientId: input.clientId, version: input.version, verifyToken }
    const value = await this.mmEncryptObject(config, `user:${input.userId}:messenger:false`)
    const credentialId = createId()
    await this.pool.query(
      `INSERT INTO "PlatformCredential" ("id", "userId", "type", "publicConfig", "value", "usePlatformCredential", "isVerified")
       VALUES ($1, $2, 'messenger', $3, $4, false, false)`,
      [credentialId, input.userId, JSON.stringify(publicConfig), JSON.stringify(value)],
    )
    return { credentialId }
  }

  async createMarketingMessagesAuth(input: {
    workspaceId: string
    accessToken: string
    version?: string
    status?: "active" | "invalid"
    tokenExpiresAt?: Date | null
    facebookUserId?: string | null
  }): Promise<{ authId: string }> {
    const auth = await this.mmEncryptObject({
      authType: "custom",
      accessToken: input.accessToken,
      version: input.version ?? "v23.0",
    })
    const authId = createId()
    await this.pool.query(
      `INSERT INTO "FacebookMarketingMessagesAuth" ("id", "workspaceId", "auth", "tokenExpiresAt", "status", "facebookUserId")
       VALUES ($1, $2, $3, $4, $5, $6)`,
      [
        authId,
        input.workspaceId,
        JSON.stringify(auth),
        input.tokenExpiresAt ? input.tokenExpiresAt.toISOString() : null,
        input.status ?? "active",
        input.facebookUserId ?? null,
      ],
    )
    return { authId }
  }

  async createMarketingMessage(input: import("../../../src/fixtures/ext/p1-08.ts").MmCampaignInput): Promise<{ id: string }> {
    const id = createId()
    const iso = (value: Date | null | undefined) => (value ? value.toISOString() : null)
    await this.pool.query(
      `INSERT INTO "FacebookMarketingMessage"
         ("id", "workspaceId", "name", "pageId", "adAccountId", "currency", "currencyOffset", "budgetType",
          "budgetMinorUnits", "content", "targetType", "audienceId", "campaignId", "status", "statusReason",
          "startsAt", "firstSentAt", "metaEndsAt", "timezoneName", "adSetId", "tokenCount", "createdAt", "updatedAt")
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $22)`,
      [
        id,
        input.workspaceId,
        input.name,
        input.pageId,
        input.adAccountId,
        input.currency,
        input.currencyOffset,
        input.budgetType,
        input.budgetMinorUnits,
        JSON.stringify(input.content),
        input.targetType ?? "optIn",
        input.audienceId ?? null,
        input.campaignId,
        input.status,
        input.statusReason ?? null,
        iso(input.startsAt),
        iso(input.firstSentAt),
        iso(input.metaEndsAt),
        input.timezoneName ?? null,
        input.adSetId ?? null,
        input.tokenCount ?? null,
        (input.createdAt ?? new Date()).toISOString(),
      ],
    )
    return { id }
  }

  async createMarketingMessageAudience(input: import("../../../src/fixtures/ext/p1-08.ts").MmAudienceInput): Promise<{ id: string }> {
    const id = createId()
    const createdAt = (input.createdAt ?? new Date()).toISOString()
    await this.pool.query(
      `INSERT INTO "FacebookMarketingMessageAudience"
         ("id", "workspaceId", "customAudienceId", "adAccountId", "pageId", "name", "description", "rowsUploaded",
          "readyAt", "createdAt", "updatedAt")
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $10)`,
      [
        id,
        input.workspaceId,
        input.customAudienceId,
        input.adAccountId,
        input.pageId,
        input.name,
        input.description ?? null,
        input.rowsUploaded,
        input.readyAt ? input.readyAt.toISOString() : null,
        createdAt,
      ],
    )
    return { id }
  }

  async insertMarketingMessageRecipients(input: {
    workspaceId: string
    campaignRowId: string
    rows: import("../../../src/fixtures/ext/p1-08.ts").MmRecipientInput[]
  }): Promise<void> {
    await this.transaction(async (client) => {
      for (const [index, row] of input.rows.entries()) {
        const now = new Date().toISOString()
        await client.query(
          `INSERT INTO "FacebookMarketingMessageRecipient"
             ("id", "workspaceId", "campaignRowId", "subscriptionToken", "status", "attempts",
              "sentAt", "deliveredAt", "failedAt", "errorCode", "errorMessage", "errorSource")
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)`,
          [
            createId(),
            input.workspaceId,
            input.campaignRowId,
            `parity-token-${input.campaignRowId}-${index}`,
            row.status,
            row.status === "pending" ? 0 : 1,
            ["sent", "delivered", "unconfirmed"].includes(row.status) ? now : null,
            row.status === "delivered" ? now : null,
            row.status === "failed" ? now : null,
            row.errorCode ?? null,
            row.errorMessage ?? null,
            row.status === "failed" ? "send" : null,
          ],
        )
      }
    })
  }
  // ---- p1-08 marketing messages: end ----

  async close(): Promise<void> {
    await this.pool.end()
  }
}

export const create = (): P108Fixtures =>
  new PostgresP108Fixtures(process.env.PARITY_PG_URL ?? "postgresql://chatbotx:secretkey@postgres:5432/chatbotx")
