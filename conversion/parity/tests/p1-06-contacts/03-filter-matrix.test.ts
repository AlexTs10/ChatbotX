// contactFilter matrix on contacts.list (and contacts.count for the same
// filter): every field group the public API accepts and their operators,
// including NULL / empty / negation logic, date windows and time zones,
// relation fields (inboxes, conversation, tags, sequences) and custom / bot
// fields per value type. One workspace, five contacts with known attributes.

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import {
  type Api,
  at,
  contactPath,
  createCustomField,
  createSequence,
  createTag,
  ext,
  filter,
  idOf,
  ids,
  listContacts,
  newTenant,
  qs,
} from "./helpers.ts"

type Key = "ann" | "ben" | "cat" | "dan" | "eve"
const ALL: Key[] = ["ann", "ben", "cat", "dan", "eve"]
const except = (...keys: Key[]): Key[] => ALL.filter((key) => !keys.includes(key))

let tenant: Tenant
let api: Api
const id: Record<Key, string> = { ann: "", ben: "", cat: "", dan: "", eve: "" }
const ref: Record<string, string> = {}

beforeAll(async () => {
  ;({ tenant, api } = await newTenant("p106-filter"))
  const fx = await ext()
  const ws = tenant.workspaceId
  const hoursAgo = (hours: number) => new Date(Date.now() - hours * 3_600_000).toISOString()

  const messenger = (await fx.createInbox({ workspaceId: ws, channel: "messenger", name: "M" })).inboxId
  const telegram = (await fx.createInbox({ workspaceId: ws, channel: "telegram", name: "T" })).inboxId
  const whatsapp = (await fx.createInbox({ workspaceId: ws, channel: "whatsapp", name: "W" })).inboxId
  Object.assign(ref, { messenger, telegram, whatsapp, owner: tenant.userId })

  ref.vip = await createTag(api, "vip")
  ref.lead = await createTag(api, "lead")
  ref.text = await createCustomField(api, "cf_text", "shortText")
  ref.num = await createCustomField(api, "cf_num", "number")
  ref.date = await createCustomField(api, "cf_date", "date")
  ref.dt = await createCustomField(api, "cf_dt", "datetime")
  ref.bool = await createCustomField(api, "cf_bool", "boolean")
  ref.long = await createCustomField(api, "cf_long", "longText")
  ref.seq = await createSequence(api, "drip")

  id.ann = (
    await fx.createContact({
      workspaceId: ws,
      firstName: "Ann",
      lastName: "Lee",
      email: "ann@parity.test",
      phoneNumber: "+12025550001",
      gender: "female",
      country: "US",
      locale: "en_US",
      timezone: "America/New_York",
      createdAt: at("2025-02-10T10:00:00Z"),
      broadcastSubscribedAt: at("2025-02-10T10:00:00Z"),
      emailVerified: true,
      inboxes: [
        {
          inboxId: messenger,
          channel: "messenger",
          language: "en",
          lastIncomingMessageAt: hoursAgo(1),
          lastOutboundMessageAt: hoursAgo(2),
          contactLastReadAt: hoursAgo(0.5),
        },
      ],
      conversation: { followed: true, assignedUserId: tenant.userId },
    })
  ).contactId
  id.ben = (
    await fx.createContact({
      workspaceId: ws,
      firstName: "Ben",
      lastName: "Ng",
      email: "ben@example.org",
      gender: "male",
      country: "VN",
      timezone: "Asia/Ho_Chi_Minh",
      createdAt: at("2025-02-11T23:30:00Z"),
      blockedAt: at("2025-03-01T00:00:00Z"),
      emailOptIn: false,
      inboxes: [
        {
          inboxId: telegram,
          channel: "telegram",
          source: "ads",
          language: "vi",
          lastIncomingMessageAt: hoursAgo(72),
          lastOutboundMessageAt: hoursAgo(24),
        },
      ],
      conversation: { archivedAt: at("2025-03-01T00:00:00Z") },
    })
  ).contactId
  id.cat = (
    await fx.createContact({
      workspaceId: ws,
      firstName: "Cat",
      phoneNumber: "+84901110000",
      country: "FR",
      createdAt: at("2025-02-12T00:00:00Z"),
      inboxes: [
        { inboxId: messenger, channel: "messenger" },
        { inboxId: whatsapp, channel: "whatsapp", source: "imported", consecutiveFailedReply: 3 },
      ],
      conversation: { botEnabled: false, lastActivityAt: hoursAgo(5) },
    })
  ).contactId
  id.dan = (await fx.createContact({ workspaceId: ws, createdAt: at("2025-02-13T00:00:00Z"), conversation: null })).contactId
  id.eve = (
    await fx.createContact({
      workspaceId: ws,
      firstName: "Eve",
      lastName: "Lee",
      email: "",
      phoneNumber: "",
      country: "",
      createdAt: at("2025-02-14T00:00:00Z"),
      inboxes: [{ inboxId: whatsapp, channel: "whatsapp" }],
    })
  ).contactId

  for (const tag of [ref.vip, ref.lead]) {
    expect((await api.post(contactPath(idOf(id.ben), "/tags"), { tagIds: [tag] })).status).toBe(204)
  }
  expect((await api.post(contactPath(idOf(id.ann), "/tags"), { tagIds: [ref.vip] })).status).toBe(204)
  expect((await api.post(contactPath(idOf(id.ann), "/sequences"), { sequenceIds: [ref.seq] })).status).toBe(204)

  const values: Array<[Key, string, string]> = [
    ["ann", "text", "Gold"],
    ["ben", "text", "silver"],
    ["eve", "text", ""],
    ["ann", "num", "10"],
    ["ben", "num", "2.5"],
    ["cat", "num", "-3"],
    ["ann", "date", "2025-05-01T00:00:00Z"],
    ["ann", "dt", "2025-05-01T12:00:00.000Z"],
    ["ann", "bool", "true"],
    ["ben", "bool", "false"],
    ["eve", "long", "Long text body"],
  ]
  for (const [key, field, value] of values) {
    await fx.setCustomFieldValue({ contactId: id[key], customFieldId: ref[field] as string, value })
  }
})

