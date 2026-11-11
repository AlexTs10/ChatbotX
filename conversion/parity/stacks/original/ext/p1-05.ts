// Postgres implementation of the P1.5 fixture extension (original stack).
// Writes the same logical rows the app writes; see the comment per method.
//
// Row sources (baseline 40e733cee):
// - Inbox + IntegrationMessenger/-Instagram + Connection: the channel connect
//   path (`connectAndPersist` in packages/connections/src/internal.ts:
//   inboxService.create, the store binding's insertRow
//   (packages/business/src/connection/store-bindings-channels.ts) and
//   `upsertConnectionRow`). Channel `auth` is plain JSON (same shape as
//   apps/worker/__tests__/integration/support/seed.ts).
// - Contact + ContactInbox + Conversation: createContactWithInbox
//   (packages/business/src/contact/create-with-inbox.ts).
// - CommentAutomation: commentAutomationService create (schema defaults of
//   packages/database/src/schema/comment-automation.ts).
// - CommentModerationLog: the moderation recorder
//   (packages/database/src/schema/comment-moderation-log.ts).
// - PlatformCredential (user-owned): platformCredentialService.upsertForUser
//   (packages/business/src/platform-credential/service.ts): value =
//   encryptUtils.encryptObject(config, "user:<userId>:<type>:false"),
//   publicConfig = the public fields.
// Encryption: AES-256-GCM, 12-byte IV, 16-byte tag, hex fields,
// {v:1, kid, iv, text, tag, aad} (packages/encryption/src/encryption.ts),
// key = ENCRYPTION_KEY of stacks/original/original.env.

