// contacts.listNotes / createNote / updateNote / deleteNote and
// contacts.listSequences / subscribeSequences / unsubscribeSequences /
// setSequences.

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { expectNormalized, Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  contactPath,
  createSequence,
  expectError,
  expectInvalid,
  idOf,
  newTenant,
  seedContact,
  sortBy,
} from "./helpers.ts"

type Note = { id: string; contactId: string; text: string; createdById: string | null; createdAt: string; updatedAt: string }

let tenant: Tenant
let api: Api

beforeAll(async () => {
  ;({ tenant, api } = await newTenant("p106-notes"))
})

const newContact = async () => (await seedContact(tenant.workspaceId, { firstName: "Noted" })).contactId

describe("notes", () => {
  const notesPath = (contactId: string, suffix = "") => contactPath(idOf(contactId), `/notes${suffix}`)
  const list = async (contactId: string) => {
    const response = await api.get<{ data: Note[] }>(notesPath(contactId))
    expect(response.status, response.text).toBe(200)
    return response.json.data
  }

  it("create (200, trimmed), list newest first, update, delete", async () => {
    const n = new Normalizer()
    const contactId = await newContact()
    expect(await list(contactId)).toEqual([])
    const first = await api.post<Note>(notesPath(contactId), { text: "  first note  " })
    expect(first.status, first.text).toBe(200)
    expect(first.json).toMatchObject({ contactId, text: "first note", createdById: null })
    expectNormalized(n, first.json).toMatchSnapshot()
    const second = await api.post<Note>(notesPath(contactId), { text: "second note" })
    expect((await list(contactId)).map((note) => note.text)).toEqual(["second note", "first note"])

    const updated = await api.put<Note>(notesPath(contactId, `/${first.json.id}`), { text: "first, edited" })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toMatchObject({ id: first.json.id, contactId, text: "first, edited", createdAt: first.json.createdAt })
    expect((await list(contactId)).map((note) => note.text)).toEqual(["second note", "first, edited"])

    const removed = await api.delete(notesPath(contactId, `/${second.json.id}`))
    expect(removed.status, removed.text).toBe(204)
    expect((await list(contactId)).map((note) => note.id)).toEqual([first.json.id])
    expectError(await api.delete(notesPath(contactId, `/${second.json.id}`)), 404, "notFound", {
      message: "Contact note not found",
    })
  })

  it("a note of another contact is not reachable through this contact", async () => {
    const owner = await newContact()
    const stranger = await newContact()
    const note = await api.post<Note>(notesPath(owner), { text: "private" })
    expectError(await api.put(notesPath(stranger, `/${note.json.id}`), { text: "x" }), 404, "notFound", {
      message: "Contact note not found",
    })
    expectError(await api.delete(notesPath(stranger, `/${note.json.id}`)), 404, "notFound")
    expect((await list(owner)).map((row) => row.text)).toEqual(["private"])
  })

  it("validation and unknown contact / note", async () => {
    const contactId = await newContact()
    expectInvalid(await api.post(notesPath(contactId), { text: "   " }), "text")
    expectInvalid(await api.post(notesPath(contactId), {}), "text")
    expectInvalid(await api.post(notesPath(contactId), { text: "x".repeat(1001) }), "text")
    expect((await api.post(notesPath(contactId), { text: "x".repeat(1000) })).status).toBe(200)
    expectInvalid(await api.put(notesPath(contactId, "/abc"), { text: "x" }), "noteId")
    expectError(await api.put(notesPath(contactId, "/1"), { text: "x" }), 404, "notFound", { message: "Contact note not found" })
    expectError(await api.get(contactPath("id:1", "/notes")), 404, "notFound")
    expectError(await api.post(contactPath("id:1", "/notes"), { text: "x" }), 404, "notFound")
  })
})

