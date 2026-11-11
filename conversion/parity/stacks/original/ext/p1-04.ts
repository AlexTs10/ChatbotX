// Postgres implementation of the P1.4 fixture extension for the original
// stack. Writes the same logical rows as the app's own code paths (cited per
// method), straight to the database. Setup only; assertions stay black-box.
//
// Row sources (baseline 40e733cee):
// - Integration + IntegrationGoogleCalendar: appointmentExternalCalendarService
//   .createGoogleFromOAuthCallback (packages/business/src/appointment-external-calendar/service.ts);
//   auth = oauth2 auth value (packages/sdk/src/lib/auth/index.ts, googleCalendarAuthSchema).
// - Inbox + IntegrationZalo: Zalo OA connect (packages/database/src/schema/integration-zalo.ts).
// - ChannelPost: channelPostRepository.insertBare + saveMetadata
//   (packages/database/src/repositories/channel-post/repository.ts).
// - Cart: cartRepository.upsert (packages/database/src/repositories/cart/repository.ts).
// - Appointment: appointmentRepository.create (packages/database/src/repositories/appointment/repository.ts).

import { randomBytes } from "node:crypto"
import pg from "pg"
import type {
  AppointmentInput,
  ChannelPostInput,
  GoogleCalendarConnectionInput,
  P104FixtureExt,
} from "../../../src/fixtures/ext/p1-04.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
/** App processes use place ids 1-3, the core fixture driver 15; this unit 14. */
const EXT_PLACE_ID = 14n

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
  return ((now << 14n) | (EXT_PLACE_ID << 10n) | sequence).toString()
}

class PostgresP104Ext implements P104FixtureExt {
  private readonly pool: pg.Pool

  constructor(connectionString: string) {
    this.pool = new pg.Pool({
      connectionString,
      max: 2,
      idleTimeoutMillis: 1_000,
      allowExitOnIdle: true,
    })
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

  async createGoogleCalendarConnection(
    input: GoogleCalendarConnectionInput,
  ): Promise<{ integrationId: string }> {
    const integrationId = createId()
    const auth = {
      authType: "oauth2",
      clientId: "parity-google-client-id",
      clientSecret: "parity-google-client-secret",
      redirectUrl: "http://app/api/integrations/googleCalendar/callback",
      tokens: {
        accessToken: input.accessToken,
        refreshToken: input.refreshToken,
        // Far in the future: the client never refreshes during a test.
        expiresAt: "2099-01-01T00:00:00.000Z",
      },
      metadata: {
        providerCalendarId: input.providerCalendarId,
        ...(input.email ? { email: input.email } : {}),
      },
    }
    await this.transaction(async (client) => {
      await client.query(
        `INSERT INTO "Integration" ("id", "workspaceId", "integrationType")
         VALUES ($1, $2, 'googleCalendar')`,
        [integrationId, input.workspaceId],
      )
      await client.query(
        `INSERT INTO "IntegrationGoogleCalendar"
           ("id", "workspaceId", "integrationId", "auth", "providerCalendarId", "email")
         VALUES ($1, $2, $3, $4, $5, $6)`,
        [
          createId(),
          input.workspaceId,
          integrationId,
          JSON.stringify(auth),
          input.providerCalendarId,
          input.email,
        ],
      )
    })
    return { integrationId }
  }

  async createInbox(input: {
    workspaceId: string
    channel: string
    name: string
    sourceId: string
  }): Promise<{ inboxId: string }> {
    const inboxId = createId()
    await this.pool.query(
      `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId")
       VALUES ($1, $2, $3, $4, $5)`,
      [inboxId, input.name, input.channel, input.sourceId, input.workspaceId],
    )
    return { inboxId }
  }

  async createZaloIntegration(input: {
    workspaceId: string
    name: string
    oaId: string
  }): Promise<{ integrationId: string; inboxId: string }> {
    const inboxId = createId()
    const integrationId = createId()
    await this.transaction(async (client) => {
      await client.query(
        `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId")
         VALUES ($1, $2, 'zalo', $3, $4)`,
        [inboxId, input.name, input.oaId, input.workspaceId],
      )
      await client.query(
        `INSERT INTO "IntegrationZalo" ("id", "auth", "oaId", "name", "workspaceId", "inboxId")
         VALUES ($1, $2, $3, $4, $5, $6)`,
        [
          integrationId,
          JSON.stringify({ authType: "secretText", secretText: "parity-zalo-token" }),
          input.oaId,
          input.name,
          input.workspaceId,
          inboxId,
        ],
      )
    })
    return { integrationId, inboxId }
  }

  async createChannelPost(input: ChannelPostInput): Promise<{ channelPostId: string }> {
    const channelPostId = createId()
    await this.pool.query(
      `INSERT INTO "ChannelPost"
         ("id", "workspaceId", "inboxId", "channel", "integrationId", "sourceAccountId",
          "externalPostId", "caption", "thumbnail", "permalink", "publishedAt", "metadataFetchedAt")
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now())`,
      [
        channelPostId,
        input.workspaceId,
        input.inboxId,
        input.channel,
        input.integrationId,
        input.sourceAccountId,
        input.externalPostId,
        input.caption,
        input.thumbnail,
        input.permalink,
        input.publishedAt,
      ],
    )
    return { channelPostId }
  }

  async createCart(input: {
    workspaceId: string
    contactId: string
    items: { productId: string; variantId: string | null; quantity: number }[]
  }): Promise<{ cartId: string }> {
    const cartId = createId()
    await this.pool.query(
      `INSERT INTO "Cart" ("id", "workspaceId", "contactId", "items")
       VALUES ($1, $2, $3, $4)`,
      [cartId, input.workspaceId, input.contactId, JSON.stringify(input.items)],
    )
    return { cartId }
  }

  async createAppointment(input: AppointmentInput): Promise<{ appointmentId: string }> {
    const appointmentId = createId()
    await this.pool.query(
      `INSERT INTO "Appointment"
         ("id", "workspaceId", "calendarId", "contactId", "startAt", "endAt",
          "inviteeTimezone", "status", "locationType", "cancelledAt")
       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)`,
      [
        appointmentId,
        input.workspaceId,
        input.calendarId,
        input.contactId,
        input.startAt,
        input.endAt,
        input.inviteeTimezone,
        input.status,
        input.locationType,
        input.status === "cancelled" ? new Date() : null,
      ],
    )
    return { appointmentId }
  }

  async conversationIdForContact(input: { workspaceId: string; contactId: string }): Promise<string> {
    const result = await this.pool.query<{ id: string }>(
      `SELECT "id"::text AS "id" FROM "Conversation"
       WHERE "workspaceId" = $1 AND "contactId" = $2 AND "sourceId" IS NULL
       ORDER BY "id" LIMIT 1`,
      [input.workspaceId, input.contactId],
    )
    const id = result.rows[0]?.id
    if (!id) {
      throw new Error(`conversationIdForContact: no conversation for contact ${input.contactId}`)
    }
    return id
  }

  async close(): Promise<void> {
    await this.pool.end()
  }
}

export const create = (): P104FixtureExt =>
  new PostgresP104Ext(
    process.env.PARITY_PG_URL ?? "postgresql://chatbotx:secretkey@postgres:5432/chatbotx",
  )
