// Rust-stack implementation of the P1.8 fixture extension
// (`src/fixtures/ext/p1-08.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same columns as `stacks/original/ext/p1-08.ts`.
// Methods are added as the ports need them (Q.D5 email topics, Q.D7 lead ads
// and marketing messages); the others throw.

import { createCipheriv, randomBytes } from "node:crypto"
import { readFileSync } from "node:fs"
import path from "node:path"
import { DatabaseSync } from "node:sqlite"
import { fileURLToPath } from "node:url"
import type {
  CreatedMessengerChannel,
  ErrorLogRow,
  FolderType,
  MmAudienceInput,
  MmCampaignInput,
  MmRecipientInput,
  P108Fixtures,
} from "../../../src/fixtures/ext/p1-08.ts"

const readEnv = (): Record<string, string> => {
  const file = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../rust.env")
  const values: Record<string, string> = {}
  for (const line of readFileSync(file, "utf8").split("\n")) {
    const match = /^([A-Z0-9_]+)=(.*)$/.exec(line.trim())
    if (match?.[1]) {
      values[match[1]] = match[2] ?? ""
    }
  }
  return values
}

/** encryptUtils.encryptObject(value, aad) of the app. */
const encryptObject = (value: unknown, aad?: string) => {
  const env = readEnv()
  const key = Buffer.from(env.ENCRYPTION_KEY ?? "", "hex")
  if (key.length !== 32) {
    throw new Error("p1-08 fixtures: ENCRYPTION_KEY must be 32 bytes of hex")
  }
  const iv = randomBytes(12)
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

const ms = (value: Date | null | undefined) => (value ? value.getTime() : null)

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
// A place id of its own (p1-03..p1-12 use 4-9), so fixture ids never collide.
const PLACE_ID = 10n

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
  throw new Error(`p1-08 fixture ${name} is not ported to the Rust stack yet`)
}

class SqliteP108Fixtures {
  private db: DatabaseSync | undefined

  constructor(private readonly path: string) {}

  private open(): DatabaseSync {
    if (!this.db) {
      const db = new DatabaseSync(this.path)
      db.exec("PRAGMA busy_timeout = 10000; PRAGMA foreign_keys = ON;")
      // The app function of the `Contact` full-name index (crates/db section 3).
      db.function("regexp_replace", { deterministic: true }, (text, pattern, replacement, flags) =>
        text == null
          ? null
          : String(text).replace(new RegExp(String(pattern), String(flags ?? "")), String(replacement)),
      )
      this.db = db
    }
    return this.db
  }

  async createFolder(input: { workspaceId: string; name: string; folderType: FolderType }): Promise<{ folderId: string }> {
    const folderId = createId()
    this.open()
      .prepare(
        `INSERT INTO "Folder" ("id", "name", "folderType", "parentId", "workspaceId", "paths")
         VALUES (?, ?, ?, NULL, ?, '[]')`,
      )
      .run(folderId, input.name, input.folderType, BigInt(input.workspaceId))
    return { folderId: folderId.toString() }
  }