describe("sequences", () => {
  const seqPath = (contactId: string) => contactPath(idOf(contactId), "/sequences")
  const list = async (contactId: string) => {
    const response = await api.get<{ data: Array<{ sequenceId: string; sequenceName: string }> }>(seqPath(contactId))
    expect(response.status, response.text).toBe(200)
    return sortBy(response.json.data, (row) => row.sequenceName)
  }
  let alpha: string
  let beta: string
  let gamma: string
  beforeAll(async () => {
    alpha = await createSequence(api, "alpha")
    beta = await createSequence(api, "beta")
    gamma = await createSequence(api, "gamma")
  })

  it("subscribe (idempotent), list, unsubscribe", async () => {
    const contactId = await newContact()
    expect(await list(contactId)).toEqual([])
    expect((await api.post(seqPath(contactId), { sequenceIds: [alpha, beta] })).status).toBe(204)
    expect((await api.post(seqPath(contactId), { sequenceIds: [alpha] })).status).toBe(204)
    expect(await list(contactId)).toEqual([
      { sequenceId: alpha, sequenceName: "alpha" },
      { sequenceId: beta, sequenceName: "beta" },
    ])
    expect((await api.delete(seqPath(contactId), { json: { sequenceIds: [alpha] } })).status).toBe(204)
    expect(await list(contactId)).toEqual([{ sequenceId: beta, sequenceName: "beta" }])
  })

  it("subscribe refuses a sequence of another workspace or an unknown id (404), all-or-nothing", async () => {
    const contactId = await newContact()
    const other = await newTenant("p106-seq-other")
    const foreign = await createSequence(other.api, "foreign")
    expectError(await api.post(seqPath(contactId), { sequenceIds: [alpha, foreign] }), 404, "notFound", {
      message: "Sequence not found",
    })
    expectError(await api.post(seqPath(contactId), { sequenceIds: ["1"] }), 404, "notFound")
    expect(await list(contactId)).toEqual([])
  })

  it("unsubscribe of an unknown or not-subscribed sequence is a 204 no-op", async () => {
    const contactId = await newContact()
    expect((await api.post(seqPath(contactId), { sequenceIds: [beta] })).status).toBe(204)
    expect((await api.delete(seqPath(contactId), { json: { sequenceIds: ["1", gamma] } })).status).toBe(204)
    expect(await list(contactId)).toEqual([{ sequenceId: beta, sequenceName: "beta" }])
  })

  it("setSequences replaces the set; [] clears; an unknown id to add is 404", async () => {
    const contactId = await newContact()
    expect((await api.put(seqPath(contactId), { sequenceIds: [alpha, beta] })).status).toBe(204)
    expect((await list(contactId)).map((row) => row.sequenceName)).toEqual(["alpha", "beta"])
    expect((await api.put(seqPath(contactId), { sequenceIds: [beta, gamma] })).status).toBe(204)
    expect((await list(contactId)).map((row) => row.sequenceName)).toEqual(["beta", "gamma"])
    expectError(await api.put(seqPath(contactId), { sequenceIds: [beta, "1"] }), 404, "notFound")
    expect((await list(contactId)).map((row) => row.sequenceName)).toEqual(["beta", "gamma"])
    expect((await api.put(seqPath(contactId), { sequenceIds: [] })).status).toBe(204)
    expect(await list(contactId)).toEqual([])
  })

  it("validation and unknown contact", async () => {
    const contactId = await newContact()
    expectInvalid(await api.post(seqPath(contactId), { sequenceIds: [] }), "sequenceIds")
    expectInvalid(await api.post(seqPath(contactId), { sequenceIds: ["abc"] }), "sequenceIds.0")
    expectInvalid(await api.delete(seqPath(contactId), { json: { sequenceIds: [] } }), "sequenceIds")
    expectInvalid(await api.put(seqPath(contactId), {}), "sequenceIds")
    expectInvalid(
      await api.put(seqPath(contactId), { sequenceIds: Array.from({ length: 101 }, (_, i) => String(i + 1)) }),
      "sequenceIds",
    )
    for (const method of ["GET", "POST", "PUT", "DELETE"] as const) {
      const response = await api.http.request(method, `/api${contactPath("id:1", "/sequences")}`, {
        bearer: tenant.token,
        noCookies: true,
        ...(method === "GET" ? {} : { json: { sequenceIds: [alpha] } }),
      })
      expectError(response, 404, "notFound")
    }
  })
})
