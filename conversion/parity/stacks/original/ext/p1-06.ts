// Postgres implementation of the P1.6 fixture extension (original stack).
// Writes the same logical rows the app writes; see the comment per method.
//
// Row sources (baseline 40e733cee):
// - Inbox: inbox create (packages/database/src/schema/inbox.ts); no
//   integration row, so no worker job can reach a third party.
// - Contact + ContactInbox + Conversation: createContactWithInbox
//   (packages/business/src/contact/create-with-inbox.ts) and the inbound
//   message path, which also fills the ContactInbox activity columns
//   (lastIncomingMessageAt, language, ...). The ContactInbox insert trigger
//   keeps InboxContactStat (packages/database/drizzle/20260624082115_*).
// - ContactToTag: tagService.attachToContact (packages/business/src/tag/service.ts).
// - ContactCustomField: contactCustomFieldService.setValues (value stored in
//   its canonical form, packages/business/src/contact-custom-field/normalize.ts).
// - ContactReferralEvent: the ad referral recorder
//   (packages/database/src/schema/contact-referral-event.ts), type "ad".
// - ContactOnSequence + SequenceDispatch: the sequence scheduler
//   (packages/database/src/schema/sequence-dispatch.ts), a completed send.
// - Workspace.targetCountry: workspace settings update.

import { randomBytes } from "node:crypto"
import pg from "pg"
import type {
  P106AdReferralInput,
  P106ContactInput,
  P106CreatedContact,
  P106Fixtures,
  P106InboxInput,
  P106SequenceDispatchInput,
} from "../../../src/fixtures/ext/p1-06.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
/** App processes use place ids 1-3; fixtures 15 (core), 14, 12, 11; P1.6 uses 10. */
const PLACE_ID = 10n

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

class PostgresP106Fixtures implements P106Fixtures {
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

  async createInbox(input: P106InboxInput): Promise<{ inboxId: string }> {
    const inboxId = createId()
    await this.pool.query(
      `INSERT INTO "Inbox" ("id", "name", "channel", "sourceId", "workspaceId", "status")
       VALUES ($1, $2, $3, $4, $5, 'connected')`,
      [inboxId, input.name, input.channel, input.sourceId ?? uniqueText("src"), input.workspaceId],
    )
    return { inboxId }
  }

