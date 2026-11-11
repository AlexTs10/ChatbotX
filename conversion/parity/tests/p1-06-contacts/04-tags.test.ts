// contacts.listTags / addTags / removeTags / addTagsByName /
// removeTagsByName / setTags.

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { expectNormalized, Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  contactPath,
  createTag,
  expectError,
  expectInvalid,
  idOf,
  newTenant,
  seedContact,
} from "./helpers.ts"

type Tag = { id: string; name: string; folderId: string | null }

let tenant: Tenant
let api: Api

beforeAll(async () => {
  ;({ tenant, api } = await newTenant("p106-tags"))
})

const newContact = async () => (await seedContact(tenant.workspaceId, { firstName: "Tagged" })).contactId
const tagsOf = async (contactId: string): Promise<Tag[]> => {
  const response = await api.get<{ data: Tag[] }>(contactPath(idOf(contactId), "/tags"))
  expect(response.status, response.text).toBe(200)
  return response.json.data
}
const namesOf = async (contactId: string) => (await tagsOf(contactId)).map((tag) => tag.name)
const workspaceTags = async () => {
  const response = await api.get<{ data: Tag[] }>("/v1/tags?perPage=50")
  expect(response.status, response.text).toBe(200)
  return response.json.data.map((tag) => tag.name).sort()
}

describe("listTags", () => {
  it("lists the contact's tags by name, as {id, name, folderId}", async () => {
    const n = new Normalizer()
    const contactId = await newContact()
    expect(await tagsOf(contactId)).toEqual([])
    const beta = await createTag(api, "beta-list")
    const alpha = await createTag(api, "alpha-list")
    expect((await api.post(contactPath(idOf(contactId), "/tags"), { tagIds: [beta, alpha] })).status).toBe(204)
    const tags = await tagsOf(contactId)
    expect(tags).toEqual([
      { id: alpha, name: "alpha-list", folderId: null },
      { id: beta, name: "beta-list", folderId: null },
    ])
    expectNormalized(n, { data: tags }).toMatchSnapshot()
    expectError(await api.get(contactPath("id:1", "/tags")), 404, "notFound")
  })
})

describe("addTags / removeTags (by id)", () => {
  it("attaches; an existing link, an unknown id or another workspace's tag is ignored", async () => {
    const contactId = await newContact()
    const vip = await createTag(api, "vip-ids")
    const other = await newTenant("p106-tags-other")
    const foreign = await createTag(other.api, "foreign")
    const add = (tagIds: string[]) => api.post(contactPath(idOf(contactId), "/tags"), { tagIds })
    expect((await add([vip])).status).toBe(204)
    expect((await add([vip, "1", foreign])).status).toBe(204)
    expect(await namesOf(contactId)).toEqual(["vip-ids"])

    const remove = (tagIds: string[]) => api.delete(contactPath(idOf(contactId), "/tags"), { json: { tagIds } })
    // A tag the contact does not have is ignored.
    expect((await remove([foreign, "1"])).status).toBe(204)
    expect(await namesOf(contactId)).toEqual(["vip-ids"])
    expect((await remove([vip])).status).toBe(204)
    expect(await namesOf(contactId)).toEqual([])
  })

  it("validation and unknown contact", async () => {
    const contactId = await newContact()
    const path = contactPath(idOf(contactId), "/tags")
    expectInvalid(await api.post(path, { tagIds: [] }), "tagIds")
    expectInvalid(await api.post(path, {}), "tagIds")
    expectInvalid(await api.post(path, { tagIds: ["abc"] }), "tagIds.0")
    expectInvalid(await api.post(path, { tagIds: Array.from({ length: 101 }, (_, i) => String(i + 1)) }), "tagIds")
    expectInvalid(await api.delete(path, { json: { tagIds: [] } }), "tagIds")
    expectError(await api.post(contactPath("id:1", "/tags"), { tagIds: ["1"] }), 404, "notFound")
    expectError(await api.delete(contactPath("id:1", "/tags"), { json: { tagIds: ["1"] } }), 404, "notFound")
  })
})

describe("addTagsByName / removeTagsByName", () => {
  it("reuses tags by exact name and creates the others (trimmed); removal never creates", async () => {
    const contactId = await newContact()
    await createTag(api, "Existing-name")
    const add = (tags: string[]) => api.post(contactPath(idOf(contactId), "/tags/by-name"), { tags })
    expect((await add(["Existing-name", "  fresh-name  ", "fresh-name"])).status).toBe(204)
    expect(await namesOf(contactId)).toEqual(["Existing-name", "fresh-name"])
    // Names are case-sensitive: "existing-name" is another tag.
    expect((await add(["existing-name"])).status).toBe(204)
    expect(await namesOf(contactId)).toEqual(["Existing-name", "existing-name", "fresh-name"])
    expect(await workspaceTags()).toEqual(expect.arrayContaining(["Existing-name", "existing-name", "fresh-name"]))

    const remove = (tags: string[]) => api.delete(contactPath(idOf(contactId), "/tags/by-name"), { json: { tags } })
    expect((await remove(["fresh-name", "never-created"])).status).toBe(204)
    expect(await namesOf(contactId)).toEqual(["Existing-name", "existing-name"])
    expect(await workspaceTags()).not.toContain("never-created")
  })

  it("validation and unknown contact", async () => {
    const contactId = await newContact()
    const path = contactPath(idOf(contactId), "/tags/by-name")
    expectInvalid(await api.post(path, { tags: [] }), "tags")
    expectInvalid(await api.post(path, { tags: ["   "] }), "tags.0")
    expectInvalid(await api.post(path, { tags: Array.from({ length: 101 }, (_, i) => `t${i}`) }), "tags")
    expectInvalid(await api.delete(path, { json: { tags: [] } }), "tags")
    expectError(await api.post(contactPath("id:1", "/tags/by-name"), { tags: ["x"] }), 404, "notFound")
    expectError(await api.delete(contactPath("id:1", "/tags/by-name"), { json: { tags: ["x"] } }), 404, "notFound")
  })
})

describe("setTags", () => {
  it("replaces the tag set by name; [] clears it", async () => {
    const contactId = await newContact()
    const path = contactPath(idOf(contactId), "/tags")
    expect((await api.put(path, { tags: ["set-a", "set-b"] })).status).toBe(204)
    expect(await namesOf(contactId)).toEqual(["set-a", "set-b"])
    expect((await api.put(path, { tags: ["set-b", "set-c"] })).status).toBe(204)
    expect(await namesOf(contactId)).toEqual(["set-b", "set-c"])
    expect((await api.put(path, { tags: [] })).status).toBe(204)
    expect(await namesOf(contactId)).toEqual([])
    // Tags removed from the contact stay in the workspace.
    expect(await workspaceTags()).toEqual(expect.arrayContaining(["set-a", "set-b", "set-c"]))
  })

  it("validation and unknown contact", async () => {
    const contactId = await newContact()
    const path = contactPath(idOf(contactId), "/tags")
    expectInvalid(await api.put(path, {}), "tags")
    expectInvalid(await api.put(path, { tags: [""] }), "tags.0")
    expectError(await api.put(contactPath("id:1", "/tags"), { tags: [] }), 404, "notFound")
  })
})