/** Placeholders `$messenger`, `$vip`, ... in a filter are replaced by setup ids. */
const resolve = (value: unknown): unknown => {
  if (typeof value === "string" && value.startsWith("$")) {
    return ref[value.slice(1)] ?? value
  }
  if (typeof value === "string" && value.startsWith("u_$")) {
    return `u_${ref[value.slice(3)]}`
  }
  if (Array.isArray(value)) {
    return value.map(resolve)
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, resolve(v)]))
  }
  return value
}

type Case = [string, Record<string, unknown>, Key[]]

const check = async (condition: Record<string, unknown>, expected: Key[], timezone?: string) => {
  const contactFilter = resolve(filter([condition], "and", timezone))
  const response = await listContacts(api, { contactFilter })
  const want = expected.map((key) => id[key]).sort()
  expect(ids(response).sort(), JSON.stringify(contactFilter)).toEqual(want)
  expect(response.json.totalCount).toBe(want.length)
}

const runCases = (cases: Case[]) =>
  it.each(cases)("%s", async (_label, condition, expected) => {
    await check(condition, expected)
  })

const cond = (field: string, operator: string, value?: unknown) => ({
  field,
  operator,
  ...(value === undefined ? {} : { value }),
})

describe("text fields (fullName, email, phone): case-insensitive ILIKE", () => {
  runCases([
    ["fullName eq (case-insensitive)", cond("fullName", "eq", "ann lee"), ["ann"]],
    ["fullName ne (NULL matches)", cond("fullName", "ne", "Ann Lee"), except("ann")],
    ["fullName contains", cond("fullName", "contains", "LEE"), ["ann", "eve"]],
    ["fullName notContains (NULL matches)", cond("fullName", "notContains", "lee"), ["ben", "cat", "dan"]],
    ["fullName startsWith", cond("fullName", "startsWith", "b"), ["ben"]],
    ["fullName endsWith", cond("fullName", "endsWith", "NG"), ["ben"]],
    ["fullName isEmpty", cond("fullName", "isEmpty"), ["dan"]],
    ["fullName isNotEmpty", cond("fullName", "isNotEmpty"), except("dan")],
    ["fullName contains a LIKE wildcard literally", cond("fullName", "contains", "%"), []],
    ["email contains", cond("email", "contains", "@parity"), ["ann"]],
    ["email endsWith", cond("email", "endsWith", ".org"), ["ben"]],
    ["email ne", cond("email", "ne", "ann@parity.test"), except("ann")],
    ["email isEmpty (NULL or '')", cond("email", "isEmpty"), ["cat", "dan", "eve"]],
    ["email isNotEmpty", cond("email", "isNotEmpty"), ["ann", "ben"]],
    ["phone startsWith", cond("phone", "startsWith", "+84"), ["cat"]],
    ["phone eq", cond("phone", "eq", "+12025550001"), ["ann"]],
    ["phone isEmpty", cond("phone", "isEmpty"), ["ben", "dan", "eve"]],
  ])
})

