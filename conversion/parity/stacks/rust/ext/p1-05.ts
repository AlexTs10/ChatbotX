// Rust-stack implementation of the P1.5 fixture extension
// (`src/fixtures/ext/p1-05.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same logical rows as
// `stacks/original/ext/p1-05.ts` (dates as epoch ms, JSON as text, booleans
// as 0/1). Encryption as the original: AES-256-GCM with the owner-bound AAD,
// key = ENCRYPTION_KEY of `stacks/rust/rust.env`.

import { createCipheriv, randomBytes } from "node:crypto"
import { readFileSync } from "node:fs"
import path from "node:path"
import { DatabaseSync } from "node:sqlite"
import { fileURLToPath } from "node:url"
import type {
  CreateCommentAutomationInput,
  CreatedChannel,
  ModerationLogRow,
  P105Fixtures,
} from "../../../src/fixtures/ext/p1-05.ts"

const here = path.dirname(fileURLToPath(import.meta.url))
const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
// A place id of its own (p1-12 uses 4, p1-06 5, p1-09 6, p1-07 7, p1-11 8, p1-03 9).
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

const readEnvFile = (): Record<string, string> => {
  const text = readFileSync(path.resolve(here, "../rust.env"), "utf8")
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

/** encryptUtils.encryptObject(value, aad) of the app. */
const encryptObject = (value: unknown, aad: string) => {
  if (ENCRYPTION_KEY.length !== 32) {
    throw new Error("p1-05 fixtures: ENCRYPTION_KEY must be 32 bytes of hex")
  }
  const iv = randomBytes(12)
  const cipher = createCipheriv("aes-256-gcm", ENCRYPTION_KEY, iv, { authTagLength: 16 })
  cipher.setAAD(Buffer.from(aad, "utf8"))
  const text = Buffer.concat([cipher.update(JSON.stringify(value), "utf8"), cipher.final()])
  return {
    v: 1,
    kid: ENCRYPTION_KEY_ID,
    iv: iv.toString("hex"),
    text: text.toString("hex"),
    tag: cipher.getAuthTag().toString("hex"),
    aad,
  }
}

const oauth2Auth = (accessToken: string, metadata: Record<string, unknown>) => ({
  authType: "oauth2",
  clientId: "parity-client",
  clientSecret: "parity-secret",
  redirectUrl: "http://app/integrations/messenger/callback",
  version: "v23.0",
  tokens: { accessToken },
  metadata,
})

const ms = (iso: string | null | undefined): number | null => (iso == null ? null : Date.parse(iso))
const optId = (value: string | null | undefined): bigint | null => (value == null ? null : BigInt(value))

class SqliteP105Fixtures implements P105Fixtures {
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

  private transaction<T>(fn: (db: DatabaseSync) => T): T {
    const db = this.open()
    db.exec("BEGIN IMMEDIATE")
    try {
      const result = fn(db)
      db.exec("COMMIT")
      return result
    } catch (error) {
      db.exec("ROLLBACK")
      throw error
    }
  }

  private insertInbox(
    db: DatabaseSync,
    input: { workspaceId: string; name: string; channel: string; sourceId: string; status?: string },
  ): bigint {
    const inboxId = createId()
    const now = Date.now()
    const status = input.status ?? "connected"
    db.prepare(
      `INSERT INTO "Inbox" ("id", "createdAt", "updatedAt", "name", "channel", "sourceId", "workspaceId", "status",
         "disconnectedAt")
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
    ).run(
      inboxId,
      now,
      now,
      input.name,
      input.channel,
      input.sourceId,
      BigInt(input.workspaceId),
      status,
      status === "disconnected" ? now : null,
    )
    return inboxId
  }

  private insertChannelConnection(
    db: DatabaseSync,
    input: { workspaceId: string; provider: string; channel: string; inboxId: bigint; sourceId: string; displayName: string },
  ): bigint {
    const connectionId = createId()
    const now = Date.now()
    db.prepare(
      `INSERT INTO "Connection"
         ("id", "createdAt", "updatedAt", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId",
          "displayName", "status", "connectedAt")
       VALUES (?, ?, ?, ?, ?, 'channel', ?, ?, ?, ?, 'connected', ?)`,
    ).run(
      connectionId,
      now,
      now,
      BigInt(input.workspaceId),
      input.provider,
      input.channel,
      input.inboxId,
      input.sourceId,
      input.displayName,
      now,
    )
    return connectionId
  }

  async createMessengerChannel(input: { workspaceId: string; pageId: string; name: string }): Promise<CreatedChannel> {
    return this.transaction((db) => {
      const inboxId = this.insertInbox(db, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "messenger",
        sourceId: input.pageId,
      })
      const integrationId = createId()
      const now = Date.now()
      db.prepare(
        `INSERT INTO "IntegrationMessenger"
           ("id", "createdAt", "updatedAt", "auth", "pageId", "name", "workspaceId", "inboxId",
            "conversationStarters", "persistentMenus", "personas")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, '[]', '[]', '[]')`,
      ).run(
        integrationId,
        now,
        now,
        JSON.stringify(
          oauth2Auth(`page-token-${input.pageId}`, { pageId: input.pageId, pageName: input.name, version: "v23.0" }),
        ),
        input.pageId,
        input.name,
        BigInt(input.workspaceId),
        inboxId,
      )
      const connectionId = this.insertChannelConnection(db, {
        workspaceId: input.workspaceId,
        provider: "messenger",
        channel: "messenger",
        inboxId,
        sourceId: input.pageId,
        displayName: input.name,
      })
      return { integrationId: String(integrationId), inboxId: String(inboxId), connectionId: String(connectionId) }
    })
  }

  async createInstagramChannel(input: {
    workspaceId: string
    igId: string
    pageId: string
    name: string
    username: string
    type: "instagram" | "facebook"
  }): Promise<CreatedChannel> {
    return this.transaction((db) => {
      const inboxId = this.insertInbox(db, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "instagram",
        sourceId: input.igId,
      })
      const integrationId = createId()
      const now = Date.now()
      db.prepare(
        `INSERT INTO "IntegrationInstagram"
           ("id", "createdAt", "updatedAt", "auth", "igId", "pageId", "name", "username", "workspaceId", "inboxId",
            "type", "conversationStarters", "persistentMenus")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, '[]', '[]')`,
      ).run(
        integrationId,
        now,
        now,
        JSON.stringify(oauth2Auth(`ig-token-${input.igId}`, { igId: input.igId, pageId: input.pageId, version: "v23.0" })),
        input.igId,
        input.pageId,
        input.name,
        input.username,
        BigInt(input.workspaceId),
        inboxId,
        input.type,
      )
      const connectionId = this.insertChannelConnection(db, {
        workspaceId: input.workspaceId,
        provider: input.type === "facebook" ? "instagramFacebook" : "instagram",
        channel: "instagram",
        inboxId,
        sourceId: input.igId,
        displayName: input.name,
      })
      return { integrationId: String(integrationId), inboxId: String(inboxId), connectionId: String(connectionId) }
    })
  }

  async createInbox(input: {
    workspaceId: string
    channel: string
    name: string
    sourceId: string
    status?: "connected" | "disconnected"
  }): Promise<{ inboxId: string }> {
    return { inboxId: String(this.insertInbox(this.open(), input)) }
  }

  async createContact(input: {
    workspaceId: string
    inboxId: string
    channel: string
    sourceId: string
    firstName?: string | null
    lastName?: string | null
  }): Promise<{ contactId: string; contactInboxId: string; conversationId: string }> {
    const contactId = createId()
    const contactInboxId = createId()
    const conversationId = createId()
    const now = Date.now()
    this.transaction((db) => {
      db.prepare(
        `INSERT INTO "Contact" ("id", "createdAt", "updatedAt", "workspaceId", "firstName", "lastName")
         VALUES (?, ?, ?, ?, ?, ?)`,
      ).run(contactId, now, now, BigInt(input.workspaceId), input.firstName ?? null, input.lastName ?? null)
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

  async createCommentAutomation(input: CreateCommentAutomationInput): Promise<{ automationId: string }> {
    const automationId = createId()
    const now = Date.now()
    this.open()
      .prepare(
        `INSERT INTO "CommentAutomation"
           ("id", "createdAt", "updatedAt", "name", "workspaceId", "type", "inboxId", "isActive", "startTime",
            "endTime", "post", "publicReply", "privateReply")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        automationId,
        now,
        now,
        input.name,
        BigInt(input.workspaceId),
        input.type,
        optId(input.inboxId),
        input.isActive === false ? 0 : 1,
        input.startTime ?? null,
        input.endTime ?? null,
        JSON.stringify(input.post ?? { type: "all", value: [] }),
        JSON.stringify(input.publicReply ?? { type: "none", value: null }),
        JSON.stringify(input.privateReply ?? { type: "none", value: null }),
      )
    return { automationId: String(automationId) }
  }

  async insertModerationLogs(rows: ModerationLogRow[]): Promise<{ ids: string[] }> {
    const ids = rows.map(() => createId())
    const now = Date.now()
    this.transaction((db) => {
      const insert = db.prepare(
        `INSERT INTO "CommentModerationLog"
           ("id", "createdAt", "updatedAt", "workspaceId", "automationId", "contactId", "contactInboxId", "postId",
            "commentId", "commentText", "source", "rule", "matchedKeyword", "categories", "reason", "action", "status",
            "errorDetail", "occurredAt")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      for (const [index, row] of rows.entries()) {
        insert.run(
          ids[index] as bigint,
          now,
          now,
          BigInt(row.workspaceId),
          BigInt(row.automationId),
          optId(row.contactId),
          optId(row.contactInboxId),
          row.postId,
          row.commentId,
          row.commentText ?? null,
          row.source,
          row.rule ?? null,
          row.matchedKeyword ?? null,
          JSON.stringify(row.categories ?? []),
          row.reason ?? null,
          row.action,
          row.status ?? "pending",
          row.errorDetail ?? null,
          ms(row.occurredAt),
        )
      }
    })
    return { ids: ids.map(String) }
  }

  async upsertMessengerCredential(input: {
    userId: string
    clientId: string
    clientSecret: string
    version: string
    verifyToken: string
  }): Promise<{ credentialId: string }> {
    const config = {
      clientId: input.clientId,
      version: input.version,
      verifyToken: input.verifyToken,
      clientSecret: input.clientSecret,
    }
    const publicConfig = { clientId: input.clientId, version: input.version, verifyToken: input.verifyToken }
    const value = encryptObject(config, `user:${input.userId}:messenger:false`)
    const credentialId = createId()
    const now = Date.now()
    const row = this.open()
      .prepare(
        `INSERT INTO "PlatformCredential"
           ("id", "createdAt", "updatedAt", "userId", "type", "publicConfig", "value", "usePlatformCredential",
            "isVerified")
         VALUES (?, ?, ?, ?, 'messenger', ?, ?, 0, 0)
         ON CONFLICT ("userId", "type") WHERE "userId" IS NOT NULL
           DO UPDATE SET "publicConfig" = excluded."publicConfig", "value" = excluded."value",
             "updatedAt" = excluded."updatedAt"
         RETURNING CAST("id" AS TEXT) AS "id"`,
      )
      .get(credentialId, now, now, BigInt(input.userId), JSON.stringify(publicConfig), JSON.stringify(value)) as
      | { id: string }
      | undefined
    return { credentialId: row?.id ?? String(credentialId) }
  }

  async expireConnectSession(input: { sessionId: string }): Promise<void> {
    this.open()
      .prepare(`UPDATE "ConnectSession" SET "expiresAt" = ? WHERE "id" = ?`)
      .run(Date.now() - 1000, BigInt(input.sessionId))
  }

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const create = (): P105Fixtures =>
  new SqliteP105Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
