// Postgres implementation of the P1.10 fixture extension (original stack).
// Writes the same logical rows the app writes; setup only, assertions stay
// black-box. Row sources (baseline 40e733cee), copied in part from the P1.6,
// P1.7 and P1.9 fixtures (units do not share files):
// - Inbox: inboxService.create (packages/database/src/schema/inbox.ts).
// - IntegrationInstagram + Connection: the Instagram connect flow
//   (schema/integration-instagram.ts, schema/connection.ts); auth is plain
//   JSON (oauth2 value of integrations/instagram*/src/schema.ts). The CAPI and
//   token-refresh columns are the ones the CAPI actions and the daily token
//   refresh cron write (meta-conversions repository, refreshChannelTokens).
// - IntegrationMessenger: the Messenger connect flow (schema/integration-messenger.ts).
// - IntegrationZalo: the Zalo connect flow (schema/integration-zalo.ts).
// - Integration: the connection engine's workspace-integration binding
//   (packages/business/src/connection/store-bindings.ts).
// - Contact + ContactInbox + Conversation: createContactWithInbox
//   (packages/business/src/contact/create-with-inbox.ts).
// - TelegramChat: recordTelegramBotChatStatus
//   (packages/business/src/leaderboard/telegram-chats.ts).

import { randomBytes } from "node:crypto"
import pg from "pg"
import type {
  P110ContactInput,
  P110Fixtures,
  P110InboxInput,
  P110InstagramInput,
} from "../../../src/fixtures/ext/p1-10.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
/** App processes use place ids 1-3; fixtures 15 (core), 7-12, 14; P1.10 uses 6. */
const PLACE_ID = 6n

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

const uniqueText = (prefix: string) => `${prefix}-${Date.now().toString(36)}${randomBytes(4).toString("hex")}`

type Queryable = pg.Pool | pg.PoolClient

class PostgresP110Fixtures implements P110Fixtures {
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

  private async insertInbox(client: Queryable, input: P110InboxInput): Promise<string> {
    const inboxId = createId()
    const status = input.status ?? "connected"
    await client.query(
      `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status", "disconnectedAt")
       VALUES ($1, $2, $3, $4, $5, $6, $7)`,
      [
        inboxId,
        input.name,
        input.channel,
        input.sourceId ?? uniqueText("src"),
        input.workspaceId,
        status,
        status === "disconnected" ? new Date().toISOString() : null,
      ],
    )
    return inboxId
  }

  async createInbox(input: P110InboxInput): Promise<{ inboxId: string }> {
    return { inboxId: await this.insertInbox(this.pool, input) }
  }

