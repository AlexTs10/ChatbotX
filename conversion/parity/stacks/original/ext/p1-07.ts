// Postgres implementation of the P1.7 fixture extension (original stack).
// Writes the same logical rows the app writes; see the comment per method.
//
// Row sources (baseline 40e733cee):
// - Inbox: inboxService.create (packages/database/src/schema/inbox.ts); no
//   integration row, so no worker job can reach a third party.
// - Inbox + IntegrationWhatsapp / IntegrationMessenger: the channel connect
//   services (inboxService.create + the integration insert), same shape as
//   apps/worker/__tests__/integration/support/seed.ts. `auth` = a plain
//   <Oauth2AuthValue> jsonb (not encrypted), with the WhatsApp auth metadata
//   of packages/business/src/integration-whatsapp/auth-schema.ts
//   (`metadata.phoneNumber.id` is the Graph path of thread_control).
// - Contact + ContactInbox + Conversation: createContactWithInbox
//   (packages/business/src/contact/create-with-inbox.ts) and the inbound
//   message / routing paths, which also fill the ContactInbox activity and
//   thread-control columns (packages/database/src/schema/contact-inbox.ts)
//   and the Conversation activity columns (schema/conversation.ts). The
//   ContactInbox insert trigger keeps InboxContactStat.
// - InboxTeam: inboxTeamService.create (schema/inbox-team.ts).
// - WorkspaceMember (agent): workspace invitation accept
//   (schema/workspace-member.ts), default agent permissions.

import { randomBytes } from "node:crypto"
import pg from "pg"
import {
  P107_META_APP_ID,
  type P107ChannelCreated,
  type P107ContactInput,
  type P107CreatedContact,
  type P107Fixtures,
  type P107InboxInput,
  type P107MessengerChannelInput,
  type P107WhatsappChannelInput,
} from "../../../src/fixtures/ext/p1-07.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
/** App processes use place ids 1-3; fixtures 15 (core), 14, 12, 11, 10; P1.7 uses 9. */
const PLACE_ID = 9n

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

/**
 * The channel integration `auth` jsonb is stored as plain JSON (an
 * `Oauth2AuthValue`); the app reads it with a raw `SELECT *`
 * (integrationLookupRepository.findAuthByInboxId) and never decrypts it.
 */
const oauth2Auth = (accessToken: string, metadata: Record<string, unknown>) => ({
  authType: "oauth2",
  clientId: P107_META_APP_ID,
  clientSecret: "parity-secret",
  redirectUrl: "http://app/integrations/callback",
  version: "v23.0",
  tokens: { accessToken },
  metadata,
})

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
const DEFAULT_MEMBER_NOTIFICATION_TYPES = { notifyAdmin: true, newMessageToHuman: true, newOrder: true }
const DEFAULT_MEMBER_NOTIFICATION_CHANNELS = { messenger: true, email: true, telegram: true, browser: true }

class PostgresP107Fixtures implements P107Fixtures {
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
    client: pg.Pool | pg.PoolClient,
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

  async createInbox(input: P107InboxInput): Promise<{ inboxId: string }> {
    const inboxId = await this.insertInbox(this.pool, {
      workspaceId: input.workspaceId,
      name: input.name,
      channel: input.channel,
      sourceId: input.sourceId ?? uniqueText("src"),
    })
    return { inboxId }
  }

  async createWhatsappChannel(input: P107WhatsappChannelInput): Promise<P107ChannelCreated> {
    const accessToken = `wa-token-${input.phoneNumberId}`
    const wabaId = input.wabaId ?? `waba-${input.phoneNumberId}`
    const businessId = `biz-${input.phoneNumberId}`
    const displayPhoneNumber = input.displayPhoneNumber ?? "+1 555 0100"
    return await this.transaction(async (client) => {
      const inboxId = await this.insertInbox(client, {
        workspaceId: input.workspaceId,
        name: input.name,
        channel: "whatsapp",
        sourceId: input.phoneNumberId,
      })
      const integrationId = createId()
      await client.query(
        `INSERT INTO "IntegrationWhatsapp"
           ("id", "auth", "phoneNumberId", "wabaId", "businessId", "name", "displayPhoneNumber", "workspaceId", "inboxId")
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)`,
        [
          integrationId,
          JSON.stringify(
            oauth2Auth(accessToken, {
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
            }),
          ),
          input.phoneNumberId,
          wabaId,
          businessId,
          input.name,
          displayPhoneNumber,
          input.workspaceId,
          inboxId,
        ],
      )
      return { inboxId, integrationId, accessToken }
    })
  }

