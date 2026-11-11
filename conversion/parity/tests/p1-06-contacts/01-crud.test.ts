// contacts.create / get / update / updateLegacy / delete / block / unblock /
// upsert: full response shapes, identifier rules, validation and business
// errors, side effects visible through other routes.

import { beforeAll, describe, expect, it } from "vitest"
import { eventually } from "../../src/eventually.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import { Normalizer } from "../../src/normalize.ts"
import { config } from "../../src/config.ts"
import {
  AVATAR_PROXY,
  type Api,
  expectContactShape,
  contactPath,
  createCustomField,
  createWebchat,
  createWebchatContact,
  ext,
  expectError,
  expectInvalid,
  idOf,
  newTenant,
  seedContact,
  uniqueName,
} from "./helpers.ts"

type Contact = {
  id: string
  workspaceId: string
  firstName: string | null
  lastName: string | null
  fullName: string | null
  email: string | null
  phoneNumber: string | null
  gender: string | null
  blockedAt: string | null
  contactInboxes?: Array<{ id: string; channel: string; source: string; sourceId: string; inboxId: string; inbox: { id: string } }>
  conversation?: { id: string } | null
  tags?: unknown[]
  contactCustomFields?: unknown[]
  [key: string]: unknown
}

describe("contacts.create", () => {
  let tenant: Tenant
  let api: Api
  let webchatInboxId: string
  let messengerInboxId: string
  let smtpInboxId: string
  let whatsappInboxId: string

  beforeAll(async () => {
    ;({ tenant, api } = await newTenant("p106-create"))
    const fx = await ext()
    // Phone numbers without "+" need the workspace country (set before any HTTP call).
    await fx.setWorkspaceTargetCountry({ workspaceId: tenant.workspaceId, targetCountry: "VN" })
    webchatInboxId = (await createWebchat(api)).inboxId
    messengerInboxId = (await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "messenger", name: "M" })).inboxId
    smtpInboxId = (await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "smtp", name: "S" })).inboxId
    whatsappInboxId = (await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "whatsapp", name: "W" })).inboxId
  })

  it("webchat: 200 with the full contact, its inbox and an open conversation", async () => {
    const n = new Normalizer()
    const response = await api.post<Contact>("/v1/contacts", {
      channel: "webchat",
      inboxId: webchatInboxId,
      firstName: "  Ada ",
      lastName: "Lovelace",
      email: "ada@parity.test",
      gender: "female",
    })
    expect(response.status, response.text).toBe(200)
    const body = response.json
    expect(body).toMatchObject({
      workspaceId: tenant.workspaceId,
      firstName: "Ada",
      lastName: "Lovelace",
      fullName: "Ada Lovelace",
      email: "ada@parity.test",
      phoneNumber: null,
      gender: "female",
      blockedAt: null,
      tags: [],
      contactCustomFields: [],
    })
    expect(body.contactInboxes).toHaveLength(1)
    expect(body.contactInboxes?.[0]).toMatchObject({
      channel: "webchat",
      source: "direct",
      inboxId: webchatInboxId,
      inbox: { id: webchatInboxId },
    })
    expect(body.conversation).toMatchObject({ contactId: body.id, workspaceId: tenant.workspaceId })
    expectContactShape(n, body, {
      replace: { [tenant.workspaceId]: "<workspace>", [body.contactInboxes?.[0]?.sourceId ?? "-"]: "<sourceId>" },
    }).toMatchSnapshot()

    // GET returns the same resource.
    const get = await api.get<Contact>(contactPath(idOf(body.id)))
    expect(get.status, get.text).toBe(200)
    expect(get.json).toEqual(body)
  })

  it("messenger: the channel user id becomes the contact inbox sourceId", async () => {
    const psid = uniqueName("psid")
    const response = await api.post<Contact>("/v1/contacts", {
      channel: "messenger",
      inboxId: messengerInboxId,
      contactId: psid,
      email: "",
      gender: "male",
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ firstName: null, lastName: null, fullName: null, email: "", gender: "male" })
    expect(response.json.contactInboxes?.[0]).toMatchObject({ channel: "messenger", sourceId: psid, source: "direct" })
    // create returns the stored row (avatar null); get resolves a messenger
    // contact without avatar to a media proxy URL on the app origin.
    expect(response.json.avatar).toBeNull()
    const get = await api.get<Contact>(contactPath(idOf(response.json.id)))
    expect(get.json.avatar).toMatch(AVATAR_PROXY)
    expect(String(get.json.avatar).startsWith(`${config.baseUrl}/media/avatar/`)).toBe(true)
    // The token differs on every read.
    const again1 = await api.get<Contact>(contactPath(idOf(response.json.id)))
    expect(again1.json.avatar).not.toBe(get.json.avatar)

    // The same channel user id on the same inbox is refused.
    const again = await api.post("/v1/contacts", {
      channel: "messenger",
      inboxId: messengerInboxId,
      contactId: psid,
      email: "",
      gender: "male",
    })
    expectError(again, 422, "validation", { message: "This contact already exists on the selected inbox" })
  })

  it("smtp: the email is the inbox identity and is required", async () => {
    const email = `${uniqueName("mail")}@parity.test`
    const response = await api.post<Contact>("/v1/contacts", { channel: "smtp", inboxId: smtpInboxId, email, gender: "unknown" })
    expect(response.status, response.text).toBe(200)
    expect(response.json.contactInboxes?.[0]).toMatchObject({ channel: "smtp", sourceId: email })

    expectInvalid(await api.post("/v1/contacts", { channel: "smtp", inboxId: smtpInboxId, email: "", gender: "unknown" }), "email")
  })

  it("whatsapp: national numbers use the workspace country; E.164 is stored; a used phone is refused", async () => {
    const local = `09${String(Date.now()).slice(-8)}`
    const response = await api.post<Contact>("/v1/contacts", {
      channel: "whatsapp",
      inboxId: whatsappInboxId,
      phoneNumber: local,
      email: "",
      gender: "unknown",
    })
    expect(response.status, response.text).toBe(200)
    const e164 = `+84${local.slice(1)}`
    expect(response.json.phoneNumber).toBe(e164)
    expect(response.json.contactInboxes?.[0]).toMatchObject({ channel: "whatsapp", sourceId: e164.slice(1) })

    // The phone is unique per workspace (any channel).
    const dup = await api.post("/v1/contacts", {
      channel: "webchat",
      inboxId: webchatInboxId,
      phoneNumber: e164,
      email: "",
      gender: "unknown",
    })
    expectError(dup, 422, "validation", { message: "Phone number is exists" })

    // WhatsApp needs a phone number or a BSUID.
    expectInvalid(
      await api.post("/v1/contacts", { channel: "whatsapp", inboxId: whatsappInboxId, email: "", gender: "unknown" }),
      "phoneNumber",
    )
    // A BSUID on another channel is refused.
    expectInvalid(
      await api.post("/v1/contacts", {
        channel: "webchat",
        inboxId: webchatInboxId,
        sourceUserId: "US.123",
        email: "",
        gender: "unknown",
      }),
      "sourceUserId",
    )
  })

  it("input validation: 422 invalidRequestData with the issue paths", async () => {
    const base = { channel: "webchat", inboxId: webchatInboxId, email: "", gender: "unknown" }
    expectInvalid(await api.post("/v1/contacts", { ...base, email: undefined }), "email")
    expectInvalid(await api.post("/v1/contacts", { ...base, email: "not-an-email" }), "email")
    expectInvalid(await api.post("/v1/contacts", { ...base, gender: undefined }), "gender")
    expectInvalid(await api.post("/v1/contacts", { ...base, gender: "other" }), "gender")
    expectInvalid(await api.post("/v1/contacts", { ...base, channel: "fax" }), "channel")
    expectInvalid(await api.post("/v1/contacts", { ...base, inboxId: "abc" }), "inboxId")
    expectInvalid(await api.post("/v1/contacts", { ...base, firstName: "x".repeat(101) }), "firstName")
    expectInvalid(await api.post("/v1/contacts", { ...base, phoneNumber: "12345" }), "phoneNumber")
    expectInvalid(
      await api.post("/v1/contacts", { channel: "messenger", inboxId: messengerInboxId, email: "", gender: "unknown" }),
      "contactId",
    )
  })

  it("business errors: unknown inbox 404, inbox of another channel 422, number without country 422", async () => {
    const base = { email: "", gender: "unknown" }
    expectError(await api.post("/v1/contacts", { ...base, channel: "webchat", inboxId: "1" }), 404, "notFound", {
      message: "Inbox not found",
    })
    expectError(
      await api.post("/v1/contacts", { ...base, channel: "telegram", inboxId: messengerInboxId, contactId: "t1" }),
      422,
      "validation",
      { message: "Selected inbox does not match the selected source" },
    )
    // Another workspace has no country: a national number cannot be read.
    const other = await newTenant("p106-create-nocountry")
    const inbox = await createWebchat(other.api)
    expectError(
      await other.api.post("/v1/contacts", { ...base, channel: "webchat", inboxId: inbox.inboxId, phoneNumber: "0901234567" }),
      422,
      "validation",
      { message: "Please include the country code (e.g. +84)" },
    )
    // An inbox of this workspace is unknown to the other one.
    expectError(
      await other.api.post("/v1/contacts", { ...base, channel: "webchat", inboxId: webchatInboxId }),
      404,
      "notFound",
    )
  })

  it("Idempotency-Key: a replay returns the first response and creates nothing", async () => {
    const key = uniqueName("idem")
    const body = { channel: "webchat", inboxId: webchatInboxId, firstName: "Idem", email: "", gender: "unknown" }
    const first = await api.post<Contact>("/v1/contacts", body, { headers: { "idempotency-key": key } })
    expect(first.status, first.text).toBe(200)
    const second = await api.post<Contact>("/v1/contacts", body, { headers: { "idempotency-key": key } })
    expect(second.status, second.text).toBe(200)
    expect(second.json.id).toBe(first.json.id)
    expect(second.headers.get("idempotent-replayed")).toBe("true")
    // Same key with another body.
    expectError(
      await api.post("/v1/contacts", { ...body, firstName: "Other" }, { headers: { "idempotency-key": key } }),
      422,
      "idempotencyKeyReused",
    )
  })
})