  async createContact(input: P106ContactInput): Promise<P106CreatedContact> {
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
            "gender", "country", "locale", "timezone", "avatar", "blockedAt", "broadcastSubscribedAt",
            "subscribedAt", "emailVerified", "emailOptIn")
         VALUES ($1, $2, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)`,
        [
          contactId,
          createdAt,
          input.workspaceId,
          input.firstName ?? null,
          input.lastName ?? null,
          input.email ?? null,
          input.phoneNumber ?? null,
          input.gender ?? null,
          input.country ?? null,
          input.locale ?? null,
          input.timezone ?? null,
          input.avatar ?? null,
          input.blockedAt ?? null,
          input.broadcastSubscribedAt ?? null,
          input.subscribedAt ?? null,
          input.emailVerified ?? false,
          input.emailOptIn ?? true,
        ],
      )
      for (const [index, entry] of inboxes.entries()) {
        await client.query(
          `INSERT INTO "ContactInbox"
             ("id", "createdAt", "updatedAt", "originalContactId", "contactId", "inboxId", "channel", "source",
              "sourceId", "language", "contactLastReadAt", "lastIncomingMessageAt", "lastOutboundMessageAt",
              "consecutiveFailedReply", "lastUserInput")
           VALUES ($1, $2, $2, $3, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)`,
          [
            contactInboxIds[index],
            createdAt,
            contactId,
            entry.inboxId,
            entry.channel,
            entry.source ?? "direct",
            sourceIds[index],
            entry.language ?? null,
            entry.contactLastReadAt ?? null,
            entry.lastIncomingMessageAt ?? null,
            entry.lastOutboundMessageAt ?? null,
            entry.consecutiveFailedReply ?? 0,
            entry.lastUserInput ?? null,
          ],
        )
      }
      if (conversationId) {
        const conversation = input.conversation ?? {}
        await client.query(
          `INSERT INTO "Conversation"
             ("id", "createdAt", "updatedAt", "workspaceId", "contactId", "archivedAt", "followed", "botEnabled",
              "lastActivityAt", "agentLastReadAt", "assignedUserId")
           VALUES ($1, $2, $2, $3, $4, $5, $6, $7, $8, $9, $10)`,
          [
            conversationId,
            createdAt,
            input.workspaceId,
            contactId,
            conversation.archivedAt ?? null,
            conversation.followed ?? false,
            conversation.botEnabled ?? true,
            conversation.lastActivityAt ?? null,
            conversation.agentLastReadAt ?? null,
            conversation.assignedUserId ?? null,
          ],
        )
      }
    })
    return { contactId, contactInboxIds, sourceIds, conversationId }
  }

  async linkTag(input: { contactId: string; tagId: string }): Promise<void> {
    await this.pool.query(`INSERT INTO "ContactToTag" ("contactId", "tagId") VALUES ($1, $2)`, [
      input.contactId,
      input.tagId,
    ])
  }

  async setCustomFieldValue(input: { contactId: string; customFieldId: string; value: string }): Promise<void> {
    await this.pool.query(
      `INSERT INTO "ContactCustomField" ("id", "contactId", "customFieldId", "value")
       VALUES ($1, $2, $3, $4)
       ON CONFLICT ("contactId", "customFieldId") DO UPDATE SET "value" = EXCLUDED."value", "updatedAt" = now()`,
      [createId(), input.contactId, input.customFieldId, input.value],
    )
  }

  async createAdReferral(input: P106AdReferralInput): Promise<{ id: string }> {
    const id = createId()
    await this.pool.query(
      `INSERT INTO "ContactReferralEvent"
         ("id", "workspaceId", "contactId", "contactInboxId", "channel", "type", "provider", "occurredAt", "adId",
          "campaignId", "adGroupId", "source", "sourceUrl", "metadata", "dedupKey")
       VALUES ($1, $2, $3, $4, $5, 'ad', $6, $7, $8, $9, $10, $11, $12, $13, $14)`,
      [
        id,
        input.workspaceId,
        input.contactId,
        input.contactInboxId,
        input.channel,
        input.provider,
        input.occurredAt,
        input.adId,
        input.campaignId ?? null,
        input.adGroupId ?? null,
        input.source ?? null,
        input.sourceUrl ?? null,
        input.title === undefined ? null : JSON.stringify({ adTitle: input.title }),
        `ad:${input.adId ?? "none"}:${id}`,
      ],
    )
    return { id }
  }

  async insertSequenceDispatch(input: P106SequenceDispatchInput): Promise<void> {
    await this.transaction(async (client) => {
      const enrollmentId = createId()
      await client.query(
        `INSERT INTO "ContactOnSequence"
           ("id", "workspaceId", "sequenceId", "contactId", "status", "completedAt", "currentStep")
         VALUES ($1, $2, $3, $4, 'completed', now(), 1)`,
        [enrollmentId, input.workspaceId, input.sequenceId, input.contactId],
      )
      const id = createId()
      await client.query(
        `INSERT INTO "SequenceDispatch"
           ("id", "runAtMs", "status", "idempotencyKey", "completedAt", "deliveredAt", "seenAt", "clickedAt",
            "failedAt", "workspaceId", "sequenceId", "contactId", "contactInboxId", "stepId", "enrollmentId")
         VALUES ($1, $2, 'completed', $3, now(), $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)`,
        [
          id,
          String(Date.now()),
          `parity:${id}`,
          input.deliveredAt ?? null,
          input.seenAt ?? null,
          input.clickedAt ?? null,
          input.failedAt ?? null,
          input.workspaceId,
          input.sequenceId,
          input.contactId,
          input.contactInboxId,
          input.stepId,
          enrollmentId,
        ],
      )
    })
  }

  async setWorkspaceTargetCountry(input: { workspaceId: string; targetCountry: string | null }): Promise<void> {
    await this.pool.query(`UPDATE "Workspace" SET "targetCountry" = $2 WHERE "id" = $1`, [
      input.workspaceId,
      input.targetCountry,
    ])
  }

  async contactIdsByEmail(input: { workspaceId: string; emails: string[] }): Promise<Record<string, string>> {
    const result = await this.pool.query<{ id: string; email: string }>(
      `SELECT "id"::text AS "id", "email" FROM "Contact" WHERE "workspaceId" = $1 AND "email" = ANY($2::text[])`,
      [input.workspaceId, input.emails],
    )
    return Object.fromEntries(result.rows.map((row) => [row.email, row.id]))
  }

  async close(): Promise<void> {
    await this.pool.end()
  }
}

export const create = (): P106Fixtures =>
  new PostgresP106Fixtures(process.env.PARITY_PG_URL ?? "postgresql://chatbotx:secretkey@postgres:5432/chatbotx")