describe("select / multi-select columns: exact, case-sensitive", () => {
  runCases([
    ["gender eq", cond("gender", "eq", "female"), ["ann"]],
    ["gender eq list", cond("gender", "eq", ["female", "male"]), ["ann", "ben"]],
    ["gender ne (NULL matches)", cond("gender", "ne", "female"), except("ann")],
    ["gender isEmpty", cond("gender", "isEmpty"), ["cat", "dan", "eve"]],
    ["country eq", cond("country", "eq", "US"), ["ann"]],
    ["country eq is case-sensitive", cond("country", "eq", "us"), []],
    ["country eq list", cond("country", "eq", ["US", "VN"]), ["ann", "ben"]],
    ["country ne list", cond("country", "ne", ["US", "VN"]), ["cat", "dan", "eve"]],
    ["country isEmpty (NULL or '')", cond("country", "isEmpty"), ["dan", "eve"]],
    ["continent EU", cond("continent", "eq", "EU"), ["cat"]],
    ["continent NA", cond("continent", "eq", "NA"), ["ann"]],
    ["continent AS", cond("continent", "eq", "AS"), ["ben"]],
    ["continent unknown", cond("continent", "eq", "unknown"), ["dan", "eve"]],
    ["continent ne EU (no country matches)", cond("continent", "ne", "EU"), except("cat")],
    ["timezone eq", cond("timezone", "eq", "Asia/Ho_Chi_Minh"), ["ben"]],
    ["timezone isEmpty", cond("timezone", "isEmpty"), ["cat", "dan", "eve"]],
    ["locale eq", cond("locale", "eq", "en_US"), ["ann"]],
  ])
})

describe("boolean and presence fields", () => {
  runCases([
    ["subscribedToBroadcast true", cond("subscribedToBroadcast", "eq", "true"), ["ann"]],
    ["subscribedToBroadcast false", cond("subscribedToBroadcast", "eq", "false"), except("ann")],
    ["subscribedToBroadcast isEmpty", cond("subscribedToBroadcast", "isEmpty"), except("ann")],
    ["blocked true", cond("blocked", "eq", "true"), ["ben"]],
    ["blocked false", cond("blocked", "eq", "false"), except("ben")],
    ["emailWasVerified true", cond("emailWasVerified", "eq", "true"), ["ann"]],
    ["emailWasVerified false", cond("emailWasVerified", "eq", "false"), except("ann")],
    ["optedInForEmail false", cond("optedInForEmail", "eq", "false"), ["ben"]],
    ["optedInForEmail true", cond("optedInForEmail", "eq", "true"), except("ben")],
    ["hasContactInfo in phone", cond("hasContactInfo", "in", ["phone"]), ["ann", "cat"]],
    ["hasContactInfo in email", cond("hasContactInfo", "in", ["email"]), ["ann", "ben"]],
    ["hasContactInfo in phoneAndEmail", cond("hasContactInfo", "in", ["phoneAndEmail"]), ["ann"]],
    ["hasContactInfo in phone, email", cond("hasContactInfo", "in", ["phone", "email"]), ["ann", "ben", "cat"]],
    ["hasContactInfo notIn email", cond("hasContactInfo", "notIn", ["email"]), ["cat", "dan", "eve"]],
    ["hasContactInfo isEmpty", cond("hasContactInfo", "isEmpty"), ["dan", "eve"]],
    ["existingContact true", cond("existingContact", "eq", "true"), ["ann", "ben", "cat"]],
    ["existingContact false", cond("existingContact", "eq", "false"), ["dan", "eve"]],
  ])
})