import { createCipheriv, randomBytes } from "node:crypto"
import { readFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import pg from "pg"
import type {
  CreateCommentAutomationInput,
  CreatedChannel,
  ModerationLogRow,
  P105Fixtures,
} from "../../../src/fixtures/ext/p1-05.ts"

const here = path.dirname(fileURLToPath(import.meta.url))
const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
/** App processes use place ids 1-3; core fixtures 15, P1.2 14, P1.3 12; P1.5 uses 11. */
const PLACE_ID = 11n

let lastMs = 0n
let sequence = 0n
const createId = (): string => {
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

/** `messagingProfileDefaults()` of store-bindings-channels.ts (IntegrationMessenger: jsonb; IntegrationInstagram: jsonb[]). */
const PROFILE_DEFAULTS_SQL = `'[]'::jsonb, '[]'::jsonb`

const oauth2Auth = (accessToken: string, metadata: Record<string, unknown>) => ({
  authType: "oauth2",
  clientId: "parity-client",
  clientSecret: "parity-secret",
  redirectUrl: "http://app/integrations/messenger/callback",
  version: "v23.0",
  tokens: { accessToken },
  metadata,
})

class PostgresP105Fixtures implements P105Fixtures {
  private readonly pool: pg.Pool

  constructor(connectionString: string) {
    this.pool = new pg.Pool({ connectionString, max: 2, idleTimeoutMillis: 1_000, allowExitOnIdle: true })
  }

  private async transaction<T>(fn: (client: pg.PoolClient) => Promise<T>): Promise<T> {
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

  private async insertInbox(
    client: pg.PoolClient | pg.Pool,
    input: { workspaceId: string; name: string; channel: string; sourceId: string; status?: string },
  ): Promise<string> {
    const inboxId = createId()
    const status = input.status ?? "connected"
    await client.query(
      `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status", "disconnectedAt")
       VALUES ($1, $2, $3, $4, $5, $6, $7)`,
      [
        inboxId,
        input.name,
        input.channel,
        input.sourceId,
        input.workspaceId,
        status,
        status === "disconnected" ? new Date().toISOString() : null,
      ],
    )
    return inboxId
  }

  private async insertChannelConnection(
    client: pg.PoolClient,
    input: { workspaceId: string; provider: string; channel: string; inboxId: string; sourceId: string; displayName: string },
  ): Promise<string> {
    const connectionId = createId()
    await client.query(
      `INSERT INTO "Connection"
         ("id", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId", "displayName", "status", "connectedAt")
       VALUES ($1, $2, $3, 'channel', $4, $5, $6, $7, 'connected', now())`,
      [connectionId, input.workspaceId, input.provider, input.channel, input.inboxId, input.sourceId, input.displayName],
    )
    return connectionId
  }

  async createMessengerChannel(input: { workspaceId: string; pageId: string; name: string }): Promise<CreatedChannel> {
    return this.transaction(async (client) => {
      const inboxId = await this.insertInbox(client, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "messenger",
        sourceId: input.pageId,
      })
      const integrationId = createId()
      await client.query(
        `INSERT INTO "IntegrationMessenger"
           ("id", "auth", "pageId", "name", "workspaceId", "inboxId", "conversationStarters", "persistentMenus", "personas")
         VALUES ($1, $2, $3, $4, $5, $6, ${PROFILE_DEFAULTS_SQL}, '[]'::jsonb)`,
        [
          integrationId,
          JSON.stringify(
            oauth2Auth(`page-token-${input.pageId}`, { pageId: input.pageId, pageName: input.name, version: "v23.0" }),
          ),
          input.pageId,
          input.name,
          input.workspaceId,
          inboxId,
        ],
      )
      const connectionId = await this.insertChannelConnection(client, {
        workspaceId: input.workspaceId,
        provider: "messenger",
        channel: "messenger",
        inboxId,
        sourceId: input.pageId,
        displayName: input.name,
      })
      return { integrationId, inboxId, connectionId }
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
    return this.transaction(async (client) => {
      const inboxId = await this.insertInbox(client, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "instagram",
        sourceId: input.igId,
      })
      const integrationId = createId()
      await client.query(
        `INSERT INTO "IntegrationInstagram"
           ("id", "auth", "igId", "pageId", "name", "username", "workspaceId", "inboxId", "type",
            "conversationStarters", "persistentMenus")
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, '{}'::jsonb[], '{}'::jsonb[])`,
        [
          integrationId,
          JSON.stringify(oauth2Auth(`ig-token-${input.igId}`, { igId: input.igId, pageId: input.pageId, version: "v23.0" })),
          input.igId,
          input.pageId,
          input.name,
          input.username,
          input.workspaceId,
          inboxId,
          input.type,
        ],
      )
      const connectionId = await this.insertChannelConnection(client, {
        workspaceId: input.workspaceId,
        provider: input.type === "facebook" ? "instagramFacebook" : "instagram",
        channel: "instagram",
        inboxId,
        sourceId: input.igId,
        displayName: input.name,
      })
      return { integrationId, inboxId, connectionId }
    })
  }

  async createInbox(input: {
    workspaceId: string
    channel: string
    name: string
    sourceId: string
    status?: "connected" | "disconnected"
  }): Promise<{ inboxId: string }> {
    return { inboxId: await this.insertInbox(this.pool, input) }
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
    await this.transaction(async (client) => {
      await client.query(
        `INSERT INTO "Contact" ("id", "workspaceId", "firstName", "lastName") VALUES ($1, $2, $3, $4)`,
        [contactId, input.workspaceId, input.firstName ?? null, input.lastName ?? null],
      )
      await client.query(
        `INSERT INTO "ContactInbox"
           ("id", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
         VALUES ($1, $2, $2, $3, $4, 'direct', $5)`,
        [contactInboxId, contactId, input.inboxId, input.channel, input.sourceId],
      )
      await client.query(`INSERT INTO "Conversation" ("id", "workspaceId", "contactId") VALUES ($1, $2, $3)`, [
        conversationId,
        input.workspaceId,
        contactId,
      ])
    })
    return { contactId, contactInboxId, conversationId }
  }

  async createCommentAutomation(input: CreateCommentAutomationInput): Promise<{ automationId: string }> {
    const automationId = createId()
    await this.pool.query(
      `INSERT INTO "CommentAutomation"
         ("id", "name", "workspaceId", "type", "inboxId", "isActive", "startTime", "endTime",
          "post", "publicReply", "privateReply")
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)`,
      [
        automationId,
        input.name,
        input.workspaceId,
        input.type,
        input.inboxId ?? null,
        input.isActive ?? true,
        input.startTime ?? null,
        input.endTime ?? null,
        JSON.stringify(input.post ?? { type: "all", value: [] }),
        JSON.stringify(input.publicReply ?? { type: "none", value: null }),
        JSON.stringify(input.privateReply ?? { type: "none", value: null }),
      ],
    )
    return { automationId }
  }

  async insertModerationLogs(rows: ModerationLogRow[]): Promise<{ ids: string[] }> {
    const ids = rows.map(() => createId())
    for (const [index, row] of rows.entries()) {
      await this.pool.query(
        `INSERT INTO "CommentModerationLog"
           ("id", "workspaceId", "automationId", "contactId", "contactInboxId", "postId", "commentId", "commentText",
            "source", "rule", "matchedKeyword", "categories", "reason", "action", "status", "errorDetail", "occurredAt")
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::text[], $13, $14, $15, $16, $17)`,
        [
          ids[index],
          row.workspaceId,
          row.automationId,
          row.contactId ?? null,
          row.contactInboxId ?? null,
          row.postId,
          row.commentId,
          row.commentText ?? null,
          row.source,
          row.rule ?? null,
          row.matchedKeyword ?? null,
          row.categories ?? [],
          row.reason ?? null,
          row.action,
          row.status ?? "pending",
          row.errorDetail ?? null,
          row.occurredAt,
        ],
      )
    }
    return { ids }
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
    const result = await this.pool.query<{ id: string }>(
      `INSERT INTO "PlatformCredential" ("id", "userId", "type", "publicConfig", "value", "usePlatformCredential", "isVerified")
       VALUES ($1, $2, 'messenger', $3, $4, false, false)
       RETURNING "id"::text AS "id"`,
      [credentialId, input.userId, JSON.stringify(publicConfig), JSON.stringify(value)],
    )
    return { credentialId: result.rows[0]?.id ?? credentialId }
  }

  async expireConnectSession(input: { sessionId: string }): Promise<void> {
    await this.pool.query(`UPDATE "ConnectSession" SET "expiresAt" = now() - interval '1 second' WHERE "id" = $1`, [
      input.sessionId,
    ])
  }

  async close(): Promise<void> {
    await this.pool.end()
  }
}

export const create = (): P105Fixtures =>
  new PostgresP105Fixtures(process.env.PARITY_PG_URL ?? "postgresql://chatbotx:secretkey@postgres:5432/chatbotx")
