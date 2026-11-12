// Postgres implementation of the P1.11 fixture extension (original stack).
// Writes the same logical rows the app writes; setup only, assertions stay
// black-box. Row sources (baseline 40e733cee), copied in part from the P1.7,
// P1.8 and P1.10 fixtures (units do not share files):
// - Inbox: inboxService.create (packages/database/src/schema/inbox.ts).
// - IntegrationMessenger + Connection: the Messenger page connect
//   (schema/integration-messenger.ts, schema/connection.ts); auth is the plain
//   JSON oauth2 value of integrations/messenger/src/schema.ts. Personas,
//   personaId and the CAPI / token-refresh columns are the ones the settings
//   save, the CAPI actions and the daily token refresh write.
// - IntegrationInstagram (type facebook): the Instagram-via-Facebook connect
//   (schema/integration-instagram.ts).
// - IntegrationWhatsapp: the WhatsApp embedded signup
//   (schema/integration-whatsapp.ts), auth metadata of
//   packages/business/src/integration-whatsapp/auth-schema.ts.
// - WhatsappMessageTemplate / MessengerMessageTemplate: the template syncs
//   (whatsappMessageTemplateService / messengerMessageTemplateService.syncFromMeta).
// - Contact + ContactInbox + Conversation: createContactWithInbox
//   (packages/business/src/contact/create-with-inbox.ts) and the inbound
//   message path (ContactInbox.lastIncomingMessageAt).
// - Message: the inbound webhook / outgoing send paths (schema/message.ts),
//   in the main database (the parity stack registers no message shard).
// - User.mustChangePassword: the member provisioning with a temporary
//   password (userService; cleared by me.changePassword).

import { randomBytes } from "node:crypto"
import pg from "pg"
import type {
  P111Contact,
  P111ContactInput,
  P111Fixtures,
  P111MessageInput,
  P111MessengerPage,
  P111MessengerPageInput,
  P111WhatsappChannel,
  P111WhatsappChannelInput,
} from "../../../src/fixtures/ext/p1-11.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
/** App processes use place ids 1-3; fixtures 15 (core), 6-12, 14; P1.11 uses 5. */
const PLACE_ID = 5n

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
const uniqueDigits = () => `${Date.now()}${randomBytes(3).readUIntBE(0, 3).toString().padStart(8, "0")}`

const oauth2Auth = (accessToken: string, metadata: Record<string, unknown>, redirectUrl: string) => ({
  authType: "oauth2",
  clientId: "parity-client",
  clientSecret: "parity-secret",
  redirectUrl,
  version: "v23.0",
  tokens: { accessToken },
  metadata,
})

type Queryable = pg.Pool | pg.PoolClient