  async createMessengerChannel(input: P107MessengerChannelInput): Promise<P107ChannelCreated> {
    const accessToken = `page-token-${input.pageId}`
    return await this.transaction(async (client) => {
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
         VALUES ($1, $2, $3, $4, $5, $6, '[]'::jsonb, '[]'::jsonb, '[]'::jsonb)`,
        [
          integrationId,
          JSON.stringify(oauth2Auth(accessToken, { pageId: input.pageId, pageName: input.name })),
          input.pageId,
          input.name,
          input.workspaceId,
          inboxId,
        ],
      )
      return { inboxId, integrationId, accessToken }
    })
  }

  async createContact(input: P107ContactInput): Promise<P107CreatedContact> {
    const contactId = createId()
    const createdAt = input.createdAt ?? new Date().toISOString()
    const inboxes = input.inboxes ?? []
    const contactInboxIds = inboxes.map(() => createId())
    const sourceIds = inboxes.map((entry) => entry.sourceId ?? uniqueText("psid"))
    const conversationId = input.conversation === null ? null : createId()
    await this.transaction(async (client) => {
      await client.query(
        `INSERT INTO "Contact"
           ("id", "createdAt", "updatedAt", "workspaceId", "firstName", "lastName", "email", "phoneNumber",
            "gender", "avatar", "blockedAt")
         VALUES ($1, $2, $2, $3, $4, $5, $6, $7, $8, $9, $10)`,
        [
          contactId,
          createdAt,
          input.workspaceId,
          input.firstName ?? null,
          input.lastName ?? null,
          input.email ?? null,
          input.phoneNumber ?? null,
          input.gender ?? null,
          input.avatar ?? null,
          input.blockedAt ?? null,
        ],
      )
      for (const [index, entry] of inboxes.entries()) {
        await client.query(
          `INSERT INTO "ContactInbox"
             ("id", "createdAt", "updatedAt", "originalContactId", "contactId", "inboxId", "channel", "source",
              "sourceId", "sourceUserId", "sourceUsername", "language", "contactLastReadAt", "lastMessageAt",
              "lastIncomingMessageAt", "threadControlState", "threadOwnerRole", "threadOwnerAppId",
              "threadControlUpdatedAt", "threadOwnerExpiresAt", "referral")
           VALUES ($1, $2, $2, $3, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)`,
          [
            contactInboxIds[index],
            createdAt,
            contactId,
            entry.inboxId,
            entry.channel,
            entry.source ?? "direct",
            sourceIds[index],
            entry.sourceUserId ?? null,
            entry.sourceUsername ?? null,
            entry.language ?? null,
            entry.contactLastReadAt ?? null,
            entry.lastMessageAt ?? null,
            entry.lastIncomingMessageAt ?? null,
            entry.threadControlState ?? null,
            entry.threadOwnerRole ?? null,
            entry.threadOwnerAppId ?? null,
            entry.threadControlUpdatedAt ?? null,
            entry.threadOwnerExpiresAt ?? null,
            entry.referral === undefined ? null : JSON.stringify(entry.referral),
          ],
        )
      }
      if (conversationId) {
        const conversation = input.conversation ?? {}
        const conversationCreatedAt = conversation.createdAt ?? createdAt
        await client.query(
          `INSERT INTO "Conversation"
             ("id", "createdAt", "updatedAt", "workspaceId", "contactId", "archivedAt", "followed", "botEnabled",
              "lastActivityAt", "agentLastReadAt", "contactLastReadAt", "assignedUserId", "assignedInboxTeamId",
              "adminRepliedAt", "contactRepliedAt", "responseDueAt", "responseSlaBreachingAt")
           VALUES ($1, $2, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)`,
          [
            conversationId,
            conversationCreatedAt,
            input.workspaceId,
            contactId,
            conversation.archivedAt ?? null,
            conversation.followed ?? false,
            conversation.botEnabled ?? true,
            conversation.lastActivityAt ?? null,
            conversation.agentLastReadAt ?? null,
            conversation.contactLastReadAt ?? null,
            conversation.assignedUserId ?? null,
            conversation.assignedInboxTeamId ?? null,
            conversation.adminRepliedAt ?? null,
            conversation.contactRepliedAt ?? null,
            conversation.responseDueAt ?? null,
            conversation.responseSlaBreachingAt ?? null,
          ],
        )
      }
    })
    return { contactId, contactInboxIds, sourceIds, conversationId }
  }

  async createInboxTeam(input: { workspaceId: string; name: string }): Promise<{ inboxTeamId: string }> {
    const inboxTeamId = createId()
    await this.pool.query(`INSERT INTO "InboxTeam" ("id", "workspaceId", "name") VALUES ($1, $2, $3)`, [
      inboxTeamId,
      input.workspaceId,
      input.name,
    ])
    return { inboxTeamId }
  }

  async addMember(input: { workspaceId: string; userId: string }): Promise<{ memberId: string }> {
    const memberId = createId()
    await this.pool.query(
      `INSERT INTO "WorkspaceMember"
         ("id", "workspaceId", "userId", "role", "permissions", "notificationTypes", "notificationChannels")
       VALUES ($1, $2, $3, 'agent', $4, $5, $6)`,
      [
        memberId,
        input.workspaceId,
        input.userId,
        JSON.stringify(AGENT_PERMISSIONS),
        JSON.stringify(DEFAULT_MEMBER_NOTIFICATION_TYPES),
        JSON.stringify(DEFAULT_MEMBER_NOTIFICATION_CHANNELS),
      ],
    )
    return { memberId }
  }

  async close(): Promise<void> {
    await this.pool.end()
  }
}

export const create = (): P107Fixtures =>
  new PostgresP107Fixtures(process.env.PARITY_PG_URL ?? "postgresql://chatbotx:secretkey@postgres:5432/chatbotx")