describe("contacts.get identifiers", () => {
  let api: Api
  let contact: Contact
  beforeAll(async () => {
    const created = await newTenant("p106-get")
    api = created.api
    const inbox = await createWebchat(api)
    const response = await api.post<Contact>("/v1/contacts", {
      channel: "webchat",
      inboxId: inbox.inboxId,
      firstName: "Grace",
      email: "grace@parity.test",
      phoneNumber: "+14155550123",
      gender: "female",
    })
    expect(response.status, response.text).toBe(200)
    contact = response.json
  })

  it("id:, email: and phone: resolve the same contact", async () => {
    for (const identifier of [idOf(contact.id), "email:grace@parity.test", "phone:+14155550123"]) {
      const response = await api.get<Contact>(contactPath(identifier))
      expect(response.status, `${identifier} ${response.text}`).toBe(200)
      expect(response.json.id).toBe(contact.id)
    }
  })

  it("email match is exact (case-sensitive)", async () => {
    expectError(await api.get(contactPath("email:GRACE@parity.test")), 404, "notFound")
  })

  it("bad identifiers are 404 notFound with the parser's message", async () => {
    expectError(await api.get(contactPath("grace@parity.test")), 404, "notFound", { message: "Invalid identifier format" })
    expectError(await api.get(contactPath("id:")), 404, "notFound", { message: "Invalid identifier format" })
    expectError(await api.get(contactPath("name:Grace")), 404, "notFound", {
      message: "Invalid identifier format. Use id:, email:, or phone: prefix",
    })
    expectError(await api.get(contactPath("id:abc")), 404, "notFound", { message: "Contact not found" })
    expectError(await api.get(contactPath("id:1")), 404, "notFound", { message: "Contact not found" })
    expectError(await api.get(contactPath("email:nobody@parity.test")), 404, "notFound", { message: "Contact not found" })
  })

  it("a contact of another workspace is 404", async () => {
    const other = await newTenant("p106-get-other")
    expectError(await other.api.get(contactPath(idOf(contact.id))), 404, "notFound")
    expectError(await other.api.get(contactPath("email:grace@parity.test")), 404, "notFound")
  })
})