  private transaction<T>(run: (db: DatabaseSync) => T): T {
    const db = this.open()
    db.exec("BEGIN IMMEDIATE")
    try {
      const out = run(db)
      db.exec("COMMIT")
      return out
    } catch (error) {
      db.exec("ROLLBACK")
      throw error
    }
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
      ...(input.withAppCredentials === false ? {} : { clientId: "parity-client", clientSecret: "parity-secret" }),
      redirectUrl: "http://app/integrations/messenger/callback",
      version: "v23.0",
      tokens: { accessToken },
      metadata: { pageId: input.pageId, pageName: input.name, version: "v23.0" },
    }
    return this.transaction((db) => {
      const now = Date.now()
      const inboxId = createId()
      db.prepare(
        `INSERT INTO "Inbox" ("id", "createdAt", "updatedAt", "name", "channel", "sourceId", "workspaceId", "status")
         VALUES (?, ?, ?, ?, 'messenger', ?, ?, 'connected')`,
      ).run(inboxId, now, now, input.name, input.pageId, BigInt(input.workspaceId))
      const integrationId = createId()
      db.prepare(
        `INSERT INTO "IntegrationMessenger"
           ("id", "createdAt", "updatedAt", "auth", "pageId", "name", "workspaceId", "inboxId",
            "conversationStarters", "persistentMenus", "personas")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, '[]', '[]', '[]')`,
      ).run(integrationId, now, now, JSON.stringify(auth), input.pageId, input.name, BigInt(input.workspaceId), inboxId)
      return { integrationId: String(integrationId), inboxId: String(inboxId), accessToken }
    })
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
    const value = encryptObject(config, `user:${input.userId}:messenger:false`)
    const credentialId = createId()
    this.open()
      .prepare(
        `INSERT INTO "PlatformCredential" ("id", "userId", "type", "publicConfig", "value", "usePlatformCredential", "isVerified")
         VALUES (?, ?, 'messenger', ?, ?, 0, 0)`,
      )
      .run(credentialId, BigInt(input.userId), JSON.stringify(publicConfig), JSON.stringify(value))
    return { credentialId: String(credentialId) }
  }

  async createMarketingMessagesAuth(input: {
    workspaceId: string
    accessToken: string
    version?: string
    status?: "active" | "invalid"
    tokenExpiresAt?: Date | null
    facebookUserId?: string | null
  }): Promise<{ authId: string }> {
    const auth = encryptObject({ authType: "custom", accessToken: input.accessToken, version: input.version ?? "v23.0" })
    const authId = createId()
    this.open()
      .prepare(
        `INSERT INTO "FacebookMarketingMessagesAuth" ("id", "workspaceId", "auth", "tokenExpiresAt", "status", "facebookUserId")
         VALUES (?, ?, ?, ?, ?, ?)`,
      )
      .run(
        authId,
        BigInt(input.workspaceId),
        JSON.stringify(auth),
        ms(input.tokenExpiresAt),
        input.status ?? "active",
        input.facebookUserId ?? null,
      )
    return { authId: String(authId) }
  }

  async createMarketingMessage(input: MmCampaignInput): Promise<{ id: string }> {
    const id = createId()
    const createdAt = (input.createdAt ?? new Date()).getTime()
    this.open()
      .prepare(
        `INSERT INTO "FacebookMarketingMessage"
           ("id", "workspaceId", "name", "pageId", "adAccountId", "currency", "currencyOffset", "budgetType",
            "budgetMinorUnits", "content", "targetType", "audienceId", "campaignId", "status", "statusReason",
            "startsAt", "firstSentAt", "metaEndsAt", "timezoneName", "adSetId", "tokenCount", "createdAt", "updatedAt")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        id,
        BigInt(input.workspaceId),
        input.name,
        input.pageId,
        input.adAccountId,
        input.currency,
        input.currencyOffset,
        input.budgetType,
        input.budgetMinorUnits,
        JSON.stringify(input.content),
        input.targetType ?? "optIn",
        input.audienceId ? BigInt(input.audienceId) : null,
        input.campaignId,
        input.status,
        input.statusReason ?? null,
        ms(input.startsAt),
        ms(input.firstSentAt),
        ms(input.metaEndsAt),
        input.timezoneName ?? null,
        input.adSetId ?? null,
        input.tokenCount ?? null,
        createdAt,
        createdAt,
      )
    return { id: String(id) }
  }

  async createMarketingMessageAudience(input: MmAudienceInput): Promise<{ id: string }> {
    const id = createId()
    const createdAt = (input.createdAt ?? new Date()).getTime()
    this.open()
      .prepare(
        `INSERT INTO "FacebookMarketingMessageAudience"
           ("id", "workspaceId", "customAudienceId", "adAccountId", "pageId", "name", "description", "rowsUploaded",
            "readyAt", "createdAt", "updatedAt")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        id,
        BigInt(input.workspaceId),
        input.customAudienceId,
        input.adAccountId,
        input.pageId,
        input.name,
        input.description ?? null,
        input.rowsUploaded,
        ms(input.readyAt),
        createdAt,
        createdAt,
      )
    return { id: String(id) }
  }

  async insertMarketingMessageRecipients(input: {
    workspaceId: string
    campaignRowId: string
    rows: MmRecipientInput[]
  }): Promise<void> {
    this.transaction((db) => {
      const insert = db.prepare(
        `INSERT INTO "FacebookMarketingMessageRecipient"
           ("id", "workspaceId", "campaignRowId", "subscriptionToken", "status", "attempts",
            "sentAt", "deliveredAt", "failedAt", "errorCode", "errorMessage", "errorSource")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      for (const [index, row] of input.rows.entries()) {
        const now = Date.now()
        insert.run(
          createId(),
          BigInt(input.workspaceId),
          BigInt(input.campaignRowId),
          `parity-token-${input.campaignRowId}-${index}`,
          row.status,
          row.status === "pending" ? 0 : 1,
          ["sent", "delivered", "unconfirmed"].includes(row.status) ? now : null,
          row.status === "delivered" ? now : null,
          row.status === "failed" ? now : null,
          row.errorCode ?? null,
          row.errorMessage ?? null,
          row.status === "failed" ? "send" : null,
        )
      }
    })
  }

  async insertErrorLogs(input: { workspaceId: string; rows: ErrorLogRow[] }): Promise<{ ids: string[] }> {
    return this.transaction((db) => {
      const insert = db.prepare(
        `INSERT INTO "ErrorLog"
           ("id", "action", "detail", "httpCode", "workspaceId", "contactId", "sourceId", "stackTrace", "createdAt", "updatedAt")
         VALUES (?, ?, ?, ?, ?, NULL, ?, ?, ?, ?)`,
      )
      const ids: string[] = []
      for (const row of input.rows) {
        const id = createId()
        ids.push(String(id))
        const at = row.createdAt.getTime()
        insert.run(
          id,
          row.action,
          row.detail,
          row.httpCode,
          BigInt(input.workspaceId),
          row.sourceId ?? null,
          row.stackTrace ?? null,
          at,
          at,
        )
      }
      return { ids }
    })
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
    const now = Date.now()
    this.transaction((db) => {
      db.prepare(
        `INSERT INTO "Contact" ("id", "createdAt", "updatedAt", "workspaceId", "firstName") VALUES (?, ?, ?, ?, ?)`,
      ).run(contactId, now, now, BigInt(input.workspaceId), input.firstName)
      db.prepare(
        `INSERT INTO "ContactInbox"
           ("id", "createdAt", "updatedAt", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
         VALUES (?, ?, ?, ?, ?, ?, ?, 'direct', ?)`,
      ).run(contactInboxId, now, now, contactId, contactId, BigInt(input.inboxId), input.channel, input.sourceId)
      db.prepare(
        `INSERT INTO "Conversation" ("id", "createdAt", "updatedAt", "workspaceId", "contactId") VALUES (?, ?, ?, ?, ?)`,
      ).run(conversationId, now, now, BigInt(input.workspaceId), contactId)
    })
    return { contactId: String(contactId), contactInboxId: String(contactInboxId), conversationId: String(conversationId) }
  }

  async setCustomFieldValue(input: { contactId: string; customFieldId: string; value: string }): Promise<void> {
    this.open()
      .prepare(
        `INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value")
         VALUES (?, ?, ?, ?)
         ON CONFLICT ("contactId", "customFieldId") DO UPDATE SET "value" = excluded."value",
           "updatedAt" = CAST(unixepoch('subsec') * 1000 AS INTEGER)`,
      )
      .run(createId(), BigInt(input.contactId), BigInt(input.customFieldId), input.value)
  }

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

// Every other method throws until a port needs it (`then` stays undefined:
// the loader awaits this object, which must not look like a promise).
export const create = (): P108Fixtures =>
  new Proxy(new SqliteP108Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db"), {
    get: (target, key, receiver) =>
      key in target || typeof key !== "string" || key === "then"
        ? Reflect.get(target, key, receiver)
        : notPorted(key),
  }) as unknown as P108Fixtures