describe("contactCreatedAt: precision windows, time zone, offsets", () => {
  runCases([
    ["eq day (UTC)", cond("contactCreatedAt", "eq", "2025-02-11"), ["ben"]],
    ["ne day (never NULL)", cond("contactCreatedAt", "ne", "2025-02-10"), except("ann")],
    ["gt day = from the next day", cond("contactCreatedAt", "gt", "2025-02-12"), ["dan", "eve"]],
    ["gte day", cond("contactCreatedAt", "gte", "2025-02-12"), ["cat", "dan", "eve"]],
    ["lt day", cond("contactCreatedAt", "lt", "2025-02-11"), ["ann"]],
    ["lte day = to the end of the day", cond("contactCreatedAt", "lte", "2025-02-11"), ["ann", "ben"]],
    ["isBetween days (inclusive)", cond("contactCreatedAt", "isBetween", ["2025-02-11", "2025-02-12"]), ["ben", "cat"]],
    ["notBetween days", cond("contactCreatedAt", "notBetween", ["2025-02-11", "2025-02-12"]), ["ann", "dan", "eve"]],
    ["eq minute", cond("contactCreatedAt", "eq", "2025-02-10T10:00"), ["ann"]],
    ["eq minute, other minute", cond("contactCreatedAt", "eq", "2025-02-10T10:01"), []],
    ["eq with an explicit offset", cond("contactCreatedAt", "eq", "2025-02-10T17:00+07:00"), ["ann"]],
    ["eq with Z", cond("contactCreatedAt", "eq", "2025-02-11T23:30:00Z"), ["ben"]],
    ["isEmpty (never)", cond("contactCreatedAt", "isEmpty"), []],
    ["minutes ago gt (older than)", cond("contactCreatedDateMinutesAgo", "gt", "60"), ALL],
    ["minutes ago lt (newer than)", cond("contactCreatedDateMinutesAgo", "lt", "60"), []],
  ])

  it("a naive day is read in the filter's time zone", async () => {
    // Asia/Ho_Chi_Minh (UTC+7): 2025-02-12 local = [02-11T17:00Z, 02-12T17:00Z).
    await check(cond("contactCreatedAt", "eq", "2025-02-12"), ["ben", "cat"], "Asia/Ho_Chi_Minh")
    // An unknown zone falls back to UTC.
    await check(cond("contactCreatedAt", "eq", "2025-02-12"), ["cat"], "Mars/Olympus")
    // An explicit offset wins over the filter zone.
    await check(cond("contactCreatedAt", "eq", "2025-02-11T23:30:00Z"), ["ben"], "Asia/Ho_Chi_Minh")
  })

  it("value formats: rejected (422) or the condition is dropped (whole workspace)", async () => {
    // Recorded on the original: some formats are refused by the schema,
    // others pass it and the condition builds no SQL, so every contact matches.
    const outcome: Record<string, string> = {}
    for (const [field, value] of [
      ["contactCreatedAt", "11/02/2025"],
      ["contactCreatedAt", "2025-02-30"],
      ["contactCreatedAt", "2025-13-01"],
      ["contactCreatedAt", "yesterday"],
      ["contactCreatedDateMinutesAgo", "1.5"],
      ["contactCreatedDateMinutesAgo", "abc"],
    ] as const) {
      const response = await listContacts(api, { contactFilter: filter([cond(field, "eq", value)]) })
      outcome[`${field} ${value}`] =
        response.status === 200 ? `200 ${response.json.totalCount === ALL.length ? "all" : response.json.totalCount}` : String(response.status)
      expect([200, 422]).toContain(response.status)
    }
    expect(outcome).toMatchSnapshot()
  })
})