describe("contacts.update / updateLegacy", () => {
  let tenant: Tenant
  let api: Api
  beforeAll(async () => {
    ;({ tenant, api } = await newTenant("p106-update"))
  })

  it.each(["PATCH", "PUT"] as const)("%s: 204, standard and custom fields change, unknown keys are ignored", async (method) => {
    const seeded = await seedContact(tenant.workspaceId, { firstName: "Old", lastName: "Name", email: "old@parity.test" })
    const fieldId = await createCustomField(api, uniqueName("note"), "shortText")
    const response = await api.http.request(method, `/api${contactPath(idOf(seeded.contactId))}`, {
      bearer: tenant.token,
      noCookies: true,
      json: { firstName: "New", [fieldId]: "hello", notAField: "x" },
    })
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    const get = await api.get<Contact & { contactCustomFields: Array<{ customFieldId: string; value: string }> }>(
      contactPath(idOf(seeded.contactId)),
    )
    expect(get.json).toMatchObject({ firstName: "New", lastName: "Name", fullName: "New Name", email: "old@parity.test" })
    expect(get.json.contactCustomFields.map((row) => [row.customFieldId, row.value])).toEqual([[fieldId, "hello"]])
    expect(get.json).not.toHaveProperty("notAField")
  })

  it("non-string values are 422; an unknown contact is 404", async () => {
    const seeded = await seedContact(tenant.workspaceId, { firstName: "Typed" })
    expectError(await api.patch(contactPath(idOf(seeded.contactId)), { firstName: 5 }), 422, "invalidRequestData")
    expectError(await api.patch(contactPath("id:1"), { firstName: "x" }), 404, "notFound")
    expectError(await api.put(contactPath("id:1"), { firstName: "x" }), 404, "notFound")
  })
})

