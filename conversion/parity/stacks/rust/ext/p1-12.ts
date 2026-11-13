// Rust-stack implementation of the P1.12–P1.14 fixture extension
// (`src/fixtures/ext/p1-12.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same columns as `stacks/original/ext/p1-12.ts`.
// Methods are added as the P3.6 domain ports need them; the others throw.

import { DatabaseSync } from "node:sqlite"
import type {
  P112Fixtures,
  P112MetaCatalogConnectionInput,
  P112PollVoteInput,
} from "../../../src/fixtures/ext/p1-12.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
// Same place id as the original p1-12 extension (DECISIONS 2026-11-12).
const PLACE_ID = 4n

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
  throw new Error(`p1-12 fixture ${name} is not ported to the Rust stack yet`)
}

class SqliteP112Fixtures implements P112Fixtures {
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

  createWhatsappChannel = notPorted("createWhatsappChannel")
  createMiniAppSubmission = notPorted("createMiniAppSubmission")
  createMinigamePlayer = notPorted("createMinigamePlayer")
  createMinigamePlay = notPorted("createMinigamePlay")
  createQuestionnaireSubmission = notPorted("createQuestionnaireSubmission")

  async createPollVote(input: P112PollVoteInput): Promise<{ voteId: string }> {
    const voteId = createId()
    const now = Date.now()
    this.open()
      .prepare(
        `INSERT INTO "PollVote" ("id", "createdAt", "updatedAt", "workspaceId", "pollId", "candidateId", "contactId", "contactInboxId")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        voteId,
        now,
        now,
        BigInt(input.workspaceId),
        BigInt(input.pollId),
        BigInt(input.candidateId),
        BigInt(input.contactId),
        input.contactInboxId == null ? null : BigInt(input.contactInboxId),
      )
    return { voteId: voteId.toString() }
  }

  // `encryptedAuth` holds the plain `{accessToken, version}` JSON: the Rust
  // port reads only the catalog binding so far (products.listSendable). The
  // Meta catalog slice must store it the way its Rust decrypt expects.
  async createMetaCatalogConnection(
    input: P112MetaCatalogConnectionInput,
  ): Promise<{ connectionId: string; integrationId: string }> {
    const integrationId = createId()
    const connectionId = createId()
    const db = this.open()
    db.exec("BEGIN IMMEDIATE")
    try {
      db.prepare(`INSERT INTO "Integration" ("id", "workspaceId", "integrationType") VALUES (?, ?, 'metaCatalog')`).run(
        integrationId,
        BigInt(input.workspaceId),
      )
      db.prepare(
        `INSERT INTO "IntegrationMetaCatalog"
           ("id", "workspaceId", "integrationId", "catalogId", "catalogName", "businessId", "encryptedAuth",
            "authMode", "tokenExpiresAt", "status")
         VALUES (?, ?, ?, ?, ?, ?, ?, 'oauth', NULL, 'active')`,
      ).run(
        connectionId,
        BigInt(input.workspaceId),
        integrationId,
        input.catalogId ?? null,
        input.catalogName ?? null,
        input.businessId ?? null,
        JSON.stringify({ accessToken: input.accessToken, version: "v23.0" }),
      )
      db.exec("COMMIT")
    } catch (error) {
      db.exec("ROLLBACK")
      throw error
    }
    return { connectionId: connectionId.toString(), integrationId: integrationId.toString() }
  }

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const create = (): P112Fixtures =>
  new SqliteP112Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
