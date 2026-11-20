// Rust-stack implementation of the P1.4 fixture extension
// (`src/fixtures/ext/p1-04.ts`): reads and rows with `node:sqlite` on the
// shared `/rust-data` database, same queries as `stacks/original/ext/p1-04.ts`.
// Methods are added as the ports need them (Q.D5 broadcasts, Q.P5 carts,
// Q.P5-fixA appointments and channel posts).

import { DatabaseSync } from "node:sqlite"
import type {
  AppointmentInput,
  ChannelPostInput,
  GoogleCalendarConnectionInput,
  P104FixtureExt,
} from "../../../src/fixtures/ext/p1-04.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
// A place id of its own (the other ext files use 4-11, 13, 15).
const PLACE_ID = 12n

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

const ms = (value: Date | null) => (value ? value.getTime() : null)

class SqliteP104Ext implements P104FixtureExt {
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

  /** The plain `Oauth2AuthValue` the original writes (the Rust server reads it as is). */
  async createGoogleCalendarConnection(input: GoogleCalendarConnectionInput): Promise<{ integrationId: string }> {
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
    this.transaction((db) => {
      db.prepare(
        `INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (?, ?, 'googleCalendar')`,
      ).run(integrationId, BigInt(input.workspaceId))
      db.prepare(
        `INSERT INTO "IntegrationGoogleCalendar" ("id", "workspaceId", "integrationId", "auth", "providerCalendarId", "email")
         VALUES (?, ?, ?, ?, ?, ?)`,
      ).run(
        createId(),
        BigInt(input.workspaceId),
        integrationId,
        JSON.stringify(auth),
        input.providerCalendarId,
        input.email,
      )
    })
    return { integrationId: String(integrationId) }
  }

  async createInbox(input: {
    workspaceId: string
    channel: string
    name: string
    sourceId: string
  }): Promise<{ inboxId: string }> {
    const inboxId = createId()
    this.open()
      .prepare(`INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (?, ?, ?, ?, ?)`)
      .run(inboxId, input.name, input.channel, input.sourceId, BigInt(input.workspaceId))
    return { inboxId: String(inboxId) }
  }

  async createZaloIntegration(input: {
    workspaceId: string
    name: string
    oaId: string
  }): Promise<{ integrationId: string; inboxId: string }> {
    const inboxId = createId()
    const integrationId = createId()
    this.transaction((db) => {
      db.prepare(
        `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId") VALUES (?, ?, 'zalo', ?, ?)`,
      ).run(inboxId, input.name, input.oaId, BigInt(input.workspaceId))
      db.prepare(
        `INSERT INTO "IntegrationZalo" ("id", "auth", "oaId", "name", "workspaceId", "inboxId") VALUES (?, ?, ?, ?, ?, ?)`,
      ).run(
        integrationId,
        JSON.stringify({ authType: "secretText", secretText: "parity-zalo-token" }),
        input.oaId,
        input.name,
        BigInt(input.workspaceId),
        inboxId,
      )
    })
    return { integrationId: String(integrationId), inboxId: String(inboxId) }
  }

  async createChannelPost(input: ChannelPostInput): Promise<{ channelPostId: string }> {
    const channelPostId = createId()
    this.open()
      .prepare(
        `INSERT INTO "ChannelPost"
           ("id", "workspaceId", "inboxId", "channel", "integrationId", "sourceAccountId",
            "externalPostId", "caption", "thumbnail", "permalink", "publishedAt", "metadataFetchedAt")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        channelPostId,
        BigInt(input.workspaceId),
        BigInt(input.inboxId),
        input.channel,
        BigInt(input.integrationId),
        input.sourceAccountId,
        input.externalPostId,
        input.caption,
        input.thumbnail,
        input.permalink,
        ms(input.publishedAt),
        Date.now(),
      )
    return { channelPostId: String(channelPostId) }
  }

  async createAppointment(input: AppointmentInput): Promise<{ appointmentId: string }> {
    const appointmentId = createId()
    this.open()
      .prepare(
        `INSERT INTO "Appointment"
           ("id", "workspaceId", "calendarId", "contactId", "startAt", "endAt",
            "inviteeTimezone", "status", "locationType", "cancelledAt")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        appointmentId,
        BigInt(input.workspaceId),
        BigInt(input.calendarId),
        BigInt(input.contactId),
        input.startAt.getTime(),
        input.endAt.getTime(),
        input.inviteeTimezone,
        input.status,
        input.locationType,
        input.status === "cancelled" ? Date.now() : null,
      )
    return { appointmentId: String(appointmentId) }
  }

  async createCart(input: {
    workspaceId: string
    contactId: string
    items: { productId: string; variantId: string | null; quantity: number }[]
  }): Promise<{ cartId: string }> {
    const cartId = createId()
    this.open()
      .prepare(`INSERT INTO "Cart" ("id", "workspaceId", "contactId", "items") VALUES (?, ?, ?, ?)`)
      .run(cartId, BigInt(input.workspaceId), BigInt(input.contactId), JSON.stringify(input.items))
    return { cartId: cartId.toString() }
  }

  async conversationIdForContact(input: { workspaceId: string; contactId: string }): Promise<string> {
    const row = this.open()
      .prepare(
        `SELECT CAST("id" AS TEXT) AS "id" FROM "Conversation"
         WHERE "workspaceId" = ? AND "contactId" = ? AND "sourceId" IS NULL
         ORDER BY "id" LIMIT 1`,
      )
      .get(BigInt(input.workspaceId), BigInt(input.contactId)) as { id: string } | undefined
    if (!row) {
      throw new Error(`conversationIdForContact: no conversation for contact ${input.contactId}`)
    }
    return row.id
  }

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const create = (): P104FixtureExt =>
  new SqliteP104Ext(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
