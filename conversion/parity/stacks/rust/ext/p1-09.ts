// Rust-stack implementation of the P1.9 fixture extension
// (`src/fixtures/ext/p1-09.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same columns as `stacks/original/ext/p1-09.ts`
// (dates as epoch ms, `paths` as a JSON array of id strings). Methods are
// added as the P3.6 ports need them; the others throw.

import { DatabaseSync } from "node:sqlite"
import type { P109Fixtures } from "../../../src/fixtures/ext/p1-09.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
const PLACE_ID = 6n

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
  throw new Error(`p1-09 fixture ${name} is not ported to the Rust stack yet`)
}

class SqliteP109Fixtures implements P109Fixtures {
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

  // folderService.create (packages/business/src/folder/service.ts), top level:
  // parentId null, paths [].
  async createFolder(input: { workspaceId: string; name: string; folderType: string }): Promise<{ folderId: string }> {
    const folderId = createId()
    const now = Date.now()
    this.open()
      .prepare(
        `INSERT INTO "Folder" ("id", "createdAt", "updatedAt", "name", "folderType", "parentId", "workspaceId", "paths")
         VALUES (?, ?, ?, ?, ?, NULL, ?, '[]')`,
      )
      .run(folderId, now, now, input.name, input.folderType, BigInt(input.workspaceId))
    return { folderId: folderId.toString() }
  }

  createInstagramChannel = notPorted("createInstagramChannel")
  createInbox = notPorted("createInbox")
  addWorkspaceMember = notPorted("addWorkspaceMember")
  createGoogleAdsConnection = notPorted("createGoogleAdsConnection")
  setGoogleAdsSettings = notPorted("setGoogleAdsSettings")
  insertGoogleAdsEvents = notPorted("insertGoogleAdsEvents")

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

export const create = (): P109Fixtures =>
  new SqliteP109Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db")