describe("contacts.delete / block / unblock", () => {
  let tenant: Tenant
  let api: Api
  beforeAll(async () => {
    ;({ tenant, api } = await newTenant("p106-delete"))
  })

  it("delete: 204, then the contact and its sub-resources are 404", async () => {
    const seeded = await seedContact(tenant.workspaceId, { firstName: "Gone", email: "gone@parity.test" })
    // Warm the identifier cache first: the delete must clear it.
    expect((await api.get(contactPath("email:gone@parity.test"))).status).toBe(200)
    const response = await api.delete(contactPath(idOf(seeded.contactId)))
    expect(response.status, response.text).toBe(204)
    expectError(await api.get(contactPath(idOf(seeded.contactId))), 404, "notFound")
    expectError(await api.get(contactPath("email:gone@parity.test")), 404, "notFound")
    expectError(await api.get(contactPath(idOf(seeded.contactId), "/tags")), 404, "notFound")
    expectError(await api.delete(contactPath(idOf(seeded.contactId))), 404, "notFound")
  })

  it("block sets blockedAt, unblock clears it; both are idempotent 204s", async () => {
    const started = Date.now()
    const seeded = await seedContact(tenant.workspaceId, { firstName: "Blocky" })
    const path = contactPath(idOf(seeded.contactId))
    expect((await api.post(`${path}/block`)).status).toBe(204)
    const blocked = await api.get<Contact>(path)
    expect(blocked.json.blockedAt).not.toBeNull()
    const blockedAt = Date.parse(blocked.json.blockedAt as string)
    expect(blockedAt).toBeGreaterThanOrEqual(started - 5_000)
    expect(blockedAt).toBeLessThanOrEqual(Date.now() + 5_000)
    expect((await api.post(`${path}/block`)).status).toBe(204)
    expect((await api.post(`${path}/unblock`)).status).toBe(204)
    expect((await api.get<Contact>(path)).json.blockedAt).toBeNull()
    expect((await api.post(`${path}/unblock`)).status).toBe(204)
    expectError(await api.post(contactPath("id:1", "/block")), 404, "notFound")
    expectError(await api.post(contactPath("id:1", "/unblock")), 404, "notFound")
  })

  it("block is recorded for analytics (blocked contacts count)", async () => {
    const { tenant: t, api: a } = await newTenant("p106-block-analytics")
    const seeded = await seedContact(t.workspaceId, { firstName: "Counted" })
    const from = new Date(Date.now() - 60 * 60 * 1000).toISOString()
    expect((await a.post(contactPath(idOf(seeded.contactId), "/block"))).status).toBe(204)
    // The analytics read is cached for 120 s per range, so each poll asks a new range.
    let attempt = 0
    await eventually(
      async () => {
        attempt += 1
        const to = new Date(Date.now() + 60 * 60 * 1000 + attempt * 1000).toISOString()
        const response = await a.get(
          `/v1/analytics/blocked-contacts-count?from=${encodeURIComponent(from)}&to=${encodeURIComponent(to)}`,
        )
        expect(response.status, response.text).toBe(200)
        expect(response.json).toEqual({ data: { count: 1 } })
      },
      { timeoutMs: 20_000, intervalMs: 500 },
    )
  })
})

