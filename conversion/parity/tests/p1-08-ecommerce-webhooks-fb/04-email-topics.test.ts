// emailTopics.* — topics that group email broadcast unsubscribe preferences.
// Unique name per workspace (400 nameTaken), optional folder of type
// `emailTopic` (fixture row: the public folders API only serves tag and
// custom field folders), newest-first list with name/folder filters.

import { describe, expect, it } from "vitest"
import { type Api, expectError, expectInvalid, ext, keysOf, qs, sortQuery, tenantApi, uniqueName } from "./helpers.ts"

const TOPIC_KEYS = [
  "clicksTotal",
  "createdAt",
  "deliveredsTotal",
  "folderId",
  "id",
  "name",
  "seensTotal",
  "sendsTotal",
  "updatedAt",
]

type Topic = {
  id: string
  name: string
  folderId: string | null
  sendsTotal: number
  deliveredsTotal: number
  seensTotal: number
  clicksTotal: number
  createdAt: string
  updatedAt: string
}

type TopicList = { data: Topic[]; pageCount: number }

const createTopic = async (api: Api, body: Record<string, unknown>): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/email-topics", body)
  expect(response.status, response.text).toBe(201)
  expect(keysOf(response.json)).toEqual(["id"])
  expect(response.json.id).toMatch(/^\d{15,20}$/)
  return response.json.id
}

