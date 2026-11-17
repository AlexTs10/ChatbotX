// Rust-stack implementation of the P1.8 fixture extension
// (`src/fixtures/ext/p1-08.ts`): rows written with `node:sqlite` into the
// shared `/rust-data` database, same columns as `stacks/original/ext/p1-08.ts`.
// Methods are added as the Q.D5 email-topic ports need them; the others throw.

import { DatabaseSync } from "node:sqlite"
import type { FolderType, P108Fixtures } from "../../../src/fixtures/ext/p1-08.ts"

const SNOWFLAKE_EPOCH_MS = Date.parse("2004-02-01T00:00:00.000Z")
// A place id of its own (p1-03..p1-12 use 4-9), so fixture ids never collide.
const PLACE_ID = 10n

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
  throw new Error(`p1-08 fixture ${name} is not ported to the Rust stack yet`)
}

class SqliteP108Fixtures {
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

  async createFolder(input: { workspaceId: string; name: string; folderType: FolderType }): Promise<{ folderId: string }> {
    const folderId = createId()
    this.open()
      .prepare(
        `INSERT INTO "Folder" ("id", "name", "folderType", "parentId", "workspaceId", "paths")
         VALUES (?, ?, ?, NULL, ?, '[]')`,
      )
      .run(folderId, input.name, input.folderType, BigInt(input.workspaceId))
    return { folderId: folderId.toString() }
  }

  async close(): Promise<void> {
    this.db?.close()
    this.db = undefined
  }
}

// Every other method throws until a port needs it (`then` stays undefined:
// the loader awaits this object, which must not look like a promise).
export const create = (): P108Fixtures =>
  new Proxy(new SqliteP108Fixtures(process.env.PARITY_SQLITE_PATH ?? "/rust-data/chatbotx.db"), {
    get: (target, key, receiver) =>
      key in target || typeof key !== "string" || key === "then"
        ? Reflect.get(target, key, receiver)
        : notPorted(key),
  }) as unknown as P108Fixtures