describe("contacts.upsert", () => {
  it("without a webchat inbox a new email contact is 404 (Inbox not found)", async () => {
    const { api } = await newTenant("p106-upsert-noinbox")
    expectError(await api.post(contactPath("email:new@parity.test", "/upsert"), { firstName: "Up" }), 404, "notFound", {
      message: "Inbox not found",
    })
  })

  it("creates by email: / phone:, updates an existing one, and refuses an unknown id:", async () => {
    const n = new Normalizer()
    const { tenant, api } = await newTenant("p106-upsert")
    const inbox = await createWebchat(api)
    const email = `${uniqueName("up")}@parity.test`
    const created = await api.post<Contact>(contactPath(`email:${email}`, "/upsert"), {
      firstName: " Upsy ",
      gender: "female",
    })
    expect(created.status, created.text).toBe(200)
    expect(created.json).toMatchObject({ email, firstName: "Upsy", lastName: null, gender: "female", phoneNumber: null })
    expect(created.json.contactInboxes?.[0]).toMatchObject({ channel: "webchat", source: "api", inboxId: inbox.inboxId })
    expectContactShape(n, created.json, {
      replace: {
        [tenant.workspaceId]: "<workspace>",
        [email]: "<email>",
        [created.json.contactInboxes?.[0]?.sourceId ?? "-"]: "<sourceId>",
      },
    }).toMatchSnapshot()

    // Same identifier again: update in place.
    const updated = await api.post<Contact>(contactPath(`email:${email}`, "/upsert"), { lastName: "Again" })
    expect(updated.status, updated.text).toBe(200)
    expect(updated.json).toMatchObject({ id: created.json.id, firstName: "Upsy", lastName: "Again" })

    // phone: creates with that phone.
    const phone = `+1415${String(Date.now()).slice(-7)}`
    const byPhone = await api.post<Contact>(contactPath(`phone:${phone}`, "/upsert"), { firstName: "Phoney" })
    expect(byPhone.status, byPhone.text).toBe(200)
    expect(byPhone.json).toMatchObject({ phoneNumber: phone, email: null, firstName: "Phoney" })

    // A new email contact whose body phone is taken.
    expectError(
      await api.post(contactPath(`email:other-${email}`, "/upsert"), { phoneNumber: phone }),
      422,
      "phoneExists",
      { message: "Phone number already exists" },
    )
    // Unknown id: cannot create.
    expectError(await api.post(contactPath("id:1", "/upsert"), { firstName: "x" }), 404, "notFound", {
      message: "Contact not found",
    })
    // Bad identifier prefix.
    expectError(await api.post(contactPath("name:x", "/upsert"), {}), 404, "notFound")
    // Body validation.
    expectInvalid(await api.post(contactPath(`email:${email}`, "/upsert"), { email: "bad" }), "email")
    expectInvalid(await api.post(contactPath(`email:${email}`, "/upsert"), { phoneNumber: "123" }), "phoneNumber")
    expectInvalid(await api.post(contactPath(`email:${email}`, "/upsert"), { gender: "x" }), "gender")
    expectInvalid(await api.post(contactPath(`email:${email}`, "/upsert"), { firstName: "x".repeat(101) }), "firstName")
  })
})
