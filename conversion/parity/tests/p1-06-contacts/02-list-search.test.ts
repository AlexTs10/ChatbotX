// contacts.list / search / count / findByCustomField: default order, sort,
// pagination, include/withCount, keyword search (incl. non-ASCII names; the
// Rust port uses SQLite FTS5 trigram), audience keys and the count shortcut.

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  expectContactShape,
  at,
  createCustomField,
  expectError,
  expectInvalid,
  ext,
  filter,
  ids,
  type ContactRow,
  type ListBody,
  listContacts,
  newTenant,
  qs,
} from "./helpers.ts"

const ALL = filter([]) // an empty filter forces the real COUNT/list path

describe("keyword search", () => {
  let tenant: Tenant
  let api: Api
  const c: Record<string, string> = {}

  beforeAll(async () => {
    ;({ tenant, api } = await newTenant("p106-keyword"))
    const fx = await ext()
    const { inboxId } = await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "messenger", name: "M" })
    const seed = async (key: string, input: Parameters<typeof fx.createContact>[0], minute: number, sourceId?: string) => {
      const created = await fx.createContact({
        ...input,
        createdAt: at(`2025-03-01T10:${String(minute).padStart(2, "0")}:00Z`),
        inboxes: [{ inboxId, channel: "messenger", ...(sourceId ? { sourceId } : {}) }],
      })
      c[key] = created.contactId
    }
    const ws = tenant.workspaceId
    await seed("emile", { workspaceId: ws, firstName: "Émile", lastName: "Zola", email: "emile@parity.test", phoneNumber: "+33612345678" }, 1)
    await seed("zoe", { workspaceId: ws, firstName: "Zoë", lastName: "Ångström", email: "zoe@example.org", phoneNumber: "+84901234567" }, 2)
    await seed("nguyen", { workspaceId: ws, firstName: "Nguyễn", lastName: "Văn An" }, 3)
    await seed("li", { workspaceId: ws, firstName: "李", lastName: "小龍" }, 4)
    await seed("bob", { workspaceId: ws, firstName: "Bob", lastName: "O'Brien" }, 5, "PSID-777")
    await seed("pct", { workspaceId: ws, firstName: "100%", lastName: "under_score" }, 6)
    await seed("anon", { workspaceId: ws }, 7)
  })

  const search = async (keyword: string) => ids(await listContacts(api, { keyword })).sort()
  const expected = (...keys: string[]) => keys.map((key) => c[key] as string).sort()

  it.each([
    ["émile", ["emile"]],
    ["ÉMILE", ["emile"]],
    ["mile", ["emile"]],
    ["  zola  ", ["emile"]],
    ["zo", ["emile", "zoe"]],
    ["ö", ["zoe"]],
    ["ångström", ["zoe"]],
    ["nguyễn", ["nguyen"]],
    ["văn an", ["nguyen"]],
    ["小龍", ["li"]],
    ["李", ["li"]],
    ["o'brien", ["bob"]],
    ["100%", ["pct"]],
    ["_", ["pct"]],
  ])("names: %j", async (keyword, keys) => {
    expect(await search(keyword)).toEqual(expected(...keys))
  })

  it.each([
    // No accent folding: "emile" is not a substring of "Émile".
    ["emile", []],
    ["zoe", []],
    ["nguyen", []],
    // The keyword is matched per name column, never against "first last".
    ["Émile Zola", []],
    ["%", ["pct"]],
  ])("no fuzzy matching: %j", async (keyword, keys) => {
    expect(await search(keyword)).toEqual(expected(...keys))
  })

  it.each([
    ["@example.org", ["zoe"]],
    ["EMILE@PARITY", ["emile"]],
    ["@", ["emile", "zoe"]],
  ])("a keyword with @ searches the email only: %j", async (keyword, keys) => {
    expect(await search(keyword)).toEqual(expected(...keys))
  })

  it.each([
    ["901234", ["zoe"]],
    ["+84 901 234", ["zoe"]],
    ["(+33) 6-123", ["emile"]],
    // 5 digits only: not treated as a phone.
    ["(+33) 6-12", []],
    // Fewer than 6 digits in a phone-like keyword: names only.
    ["+84 9", []],
    ["777", []],
  ])("digits and phone-like keywords: %j", async (keyword, keys) => {
    expect(await search(keyword)).toEqual(expected(...keys))
  })

  it("a contact id matches exactly; a channel user id (sourceId) matches exactly", async () => {
    expect(await search(c.bob as string)).toEqual(expected("bob"))
    expect(await search("PSID-777")).toEqual(expected("bob"))
    expect(await search("psid-777")).toEqual([])
  })

  it("an empty keyword lists everything; POST /search takes the same keyword in the body", async () => {
    expect(await search("")).toEqual(Object.values(c).sort())
    const response = await api.post<ListBody>("/v1/contacts/search", { keyword: "zo" })
    expect(response.status, response.text).toBe(200)
    expect(response.json.data.map((row) => row.id).sort()).toEqual(expected("emile", "zoe"))
    expect(response.json).toMatchObject({ totalCount: 2, pageCount: 1, totalCountCapped: false })
  })

  it("count with a keyword is an exact count", async () => {
    const response = await api.get(`/v1/contacts/count${qs({ keyword: "zo" })}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ total: 2, totalCountCapped: false })
  })
})

describe("order, sort and pagination", () => {
  let tenant: Tenant
  let api: Api
  const byName: Record<string, string> = {}
  // createdAt order (oldest first): carol, alice, dave, bob, (null name), eve
  const seeds: Array<[string | null, string | null, string]> = [
    ["carol", "Young", "2025-01-01T00:00:00Z"],
    ["alice", "Xavier", "2025-01-02T00:00:00Z"],
    ["dave", "Young", "2025-01-03T00:00:00Z"],
    ["bob", "Zed", "2025-01-04T00:00:00Z"],
    [null, "Anonymous", "2025-01-05T00:00:00Z"],
    ["eve", null, "2025-01-06T00:00:00Z"],
  ]

  beforeAll(async () => {
    ;({ tenant, api } = await newTenant("p106-order"))
    const fx = await ext()
    const { inboxId } = await fx.createInbox({
      workspaceId: tenant.workspaceId,
      channel: "telegram",
      name: "T",
      sourceId: "parity-telegram-bot",
    })
    for (const [firstName, lastName, createdAt] of seeds) {
      const created = await fx.createContact({
        workspaceId: tenant.workspaceId,
        firstName,
        lastName,
        createdAt: at(createdAt),
        inboxes: [{ inboxId, channel: "telegram" }],
      })
      byName[firstName ?? "null"] = created.contactId
    }
  })

  const names = (response: { json: ListBody; status: number; text: string }) => {
    expect(response.status, response.text).toBe(200)
    return response.json.data.map((row) => row.firstName ?? "null")
  }
  const sorted = (sort: unknown) => listContacts(api, { sort: JSON.stringify(sort) })

  it("default order is createdAt desc, with counts", async () => {
    const response = await listContacts(api)
    expect(names(response)).toEqual(["eve", "null", "bob", "dave", "alice", "carol"])
    expect(response.json).toMatchObject({ totalCount: 6, pageCount: 1, totalCountCapped: false })
  })

  it("sort by one column asc / desc; NULLs sort last ascending and first descending", async () => {
    expect(names(await sorted([{ id: "firstName", desc: false }]))).toEqual(["alice", "bob", "carol", "dave", "eve", "null"])
    expect(names(await sorted([{ id: "firstName", desc: true }]))).toEqual(["null", "eve", "dave", "carol", "bob", "alice"])
    expect(names(await sorted([{ id: "createdAt", desc: false }]))).toEqual(["carol", "alice", "dave", "bob", "null", "eve"])
  })

  it("multi-column sort applies the keys in order", async () => {
    expect(
      names(
        await sorted([
          { id: "lastName", desc: true },
          { id: "firstName", desc: false },
        ]),
      ),
    ).toEqual(["eve", "bob", "carol", "dave", "alice", "null"])
  })

  it("unknown sort columns and unparsable sort values fall back to the default order", async () => {
    const fallback = ["eve", "null", "bob", "dave", "alice", "carol"]
    expect(names(await sorted([{ id: "notAColumn", desc: false }]))).toEqual(fallback)
    expect(names(await listContacts(api, { sort: "not json" }))).toEqual(fallback)
    expect(names(await listContacts(api, { sort: JSON.stringify({ id: "firstName" }) }))).toEqual(fallback)
  })

  it("page / perPage slices the ordered list; pageCount follows perPage", async () => {
    const page = async (pageNo: number, perPage: number) => listContacts(api, { page: pageNo, perPage })
    const first = await page(1, 4)
    expect(names(first)).toEqual(["eve", "null", "bob", "dave"])
    expect(first.json).toMatchObject({ totalCount: 6, pageCount: 2 })
    expect(names(await page(2, 4))).toEqual(["alice", "carol"])
    const beyond = await page(3, 4)
    expect(names(beyond)).toEqual([])
    expect(beyond.json).toMatchObject({ totalCount: 6, pageCount: 2 })
    expect(names(await page(3, 2))).toEqual(["alice", "carol"])
    expect((await page(1, 5)).json.pageCount).toBe(2)
  })

  it("pagination input validation", async () => {
    expectInvalid(await listContacts(api, { perPage: 0 }), "perPage")
    expectInvalid(await listContacts(api, { page: 0 }), "page")
    expectInvalid(await listContacts(api, { perPage: "abc" }), "perPage")
    expectInvalid(await listContacts(api, { page: 1.5 }), "page")
  })

  it("withCount=false skips the count", async () => {
    const response = await api.post<ListBody>("/v1/contacts/search", { withCount: false, perPage: 2 })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ totalCount: 0, pageCount: 0, totalCountCapped: false })
    expect(response.json.data).toHaveLength(2)
  })

  it("include narrows the embedded relations", async () => {
    const keysOf = (body: ListBody) => Object.keys(body.data[0] ?? {})
    const all = await listContacts(api, { perPage: 1 })
    expect(keysOf(all.json)).toEqual(expect.arrayContaining(["tags", "contactCustomFields", "contactInboxes", "conversation"]))
    const tagsOnly = await listContacts(api, { perPage: 1, include: ["tags"] })
    expect(tagsOnly.status, tagsOnly.text).toBe(200)
    expect(keysOf(tagsOnly.json)).toContain("tags")
    for (const key of ["contactCustomFields", "contactInboxes", "conversation"]) {
      expect(keysOf(tagsOnly.json)).not.toContain(key)
    }
    const none = await api.post<ListBody>("/v1/contacts/search", { perPage: 1, include: [] })
    expect(none.status, none.text).toBe(200)
    for (const key of ["tags", "contactCustomFields", "contactInboxes", "conversation"]) {
      expect(keysOf(none.json)).not.toContain(key)
    }
    expectInvalid(await listContacts(api, { include: ["notes"] }), "include.0")
  })

  it("the full row shape (snapshot); messenger-style avatars are media proxy URLs", async () => {
    const n = new Normalizer()
    const response = await listContacts(api, { perPage: 1 })
    expect(response.status, response.text).toBe(200)
    const row = response.json.data[0] as ContactRow & { avatar: string; contactInboxes: Array<{ sourceId: string }> }
    // A telegram contact without avatar: no proxy (only messenger / instagram hydrate).
    expect(row.avatar).toBeNull()
    expectContactShape(n, response.json, {
      checkTimestamps: false,
      replace: { [tenant.workspaceId]: "<workspace>", [row.contactInboxes[0]?.sourceId ?? "-"]: "<sourceId>" },
    }).toMatchSnapshot()
  })
})

describe("perPage is capped at 50 and defaults to 50", () => {
  it("lists at most 50 rows", async () => {
    const { tenant, api } = await newTenant("p106-cap")
    const fx = await ext()
    const { inboxId } = await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "zalo", name: "Z" })
    for (let index = 0; index < 52; index += 1) {
      await fx.createContact({ workspaceId: tenant.workspaceId, firstName: `n${index}`, inboxes: [{ inboxId, channel: "zalo" }] })
    }
    const big = await listContacts(api, { perPage: 500 })
    expect(big.json.data).toHaveLength(50)
    expect(big.json).toMatchObject({ totalCount: 52, pageCount: 2 })
    const byDefault = await listContacts(api)
    expect(byDefault.json.data).toHaveLength(50)
    expect(byDefault.json).toMatchObject({ totalCount: 52, pageCount: 2 })
    const pageTwo = await listContacts(api, { page: 2 })
    expect(pageTwo.json.data).toHaveLength(2)
  })
})

describe("audience keys: channels / inboxIds", () => {
  it("restrict to contacts with an inbox on the channels / inboxes", async () => {
    const { tenant, api } = await newTenant("p106-audience")
    const fx = await ext()
    const ws = tenant.workspaceId
    const messenger = (await fx.createInbox({ workspaceId: ws, channel: "messenger", name: "M" })).inboxId
    const telegram = (await fx.createInbox({ workspaceId: ws, channel: "telegram", name: "T" })).inboxId
    const m = (await fx.createContact({ workspaceId: ws, firstName: "m", inboxes: [{ inboxId: messenger, channel: "messenger" }] })).contactId
    const t = (await fx.createContact({ workspaceId: ws, firstName: "t", inboxes: [{ inboxId: telegram, channel: "telegram" }] })).contactId
    const both = (
      await fx.createContact({
        workspaceId: ws,
        firstName: "both",
        inboxes: [
          { inboxId: messenger, channel: "messenger" },
          { inboxId: telegram, channel: "telegram" },
        ],
      })
    ).contactId
    const none = (await fx.createContact({ workspaceId: ws, firstName: "none" })).contactId
    const none2 = (await fx.createContact({ workspaceId: ws, firstName: "none2", conversation: null })).contactId

    expect(ids(await listContacts(api, { channels: ["messenger"] })).sort()).toEqual([m, both].sort())
    expect(ids(await listContacts(api, { channels: ["messenger", "telegram"] })).sort()).toEqual([m, t, both].sort())
    expect(ids(await listContacts(api, { inboxIds: [telegram] })).sort()).toEqual([t, both].sort())
    expect(ids(await listContacts(api, { channels: ["whatsapp"] }))).toEqual([])
    expect(ids(await listContacts(api)).sort()).toEqual([m, t, both, none, none2].sort())
    expectInvalid(await listContacts(api, { channels: ["fax"] }), "channels.0")
    expectInvalid(await listContacts(api, { inboxIds: ["abc"] }), "inboxIds.0")

    // count: without keyword/filter/keys it sums InboxContactStat (contacts
    // per inbox, kept by a ContactInbox trigger), so a contact on two inboxes
    // counts twice and one without an inbox not at all (4 = 2 + 2, real: 5);
    // any filter, even an empty one, makes it a real count.
    expect((await api.get("/v1/contacts/count")).json).toEqual({ total: 4, totalCountCapped: false })
    expect((await api.get(`/v1/contacts/count${qs({ contactFilter: JSON.stringify(ALL) })}`)).json).toEqual({
      total: 5,
      totalCountCapped: false,
    })
    expect((await api.get(`/v1/contacts/count${qs({ channels: ["messenger"] })}`)).json).toEqual({
      total: 2,
      totalCountCapped: false,
    })
    const bounded = await api.get(`/v1/contacts/count${qs({ countMode: "bounded", contactFilter: JSON.stringify(ALL) })}`)
    expect(bounded.status, bounded.text).toBe(200)
    expect(bounded.json).toMatchObject({ total: 5, totalCountCapped: false })
    expect(bounded.json).toMatchSnapshot()
    expectInvalid(await api.get(`/v1/contacts/count${qs({ countMode: "fast" })}`), "countMode")
  })
})

describe("contacts.findByCustomField (deprecated)", () => {
  it("matches email / phone columns or a custom field value exactly, newest update first", async () => {
    const { tenant, api } = await newTenant("p106-find-cf")
    const fx = await ext()
    const fieldId = await createCustomField(api, "member_code", "shortText")
    const a = await fx.createContact({ workspaceId: tenant.workspaceId, firstName: "A", email: "a@parity.test", phoneNumber: "+12025550101" })
    const b = await fx.createContact({ workspaceId: tenant.workspaceId, firstName: "B", email: "b@parity.test" })
    await fx.setCustomFieldValue({ contactId: a.contactId, customFieldId: fieldId, value: "X-1" })
    await fx.setCustomFieldValue({ contactId: b.contactId, customFieldId: fieldId, value: "x-1" })

    const find = (customFieldId: string, value: string) =>
      api.get<{ data: Array<{ id: string }> }>(`/v1/contacts/find-by-custom-field${qs({ customFieldId, value })}`)
    const idsOf = async (customFieldId: string, value: string) => {
      const response = await find(customFieldId, value)
      expect(response.status, response.text).toBe(200)
      expect(Object.keys(response.json)).toEqual(["data"])
      return response.json.data.map((row) => row.id)
    }
    expect(await idsOf("email", "a@parity.test")).toEqual([a.contactId])
    expect(await idsOf("email", "A@parity.test")).toEqual([])
    expect(await idsOf("phone", "+12025550101")).toEqual([a.contactId])
    expect(await idsOf(fieldId, "X-1")).toEqual([a.contactId])
    expect(await idsOf(fieldId, "x-1")).toEqual([b.contactId])
    expect(await idsOf(fieldId, "X")).toEqual([])
    const row = (await find("email", "a@parity.test")).json.data[0] as Record<string, unknown>
    expect(Object.keys(row)).toEqual(expect.arrayContaining(["tags", "contactCustomFields", "contactInboxes", "conversation"]))
    expectError(await api.get("/v1/contacts/find-by-custom-field?customFieldId=email"), 422, "invalidRequestData")
  })
})

describe("filter input on list / count / search", () => {
  let api: Api
  beforeAll(async () => {
    ;({ api } = await newTenant("p106-filter-input"))
  })

  it("an invalid contactFilter is 422 (not ignored) on every surface", async () => {
    const bad = [
      "not json",
      JSON.stringify({ operator: "xor", conditions: [] }),
      JSON.stringify({ operator: "and" }),
      JSON.stringify(filter([{ field: "noSuchField", operator: "eq", value: "x" }])),
      JSON.stringify(filter([{ field: "gender", operator: "contains", value: "male" }])),
      JSON.stringify(filter([{ field: "fullName", operator: "eq" }])),
      JSON.stringify(filter([{ field: "contactCreatedAt", operator: "isBetween", value: ["2025-01-01"] }])),
      JSON.stringify(filter([{ field: "isWithinWorkingHours", operator: "eq", value: "true" }])),
    ]
    for (const contactFilter of bad) {
      expectError(await api.get(`/v1/contacts${qs({ contactFilter })}`), 422, "invalidRequestData")
      expectError(await api.get(`/v1/contacts/count${qs({ contactFilter })}`), 422, "invalidRequestData")
      expectError(await api.post("/v1/contacts/search", { contactFilter }), 422, "invalidRequestData")
    }
  })

  it("the filter may be an object in the POST body", async () => {
    const response = await api.post<ListBody>("/v1/contacts/search", {
      contactFilter: filter([{ field: "fullName", operator: "isEmpty" }]),
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ data: [], totalCount: 0, pageCount: 0, totalCountCapped: false })
  })
})