describe("contact inbox fields", () => {
  runCases([
    ["currentChannel in", cond("currentChannel", "in", ["messenger"]), ["ann", "cat"]],
    ["currentChannel eq", cond("currentChannel", "eq", "whatsapp"), ["cat", "eve"]],
    ["currentChannel notIn (no inbox matches)", cond("currentChannel", "notIn", ["messenger"]), ["ben", "dan", "eve"]],
    ["currentChannel isEmpty = no inbox", cond("currentChannel", "isEmpty"), ["dan"]],
    ["source in ads", cond("source", "in", ["ads"]), ["ben"]],
    ["source in imported", cond("source", "in", ["imported"]), ["cat"]],
    ["source notIn direct = no direct inbox", cond("source", "notIn", ["direct"]), ["ben", "dan"]],
    ["inbox in", cond("inbox", "in", ["$messenger"]), ["ann", "cat"]],
    ["inbox ne", cond("inbox", "ne", "$telegram"), except("ben")],
    ["language eq", cond("language", "eq", "vi"), ["ben"]],
    ["language in", cond("language", "in", ["en", "vi"]), ["ann", "ben"]],
    ["language notIn", cond("language", "notIn", ["en"]), except("ann")],
    ["language isEmpty", cond("language", "isEmpty"), ["cat", "dan", "eve"]],
    ["lastInteraction isEmpty", cond("lastInteraction", "isEmpty"), ["cat", "dan", "eve"]],
    ["lastInteraction isNotEmpty", cond("lastInteraction", "isNotEmpty"), ["ann", "ben"]],
    ["lastInteractionMinutesAgo lt 120", cond("lastInteractionMinutesAgo", "lt", "120"), ["ann"]],
    ["lastInteractionMinutesAgo gt 120", cond("lastInteractionMinutesAgo", "gt", "120"), ["ben"]],
    ["lastSeen isEmpty", cond("lastSeen", "isEmpty"), except("ann")],
    ["lastSent isEmpty", cond("lastSent", "isEmpty"), ["cat", "dan", "eve"]],
    ["interactedInLast24h true", cond("interactedInLast24h", "eq", "true"), ["ann"]],
    ["interactedInLast24h false", cond("interactedInLast24h", "eq", "false"), except("ann")],
    ["unreplied true", cond("unreplied", "eq", "true"), ["ann"]],
    ["unreplied false", cond("unreplied", "eq", "false"), except("ann")],
    ["consecutiveAiFailures gt 0", cond("consecutiveAiFailures", "gt", "0"), ["cat"]],
    ["consecutiveAiFailures eq 0", cond("consecutiveAiFailures", "eq", "0"), ["ann", "ben", "eve"]],
    ["consecutiveAiFailures isBetween", cond("consecutiveAiFailures", "isBetween", ["1", "3"]), ["cat"]],
  ])
})

describe("conversation fields", () => {
  runCases([
    ["archived true", cond("archived", "eq", "true"), ["ben"]],
    ["archived false (no conversation matches)", cond("archived", "eq", "false"), except("ben")],
    ["followUp true", cond("followUp", "eq", "true"), ["ann"]],
    ["followUp isEmpty", cond("followUp", "isEmpty"), except("ann")],
    ["conversationTransferredToHuman true", cond("conversationTransferredToHuman", "eq", "true"), ["cat"]],
    ["unread true", cond("unread", "eq", "true"), ["cat"]],
    ["conversationAssigned in user", cond("conversationAssigned", "in", ["u_$owner"]), ["ann"]],
    ["conversationAssigned notIn user", cond("conversationAssigned", "notIn", ["u_$owner"]), except("ann")],
    ["conversationAssigned isEmpty", cond("conversationAssigned", "isEmpty"), except("ann")],
  ])
})