const getTopic = async (api: Api, id: string): Promise<Topic> => {
  const response = await api.get<Topic>(`/v1/email-topics/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("emailTopics.create / get", () => {
  it("creates a topic (201 {id}) and returns it with zero counters", async () => {
    const { api } = await tenantApi("p108-et-create")
    const name = uniqueName("News")
    const id = await createTopic(api, { name: `  ${name}  ` })
    const topic = await getTopic(api, id)
    expect(keysOf(topic)).toEqual(TOPIC_KEYS)
    expect(topic).toMatchObject({
      id,
      name,
      folderId: null,
      sendsTotal: 0,
      deliveredsTotal: 0,
      seensTotal: 0,
      clicksTotal: 0,
    })
    expect(Date.parse(topic.createdAt)).not.toBeNaN()
  })

  it("places a topic in an emailTopic folder; folder 0 means root; other folders are 404", async () => {
    const { tenant, api } = await tenantApi("p108-et-folder")
    const fixtures = await ext()
    const { folderId } = await fixtures.createFolder({ workspaceId: tenant.workspaceId, name: "Topics", folderType: "emailTopic" })
    const { folderId: wrongType } = await fixtures.createFolder({ workspaceId: tenant.workspaceId, name: "FB", folderType: "fbComment" })
    const { tenant: other } = await tenantApi("p108-et-folder-other")
    const { folderId: foreign } = await fixtures.createFolder({ workspaceId: other.workspaceId, name: "Theirs", folderType: "emailTopic" })

    const inFolder = await createTopic(api, { name: uniqueName("In folder"), folderId })
    expect((await getTopic(api, inFolder)).folderId).toBe(folderId)
    const atRoot = await createTopic(api, { name: uniqueName("Root"), folderId: "0" })
    expect((await getTopic(api, atRoot)).folderId).toBeNull()
    const explicitNull = await createTopic(api, { name: uniqueName("Null"), folderId: null })
    expect((await getTopic(api, explicitNull)).folderId).toBeNull()

    for (const bad of [wrongType, foreign, "1"]) {
      expectError(await api.post("/v1/email-topics", { name: uniqueName("Bad"), folderId: bad }), 404, "notFound", {
        defined: true,
        message: "Folder not found",
      })
    }
  })

  it("rejects a taken name (400 nameTaken) and invalid input (422)", async () => {
    const { api } = await tenantApi("p108-et-create-errors")
    const { api: other } = await tenantApi("p108-et-create-errors-other")
    const name = uniqueName("Taken")
    await createTopic(api, { name })
    expectError(await api.post("/v1/email-topics", { name }), 400, "nameTaken", { defined: true, message: "Name is already taken." })
    // Names are unique per workspace only.
    await createTopic(other, { name })

    expectInvalid(await api.post("/v1/email-topics", {}), "name")
    expectInvalid(await api.post("/v1/email-topics", { name: "   " }), "name")
    expectInvalid(await api.post("/v1/email-topics", { name: "x".repeat(256) }), "name")
    expectInvalid(await api.post("/v1/email-topics", { name: "ok", folderId: "abc" }), "folderId")
  })

  it("answers 404 for an unknown or foreign topic and 422 for a non-numeric id", async () => {
    const { api } = await tenantApi("p108-et-get")
    const { api: other } = await tenantApi("p108-et-get-other")
    const foreign = await createTopic(other, { name: uniqueName("Foreign") })
    expectError(await api.get("/v1/email-topics/1"), 404, "notFound", { defined: true, message: "Email topic not found" })
    expectError(await api.get(`/v1/email-topics/${foreign}`), 404, "notFound")
    expectInvalid(await api.get("/v1/email-topics/abc"), "id")
  })
})

describe("emailTopics.list", () => {
  it("lists newest first, filters by name and folder, sorts and pages", async () => {
    const { tenant, api } = await tenantApi("p108-et-list")
    const { api: otherApi } = await tenantApi("p108-et-list-other")
    const { folderId } = await (await ext()).createFolder({
      workspaceId: tenant.workspaceId,
      name: "Topics",
      folderType: "emailTopic",
    })
    const a = await createTopic(api, { name: "alpha news" })
    const b = await createTopic(api, { name: "beta offers", folderId })
    const c = await createTopic(api, { name: "gamma ALPHA" })
    await createTopic(otherApi, { name: "alpha news" })

    const all = await api.get<TopicList>("/v1/email-topics")
    expect(all.status).toBe(200)
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((topic) => topic.id)).toEqual([c, b, a])
    expect(all.json.data[1]).toEqual(await getTopic(api, b))

    const byName = await api.get<TopicList>(`/v1/email-topics${qs({ name: "ALPHA" })}`)
    expect(byName.json.data.map((topic) => topic.id)).toEqual([c, a])

    const inFolder = await api.get<TopicList>(`/v1/email-topics${qs({ folderId })}`)
    expect(inFolder.json).toMatchObject({ data: [{ id: b, folderId }], pageCount: 1 })
    const root = await api.get<TopicList>(`/v1/email-topics${qs({ folderId: "0" })}`)
    expect(root.json.data.map((topic) => topic.id)).toEqual([c, a])

    const sorted = await api.get<TopicList>(`/v1/email-topics?${sortQuery([{ id: "name", desc: false }])}`)
    expect(sorted.status, sorted.text).toBe(200)
    expect(sorted.json.data.map((topic) => topic.name)).toEqual(["alpha news", "beta offers", "gamma ALPHA"])

    const page2 = await api.get<TopicList>(`/v1/email-topics${qs({ perPage: 2, page: 2 })}`)
    expect(page2.json).toMatchObject({ data: [{ id: a }], pageCount: 2 })

    const none = await api.get(`/v1/email-topics${qs({ name: "nothing-matches" })}`)
    expect(none.json).toEqual({ data: [], pageCount: 0 })

    expectInvalid(await api.get(`/v1/email-topics${qs({ perPage: 0 })}`), "perPage")
    expectInvalid(await api.get(`/v1/email-topics${qs({ folderId: "x" })}`), "folderId")
  })
})

describe("emailTopics.update", () => {
  it("renames a topic and returns it; keeps the folder", async () => {
    const { tenant, api } = await tenantApi("p108-et-update")
    const { folderId } = await (await ext()).createFolder({
      workspaceId: tenant.workspaceId,
      name: "Topics",
      folderType: "emailTopic",
    })
    const id = await createTopic(api, { name: uniqueName("Old"), folderId })
    const before = await getTopic(api, id)
    const name = uniqueName("New")
    const response = await api.put<Topic>(`/v1/email-topics/${id}`, { name: ` ${name} `, folderId: null })
    expect(response.status, response.text).toBe(200)
    expect(keysOf(response.json)).toEqual(TOPIC_KEYS)
    expect(response.json).toMatchObject({ id, name, folderId, createdAt: before.createdAt })
    expect(await getTopic(api, id)).toEqual(response.json)
    // Same name again is allowed (the topic itself is excluded).
    expect((await api.put(`/v1/email-topics/${id}`, { name })).status).toBe(200)
  })

  it("rejects a name another topic has (400), an unknown id (404) and invalid input (422)", async () => {
    const { api } = await tenantApi("p108-et-update-errors")
    const taken = uniqueName("Taken")
    await createTopic(api, { name: taken })
    const id = await createTopic(api, { name: uniqueName("Mine") })
    expectError(await api.put(`/v1/email-topics/${id}`, { name: taken }), 400, "nameTaken", { message: "Name is already taken." })
    expectError(await api.put("/v1/email-topics/1", { name: uniqueName("x") }), 404, "notFound", {
      defined: true,
      message: "Email topic not found",
    })
    expectInvalid(await api.put(`/v1/email-topics/${id}`, {}), "name")
    expectInvalid(await api.put(`/v1/email-topics/${id}`, { name: "" }), "name")
  })
})

describe("emailTopics.delete", () => {
  it("deletes a topic (204); 404 for an unknown, foreign or already deleted topic", async () => {
    const { api } = await tenantApi("p108-et-delete")
    const { api: other } = await tenantApi("p108-et-delete-other")
    const id = await createTopic(api, { name: uniqueName("Gone") })
    const foreign = await createTopic(other, { name: uniqueName("Theirs") })

    const deleted = await api.delete(`/v1/email-topics/${id}`)
    expect(deleted.status).toBe(204)
    expect(deleted.text).toBe("")
    expectError(await api.get(`/v1/email-topics/${id}`), 404, "notFound")
    expectError(await api.delete(`/v1/email-topics/${id}`), 404, "notFound", { defined: true, message: "Email topic not found" })
    expectError(await api.delete(`/v1/email-topics/${foreign}`), 404, "notFound")
    expect((await other.get(`/v1/email-topics/${foreign}`)).status).toBe(200)
    expectInvalid(await api.delete("/v1/email-topics/abc"), "id")
  })
})
