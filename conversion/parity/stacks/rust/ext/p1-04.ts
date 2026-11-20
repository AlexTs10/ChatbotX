// Rust-stack implementation of the P1.4 fixture extension
// (`src/fixtures/ext/p1-04.ts`): reads and rows with `node:sqlite` on the
// shared `/rust-data` database, same queries as `stacks/original/ext/p1-04.ts`.
// Methods are added as the ports need them (Q.D5 broadcasts, Q.P5 carts);
// the others throw.

import { DatabaseSync } from "node:sqlite"
import type { P104FixtureExt } from "../../../src/fixtures/ext/p1-04.ts"

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

const notPorted = (name: string) => (): never => {
  throw new Error(`p1-04 fixture ${name} is not ported to the Rust stack yet`)
}

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

  createGoogleCalendarConnection = notPorted("createGoogleCalendarConnection")
  createInbox = notPorted("createInbox")
  createZaloIntegration = notPorted("createZaloIntegration")
  createChannelPost = notPorted("createChannelPost")
  createAppointment = notPorted("createAppointment")

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