describe("tags and sequences", () => {
  runCases([
    ["tags in", cond("tags", "in", ["$vip"]), ["ann", "ben"]],
    ["tags in (any of)", cond("tags", "in", ["$vip", "$lead"]), ["ann", "ben"]],
    ["tags eq", cond("tags", "eq", "$lead"), ["ben"]],
    ["tags notIn (none of)", cond("tags", "notIn", ["$vip"]), ["cat", "dan", "eve"]],
    ["tags ne", cond("tags", "ne", "$lead"), except("ben")],
    ["tags isEmpty", cond("tags", "isEmpty"), ["cat", "dan", "eve"]],
    ["tags in an unknown id", cond("tags", "in", ["1"]), []],
    ["subscribedToDripCampaign in", cond("subscribedToDripCampaign", "in", ["$seq"]), ["ann"]],
    ["subscribedToDripCampaign isEmpty", cond("subscribedToDripCampaign", "isEmpty"), except("ann")],
  ])
})

const cf = (fieldRef: string, valueType: string, operator: string, value?: unknown, customFieldType?: string) => ({
  field: "customField",
  customFieldId: `$${fieldRef}`,
  valueType,
  ...(customFieldType ? { customFieldType } : {}),
  operator,
  ...(value === undefined ? {} : { value }),
})

describe("custom fields", () => {
  runCases([
    ["text eq is case-sensitive", cf("text", "text", "eq", "Gold"), ["ann"]],
    ["text eq other case", cf("text", "text", "eq", "gold"), []],
    ["text contains (ILIKE)", cf("text", "text", "contains", "ILV"), ["ben"]],
    ["text startsWith", cf("text", "text", "startsWith", "go"), ["ann"]],
    ["text ne (no value matches)", cf("text", "text", "ne", "Gold"), except("ann")],
    ["text notContains", cf("text", "text", "notContains", "old"), except("ann")],
    ["text isEmpty ('' counts as empty)", cf("text", "text", "isEmpty"), ["cat", "dan", "eve"]],
    ["text isNotEmpty", cf("text", "text", "isNotEmpty"), ["ann", "ben"]],
    ["number gt", cf("num", "number", "gt", "2"), ["ann", "ben"]],
    ["number lt negative", cf("num", "number", "lt", "0"), ["cat"]],
    ["number eq", cf("num", "number", "eq", "10"), ["ann"]],
    ["number eq 10.0", cf("num", "number", "eq", "10.0"), ["ann"]],
    ["number gte decimal", cf("num", "number", "gte", "2.5"), ["ann", "ben"]],
    ["number isBetween (inclusive)", cf("num", "number", "isBetween", ["-3", "2.5"]), ["ben", "cat"]],
    ["number notBetween", cf("num", "number", "notBetween", ["-3", "2.5"]), ["ann", "dan", "eve"]],
    ["number ne", cf("num", "number", "ne", "10"), except("ann")],
    ["number contains is textual", cf("num", "number", "contains", "5"), ["ben"]],
    ["number isEmpty", cf("num", "number", "isEmpty"), ["dan", "eve"]],
    ["date eq", cf("date", "datetime", "eq", "2025-05-01", "date"), ["ann"]],
    ["date gt", cf("date", "datetime", "gt", "2025-04-30", "date"), ["ann"]],
    ["date lt", cf("date", "datetime", "lt", "2025-05-01", "date"), []],
    ["date isEmpty", cf("date", "datetime", "isEmpty", undefined, "date"), except("ann")],
    ["date without customFieldType", cf("date", "datetime", "eq", "2025-05-01"), ["ann"]],
    ["datetime eq day (UTC)", cf("dt", "datetime", "eq", "2025-05-01", "datetime"), ["ann"]],
    ["datetime gt minute", cf("dt", "datetime", "gt", "2025-05-01T11:59", "datetime"), ["ann"]],
    ["datetime lt minute", cf("dt", "datetime", "lt", "2025-05-01T12:00", "datetime"), []],
    ["boolean eq true", cf("bool", "boolean", "eq", "true"), ["ann"]],
    ["boolean eq yes (canonicalized)", cf("bool", "boolean", "eq", "yes"), ["ann"]],
    ["boolean eq false", cf("bool", "boolean", "eq", "false"), ["ben"]],
    ["boolean isEmpty", cf("bool", "boolean", "isEmpty"), ["cat", "dan", "eve"]],
    ["boolean isNotEmpty", cf("bool", "boolean", "isNotEmpty"), ["ann", "ben"]],
    ["longText eq", cf("long", "text", "eq", "Long text body", "longText"), ["eve"]],
    ["longText isNotEmpty", cf("long", "text", "isNotEmpty", undefined, "longText"), ["eve"]],
    ["unknown custom field id", { ...cf("text", "text", "isNotEmpty"), customFieldId: "1" }, []],
  ])

  it("a datetime day is read in the filter's time zone", async () => {
    // 2025-05-01T12:00Z: inside 2025-05-01 in New York (UTC-4), outside it on Kiritimati (UTC+14).
    await check(cf("dt", "datetime", "eq", "2025-05-01", "datetime"), ["ann"], "America/New_York")
    await check(cf("dt", "datetime", "eq", "2025-05-01", "datetime"), [], "Pacific/Kiritimati")
    await check(cf("dt", "datetime", "eq", "2025-05-02", "datetime"), ["ann"], "Pacific/Kiritimati")
  })

  it("operators outside the value type are 422", async () => {
    for (const condition of [
      cf("long", "text", "contains", "Long", "longText"),
      cf("bool", "boolean", "gt", "1"),
      cf("text", "text", "gt", "1"),
      cf("dt", "datetime", "contains", "2025"),
      { ...cf("text", "text", "eq", "x"), valueType: "color" },
    ]) {
      const response = await listContacts(api, { contactFilter: resolve(filter([condition])) })
      expect(response.status, `${JSON.stringify(condition)} ${response.text}`).toBe(422)
    }
  })
})