  async createInstagramChannel(input: P110InstagramInput): Promise<{ integrationId: string; inboxId: string }> {
    return this.transaction(async (client) => {
      const inboxId = await this.insertInbox(client, {
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
            "conversationStarters", "persistentMenus", "hasCapiScope", "datasetId", "capiTestEventCode",
            "capiDisconnectedAt", "tokenRefreshError")
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, '{}'::jsonb[], '{}'::jsonb[], $10, $11, $12, $13, $14)`,
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
          input.hasCapiScope ?? false,
          input.datasetId ?? null,
          input.capiTestEventCode ?? null,
          input.capiDisconnected ? new Date().toISOString() : null,
          input.tokenRefreshError ?? null,
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
  }

  async createMessengerPage(input: { workspaceId: string; pageId: string; name: string }) {
    return this.transaction(async (client) => {
      const inboxId = await this.insertInbox(client, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "messenger",
        sourceId: input.pageId,
      })
      const integrationId = createId()
      const auth = {
        authType: "oauth2",
        clientId: "parity-client",
        clientSecret: "parity-secret",
        redirectUrl: "http://app/integrations/callback",
        version: "v23.0",
        tokens: { accessToken: `page-token-${input.pageId}` },
        metadata: { pageId: input.pageId, pageName: input.name },
      }
      await client.query(
        `INSERT INTO "IntegrationMessenger"
           ("id", "auth", "pageId", "name", "workspaceId", "inboxId", "conversationStarters", "persistentMenus", "personas")
         VALUES ($1, $2, $3, $4, $5, $6, '[]'::jsonb, '[]'::jsonb, '[]'::jsonb)`,
        [integrationId, JSON.stringify(auth), input.pageId, input.name, input.workspaceId, inboxId],
      )
      return { integrationId, inboxId }
    })
  }

  async createZaloIntegration(input: {
    workspaceId: string
    name: string
    oaId: string
    tokenRefreshError?: string | null
  }) {
    return this.transaction(async (client) => {
      const inboxId = await this.insertInbox(client, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "zalo",
        sourceId: input.oaId,
      })
      const integrationId = createId()
      await client.query(
        `INSERT INTO "IntegrationZalo" ("id", "auth", "oaId", "name", "workspaceId", "inboxId", "tokenRefreshError")
         VALUES ($1, $2, $3, $4, $5, $6, $7)`,
        [
          integrationId,
          JSON.stringify({ authType: "secretText", secretText: "parity-zalo-token" }),
          input.oaId,
          input.name,
          input.workspaceId,
          inboxId,
          input.tokenRefreshError ?? null,
        ],
      )
      return { integrationId, inboxId }
    })
  }

  async createIntegration(input: { workspaceId: string; integrationType: string }) {
    const integrationId = createId()
    await this.pool.query(`INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES ($1, $2, $3)`, [
      integrationId,
      input.workspaceId,
      input.integrationType,
    ])
    return { integrationId }
  }

  async createContact(input: P110ContactInput) {
    const contactId = createId()
    const createdAt = new Date().toISOString()
    const inboxes = input.inboxes ?? []
    const contactInboxIds = inboxes.map(() => createId())
    const conversationId = input.conversation === false ? null : createId()
    await this.transaction(async (client) => {
      await client.query(
        `INSERT INTO "Contact" ("id", "createdAt", "updatedAt", "workspaceId", "firstName", "lastName", "phoneNumber", "avatar")
         VALUES ($1, $2, $2, $3, $4, $5, $6, $7)`,
        [
          contactId,
          createdAt,
          input.workspaceId,
          input.firstName ?? null,
          input.lastName ?? null,
          input.phoneNumber ?? null,
          input.avatar ?? null,
        ],
      )
      for (const [index, entry] of inboxes.entries()) {
        await client.query(
          `INSERT INTO "ContactInbox"
             ("id", "createdAt", "updatedAt", "originalContactId", "contactId", "inboxId", "channel", "source", "sourceId")
           VALUES ($1, $2, $2, $3, $3, $4, $5, 'direct', $6)`,
          [contactInboxIds[index], createdAt, contactId, entry.inboxId, entry.channel, entry.sourceId ?? uniqueText("psid")],
        )
      }
      if (conversationId) {
        await client.query(
          `INSERT INTO "Conversation" ("id", "createdAt", "updatedAt", "workspaceId", "contactId")
           VALUES ($1, $2, $2, $3, $4)`,
          [conversationId, createdAt, input.workspaceId, contactId],
        )
      }
    })
    return { contactId, contactInboxIds, conversationId }
  }

  async createTelegramChat(input: {
    workspaceId: string
    inboxId: string
    chatId: string
    title: string | null
    type: string
    botStatus: string
  }): Promise<void> {
    await this.pool.query(
      `INSERT INTO "TelegramChat" ("id", "workspaceId", "inboxId", "chatId", "title", "type", "botStatus")
       VALUES ($1, $2, $3, $4, $5, $6, $7)`,
      [createId(), input.workspaceId, input.inboxId, input.chatId, input.title, input.type, input.botStatus],
    )
  }

  async close(): Promise<void> {
    await this.pool.end()
  }
}

export const create = (): P110Fixtures =>
  new PostgresP110Fixtures(process.env.PARITY_PG_URL ?? "postgresql://chatbotx:secretkey@postgres:5432/chatbotx")