class PostgresP111Fixtures implements P111Fixtures {
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
    client: Queryable,
    input: { workspaceId: string; name: string; channel: string; sourceId: string },
  ): Promise<string> {
    const inboxId = createId()
    await client.query(
      `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status")
       VALUES ($1, $2, $3, $4, $5, 'connected')`,
      [inboxId, input.name, input.channel, input.sourceId, input.workspaceId],
    )
    return inboxId
  }

  async createMessengerPage(input: P111MessengerPageInput): Promise<P111MessengerPage> {
    const accessToken = `page-token-${input.pageId}`
    return await this.transaction(async (client) => {
      const inboxId = await this.insertInbox(client, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "messenger",
        sourceId: input.pageId,
      })
      const integrationId = createId()
      const auth = oauth2Auth(
        accessToken,
        { pageId: input.pageId, pageName: input.name, version: "v23.0" },
        "http://app/integrations/messenger/callback",
      )
      await client.query(
        `INSERT INTO "IntegrationMessenger"
           ("id", "auth", "pageId", "name", "workspaceId", "inboxId", "conversationStarters", "persistentMenus",
            "personas", "personaId", "hasCapiScope", "datasetId", "capiTestEventCode", "capiDisconnectedAt",
            "tokenRefreshError")
         VALUES ($1, $2, $3, $4, $5, $6, '[]'::jsonb, '[]'::jsonb, $7, $8, $9, $10, $11, $12, $13)`,
        [
          integrationId,
          JSON.stringify(auth),
          input.pageId,
          input.name,
          input.workspaceId,
          inboxId,
          JSON.stringify(input.personas ?? []),
          input.personaId ?? null,
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
         VALUES ($1, $2, 'messenger', 'channel', 'messenger', $3, $4, $5, 'connected', now())`,
        [createId(), input.workspaceId, inboxId, input.pageId, input.name],
      )
      return { integrationId, inboxId, accessToken }
    })
  }

  async createInstagramForPage(input: { workspaceId: string; pageId: string; igId: string; name: string }) {
    return await this.transaction(async (client) => {
      const inboxId = await this.insertInbox(client, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "instagram",
        sourceId: input.igId,
      })
      const integrationId = createId()
      const auth = oauth2Auth(
        `ig-token-${input.igId}`,
        { igId: input.igId, pageId: input.pageId, version: "v23.0" },
        "http://app/integrations/instagram/callback",
      )
      await client.query(
        `INSERT INTO "IntegrationInstagram"
           ("id", "auth", "igId", "pageId", "name", "username", "workspaceId", "inboxId", "type",
            "conversationStarters", "persistentMenus")
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'facebook', '{}'::jsonb[], '{}'::jsonb[])`,
        [
          integrationId,
          JSON.stringify(auth),
          input.igId,
          input.pageId,
          input.name,
          `parity_${input.igId}`,
          input.workspaceId,
          inboxId,
        ],
      )
      await client.query(
        `INSERT INTO "Connection"
           ("id", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId", "displayName", "status", "connectedAt")
         VALUES ($1, $2, 'instagramFacebook', 'channel', 'instagram', $3, $4, $5, 'connected', now())`,
        [createId(), input.workspaceId, inboxId, input.igId, input.name],
      )
      return { integrationId, inboxId }
    })
  }

  async createWhatsappChannel(input: P111WhatsappChannelInput): Promise<P111WhatsappChannel> {
    const accessToken = `wa-token-${input.phoneNumberId}`
    const wabaId = `waba-${input.phoneNumberId}`
    const businessId = `biz-${input.phoneNumberId}`
    const displayPhoneNumber = "+1 555 0100"
    return await this.transaction(async (client) => {
      const inboxId = await this.insertInbox(client, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "whatsapp",
        sourceId: input.phoneNumberId,
      })
      const integrationId = createId()
      const auth = oauth2Auth(
        accessToken,
        {
          wabaId,
          businessId,
          webhookUrl: "http://app/api/hooks/whatsapp",
          phoneNumber: {
            id: input.phoneNumberId,
            verified_name: input.name,
            code_verification_status: "VERIFIED",
            display_phone_number: displayPhoneNumber,
            quality_rating: "GREEN",
            platform_type: "CLOUD_API",
            throughput: { level: "STANDARD" },
            webhook_configuration: {},
          },
        },
        "http://app/integrations/callback",
      )
      await client.query(
        `INSERT INTO "IntegrationWhatsapp"
           ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "displayPhoneNumber", "workspaceId", "inboxId")
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)`,
        [
          integrationId,
          JSON.stringify(auth),
          input.phoneNumberId,
          wabaId,
          businessId,
          input.name,
          displayPhoneNumber,
          input.workspaceId,
          inboxId,
        ],
      )
      await client.query(
        `INSERT INTO "Connection"
           ("id", "workspaceId", "provider", "kind", "channel", "inboxId", "sourceId", "displayName", "status", "connectedAt")
         VALUES ($1, $2, 'whatsapp', 'channel', 'whatsapp', $3, $4, $5, 'connected', now())`,
        [createId(), input.workspaceId, inboxId, input.phoneNumberId, input.name],
      )
      return { integrationId, inboxId, accessToken }
    })
  }

  async createWhatsappTemplate(input: {
    integrationWhatsappId: string
    name: string
    language: string
    status: string
    components: unknown[]
  }): Promise<{ templateId: string }> {
    const templateId = createId()
    await this.pool.query(
      `INSERT INTO "WhatsappMessageTemplate"
         ("id", "name", "integrationWhatsappId", "sourceId", "language", "category", "status", "components")
       VALUES ($1, $2, $3, $4, $5, 'UTILITY', $6, $7)`,
      [
        templateId,
        input.name,
        input.integrationWhatsappId,
        uniqueDigits(),
        input.language,
        input.status,
        JSON.stringify(input.components),
      ],
    )
    return { templateId }
  }

  async createMessengerTemplate(input: {
    integrationMessengerId: string
    name: string
    language: string
    status: string
    category: string
    components: unknown[]
    sourceId?: string
  }): Promise<{ templateId: string }> {
    const templateId = createId()
    await this.pool.query(
      `INSERT INTO "MessengerMessageTemplate"
         ("id", "name", "integrationMessengerId", "sourceId", "language", "category", "status", "parameterFormat", "components")
       VALUES ($1, $2, $3, $4, $5, $6, $7, 'POSITIONAL', $8)`,
      [
        templateId,
        input.name,
        input.integrationMessengerId,
        input.sourceId ?? uniqueDigits(),
        input.language,
        input.category,
        input.status,
        JSON.stringify(input.components),
      ],
    )
    return { templateId }
  }

  async createContact(input: P111ContactInput): Promise<P111Contact> {
    const contactId = createId()
    const createdAt = new Date().toISOString()
    const contactInboxIds = input.inboxes.map(() => createId())
    const sourceIds = input.inboxes.map((entry) => entry.sourceId ?? uniqueText("psid"))
    const conversationId = createId()
    await this.transaction(async (client) => {
      await client.query(
        `INSERT INTO "Contact" ("id", "createdAt", "updatedAt", "workspaceId", "firstName", "lastName")
         VALUES ($1, $2, $2, $3, $4, 'Parity')`,
        [contactId, createdAt, input.workspaceId, input.firstName],
      )
      for (const [index, entry] of input.inboxes.entries()) {
        await client.query(
          `INSERT INTO "ContactInbox"
             ("id", "createdAt", "updatedAt", "originalContactId", "contactId", "inboxId", "channel", "source",
              "sourceId", "lastIncomingMessageAt")
           VALUES ($1, $2, $2, $3, $3, $4, $5, 'direct', $6, $7)`,
          [
            contactInboxIds[index],
            createdAt,
            contactId,
            entry.inboxId,
            entry.channel,
            sourceIds[index],
            entry.lastIncomingMessageAt ?? null,
          ],
        )
      }
      await client.query(
        `INSERT INTO "Conversation" ("id", "createdAt", "updatedAt", "workspaceId", "contactId")
         VALUES ($1, $2, $2, $3, $4)`,
        [conversationId, createdAt, input.workspaceId, contactId],
      )
    })
    return { contactId, contactInboxIds, sourceIds, conversationId }
  }

  async createMessage(input: P111MessageInput): Promise<{ messageId: string; createdAt: string }> {
    const messageId = createId()
    const createdAt = input.createdAt ?? new Date().toISOString()
    await this.pool.query(
      `INSERT INTO "Message"
         ("id", "createdAt", "updatedAt", "conversationId", "contactInboxId", "workspaceId", "text",
          "contentAttributes", "messageType", "contentType", "senderType", "sourceId", "type", "parentId")
       VALUES ($1, $2, $2, $3, $4, $5, $6, $7, $8, 'text', $9, $10, $11, $12)`,
      [
        messageId,
        createdAt,
        input.conversationId,
        input.contactInboxId,
        input.workspaceId,
        input.text ?? null,
        input.contentAttributes ? JSON.stringify(input.contentAttributes) : null,
        input.messageType,
        input.senderType,
        input.sourceId ?? null,
        input.type ?? "message",
        input.parentId ?? null,
      ],
    )
    return { messageId, createdAt }
  }

  async setMustChangePassword(input: { userId: string; value: boolean }): Promise<void> {
    await this.pool.query(`UPDATE "User" SET "mustChangePassword" = $2 WHERE "id" = $1`, [input.userId, input.value])
  }

  async close(): Promise<void> {
    await this.pool.end()
  }
}

export const create = (): P111Fixtures =>
  new PostgresP111Fixtures(process.env.PARITY_PG_URL ?? "postgresql://chatbotx:secretkey@postgres:5432/chatbotx")