describe("bot fields (workspace-level: all contacts or none)", () => {
  it("matches every contact when the workspace value matches, else none", async () => {
    const response = await api.post<{ id: string }>("/v1/bot-fields", { name: "bf_mode", type: "shortText", value: "on" })
    expect(response.status, response.text).toBe(201)
    const bot = (operator: string, value?: string) => ({
      field: "botField",
      botFieldId: response.json.id,
      valueType: "text",
      botFieldType: "shortText",
      operator,
      ...(value === undefined ? {} : { value }),
    })
    await check(bot("eq", "on"), ALL)
    await check(bot("eq", "off"), [])
    await check(bot("isNotEmpty"), ALL)
  })
})

describe("operator and / or, count", () => {
  it("and / or combine the conditions; count agrees with the list", async () => {
    const or = resolve(filter([cond("gender", "eq", "female"), cond("country", "eq", "VN")], "or"))
    const and = resolve(filter([cond("tags", "in", ["$vip"]), cond("blocked", "eq", "true")], "and"))
    const empty = filter([])
    for (const [contactFilter, expected] of [
      [or, ["ann", "ben"]],
      [and, ["ben"]],
      [empty, ALL],
    ] as Array<[unknown, Key[]]>) {
      const want = expected.map((key) => id[key]).sort()
      expect(ids(await listContacts(api, { contactFilter })).sort()).toEqual(want)
      const count = await api.get(`/v1/contacts/count${qs({ contactFilter: JSON.stringify(contactFilter) })}`)
      expect(count.status, count.text).toBe(200)
      expect(count.json).toEqual({ total: want.length, totalCountCapped: false })
      const search = await api.post<{ data: Array<{ id: string }> }>("/v1/contacts/search", { contactFilter })
      expect(search.json.data.map((row) => row.id).sort()).toEqual(want)
    }
  })

  it("keyword and filter both apply (AND)", async () => {
    const response = await listContacts(api, { keyword: "lee", contactFilter: filter([cond("gender", "eq", "female")]) })
    expect(ids(response)).toEqual([id.ann])
  })
})
