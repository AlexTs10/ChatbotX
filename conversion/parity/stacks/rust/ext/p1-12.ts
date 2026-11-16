// Rust-stack implementation of the P1.12–P1.14 fixture extension
// (`src/fixtures/ext/p1-12.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same columns as `stacks/original/ext/p1-12.ts`.
// Methods are added as the P3.6 domain ports need them; the others throw.
// Times are stored as unix ms (the Rust schema), JSON as text.

import { DatabaseSync } from "node:sqlite"
import type {
  P112Fixtures,
  P112MetaCatalogConnectionInput,
  P112MiniAppSubmissionInput,
  P112MinigamePlayerInput,
  P112MinigamePlayInput,
  P112PollVoteInput,
  P112QuestionnaireSubmissionInput,
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

  async createMiniAppSubmission(
    input: P112MiniAppSubmissionInput,
  ): Promise<{ submissionId: string; createdAt: string }> {
    const submissionId = createId()
    const createdAt = input.createdAt ?? new Date().toISOString()
    const ms = Date.parse(createdAt)
    this.transaction((db) => {
      db.prepare(
        `INSERT INTO "MiniAppSubmission"
           ("id", "createdAt", "updatedAt", "workspaceId", "miniAppId", "contactId", "source", "answers")
         VALUES (?, ?, ?, ?, ?, ?, 'web', ?)`,
      ).run(
        submissionId,
        ms,
        ms,
        BigInt(input.workspaceId),
        BigInt(input.miniAppId),
        input.contactId == null ? null : BigInt(input.contactId),
        JSON.stringify(input.answers),
      )
      db.prepare(`UPDATE "MiniApp" SET "submissionsCount" = "submissionsCount" + 1 WHERE "id" = ?`).run(
        BigInt(input.miniAppId),
      )
    })
    return { submissionId: submissionId.toString(), createdAt }
  }

  async createMinigamePlayer(input: P112MinigamePlayerInput): Promise<{ playerId: string }> {
    const playerId = createId()
    const now = Date.now()
    const openedAt = input.openedAt === undefined ? now : Date.parse(input.openedAt)
    const lastPlayedAt = input.lastPlayedAt === undefined ? now : Date.parse(input.lastPlayedAt)
    this.open()
      .prepare(
        `INSERT INTO "MinigameContact"
           ("id", "createdAt", "updatedAt", "minigameId", "contactId", "contactInboxId", "openedAt", "played",
            "remaining", "sharesCount")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        playerId,
        openedAt,
        lastPlayedAt,
        BigInt(input.minigameId),
        BigInt(input.contactId),
        input.contactInboxId == null ? null : BigInt(input.contactInboxId),
        openedAt,
        input.played,
        input.remaining,
        input.sharesCount ?? 0,
      )
    return { playerId: playerId.toString() }
  }

  async createMinigamePlay(input: P112MinigamePlayInput): Promise<{ playId: string; createdAt: string }> {
    const playId = createId()
    const createdAt = input.createdAt ?? new Date().toISOString()
    const ms = Date.parse(createdAt)
    this.open()
      .prepare(
        `INSERT INTO "MinigamePlay"
           ("id", "createdAt", "updatedAt", "minigameId", "contactId", "isWinning", "prizeId", "prizeName")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        playId,
        ms,
        ms,
        BigInt(input.minigameId),
        BigInt(input.contactId),
        input.isWinning ? 1 : 0,
        input.prizeId ?? null,
        input.prizeName ?? null,
      )
    return { playId: playId.toString(), createdAt }
  }

  async createQuestionnaireSubmission(input: P112QuestionnaireSubmissionInput): Promise<{ submissionId: string }> {
    const submissionId = createId()
    const createdAt = input.createdAt ?? new Date().toISOString()
    const completedAt =
      input.completedAt !== undefined ? input.completedAt : input.status === "completed" ? createdAt : null
    const base = Date.parse(createdAt)
    this.transaction((db) => {
      db.prepare(
        `INSERT INTO "QuestionnaireSubmission"
           ("id", "createdAt", "updatedAt", "workspaceId", "questionnaireId", "contactId", "conversationId", "status",
            "totalPoints", "startedAt", "completedAt")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      ).run(
        submissionId,
        base,
        base,
        BigInt(input.workspaceId),
        BigInt(input.questionnaireId),
        BigInt(input.contactId),
        input.conversationId == null ? null : BigInt(input.conversationId),
        input.status,
        input.totalPoints ?? null,
        base,
        completedAt == null ? null : Date.parse(completedAt),
      )
      const answer = db.prepare(
        `INSERT INTO "QuestionnaireAnswer"
           ("id", "createdAt", "updatedAt", "submissionId", "questionId", "questionIdSnapshot", "questionTitleSnapshot",
            "questionTypeSnapshot", "labelSnapshot", "value", "pointsEarned", "answeredAt")
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      for (const [index, item] of (input.answers ?? []).entries()) {
        const answeredAt = base + index + 1
        answer.run(
          createId(),
          answeredAt,
          answeredAt,
          submissionId,
          BigInt(item.questionId),
          item.questionId,
          item.questionTitle,
          item.questionType,
          item.label,
          JSON.stringify(item.value),
          item.pointsEarned ?? null,
          answeredAt,
        )
      }
    })
    return { submissionId: submissionId.toString() }
  }

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
