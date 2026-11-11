// contacts.listInboxes / listAdReferrals / refreshProfile.

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { expectNormalized, Normalizer } from "../../src/normalize.ts"
import {
  type Api,
  at,
  contactPath,
  createWebchat,
  createWebchatContact,
  expectError,
  expectInvalid,
  ext,
  idOf,
  newTenant,
  qs,
  sortBy,
} from "./helpers.ts"

let tenant: Tenant
let api: Api

beforeAll(async () => {
  ;({ tenant, api } = await newTenant("p106-inbox"))
})

describe("listInboxes", () => {
  it("lists each channel identity of the contact with its inbox name", async () => {
    const n = new Normalizer()
    const fx = await ext()
    const messenger = (await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "messenger", name: "Page A" })).inboxId
    const telegram = (await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "telegram", name: "Bot B" })).inboxId
    const created = await fx.createContact({
      workspaceId: tenant.workspaceId,
      firstName: "Two",
      createdAt: at("2025-04-01T00:00:00Z"),
      inboxes: [
        { inboxId: messenger, channel: "messenger", sourceId: "psid-two", language: "en", lastIncomingMessageAt: at("2025-04-02T00:00:00Z") },
        { inboxId: telegram, channel: "telegram", sourceId: "tg-two", source: "ads" },
      ],
    })
    const response = await api.get<{ data: Array<{ id: string; inbox: { name: string } }> }>(
      contactPath(idOf(created.contactId), "/inboxes"),
    )
    expect(response.status, response.text).toBe(200)
    const rows = sortBy(response.json.data, (row) => row.inbox.name)
    expect(rows).toEqual([
      {
        id: created.contactInboxIds[1],
        contactId: created.contactId,
        inboxId: telegram,
        channel: "telegram",
        source: "ads",
        sourceId: "tg-two",
        sourceUserId: null,
        sourceUsername: null,
        language: null,
        lastIncomingMessageAt: null,
        contactLastReadAt: null,
        inbox: { name: "Bot B" },
      },
      {
        id: created.contactInboxIds[0],
        contactId: created.contactId,
        inboxId: messenger,
        channel: "messenger",
        source: "direct",
        sourceId: "psid-two",
        sourceUserId: null,
        sourceUsername: null,
        language: "en",
        lastIncomingMessageAt: "2025-04-02T00:00:00.000Z",
        contactLastReadAt: null,
        inbox: { name: "Page A" },
      },
    ])
    expectNormalized(n, { data: rows }, { checkTimestamps: false }).toMatchSnapshot()
  })

  it("a contact without inboxes has none; unknown contact 404", async () => {
    const fx = await ext()
    const lonely = await fx.createContact({ workspaceId: tenant.workspaceId, firstName: "Lonely" })
    expect((await api.get(contactPath(idOf(lonely.contactId), "/inboxes"))).json).toEqual({ data: [] })
    expectError(await api.get(contactPath("id:1", "/inboxes")), 404, "notFound")
  })
})

describe("listAdReferrals", () => {
  it("lists ad clicks newest first with cursor pages", async () => {
    const n = new Normalizer()
    const fx = await ext()
    const { inboxId } = await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "messenger", name: "Ads page" })
    const contact = await fx.createContact({ workspaceId: tenant.workspaceId, firstName: "Clicker", inboxes: [{ inboxId, channel: "messenger" }] })
    const contactInboxId = contact.contactInboxIds[0] as string
    const refs: string[] = []
    for (const [day, adId, title] of [
      ["2025-06-01", "120000000000000001", "Spring sale"],
      ["2025-06-03", "120000000000000003", null],
      ["2025-06-02", "120000000000000002", "Summer sale"],
    ] as const) {
      const { id } = await fx.createAdReferral({
        workspaceId: tenant.workspaceId,
        contactId: contact.contactId,
        contactInboxId,
        channel: "messenger",
        provider: "meta",
        occurredAt: at(`${day}T08:00:00Z`),
        adId,
        campaignId: "camp-1",
        adGroupId: null,
        source: "ADS",
        sourceUrl: "https://fb.me/ad",
        ...(title ? { title } : {}),
      })
      refs.push(id)
    }
    const page = (query: Record<string, string | number | undefined>) =>
      api.get<{ data: Array<Record<string, unknown>>; nextCursor: string | null }>(
        `${contactPath(idOf(contact.contactId), "/ad-referrals")}${qs(query)}`,
      )
    const all = await page({})
    expect(all.status, all.text).toBe(200)
    expect(all.json.data.map((row) => row.adId)).toEqual(["120000000000000003", "120000000000000002", "120000000000000001"])
    expect(all.json.nextCursor).toBeNull()
    expect(all.json.data[0]).toEqual({
      id: refs[1],
      adId: "120000000000000003",
      title: null,
      channel: "messenger",
      provider: "meta",
      campaignId: "camp-1",
      adGroupId: null,
      source: "ADS",
      sourceUrl: "https://fb.me/ad",
      occurredAt: "2025-06-03T08:00:00.000Z",
    })
    expectNormalized(n, all.json, { checkTimestamps: false }).toMatchSnapshot()

    const first = await page({ perPage: 2 })
    expect(first.json.data.map((row) => row.title)).toEqual([null, "Summer sale"])
    expect(typeof first.json.nextCursor).toBe("string")
    const second = await page({ perPage: 2, cursor: first.json.nextCursor as string })
    expect(second.json.data.map((row) => row.title)).toEqual(["Spring sale"])
    expect(second.json.nextCursor).toBeNull()

    expectError(await page({ cursor: "not-a-cursor" }), 400, "invalidCursor", {
      message: "The cursor is invalid. Start again without a cursor.",
    })
    expectInvalid(await page({ perPage: 0 }), "perPage")
    expectInvalid(await page({ perPage: 101 }), "perPage")
  })

  it("no clicks: empty; unknown contact 404", async () => {
    const fx = await ext()
    const plain = await fx.createContact({ workspaceId: tenant.workspaceId, firstName: "Organic" })
    expect((await api.get(contactPath(idOf(plain.contactId), "/ad-referrals"))).json).toEqual({ data: [], nextCursor: null })
    expectError(await api.get(contactPath("id:1", "/ad-referrals")), 404, "notFound")
  })
})

describe("refreshProfile", () => {
  const refresh = (contactId: string, contactInboxId: unknown) =>
    api.post(contactPath(idOf(contactId), "/refresh-profile"), { contactInboxId })

  it("webchat cannot look profiles up: skipped / channelNotCapable", async () => {
    const webchat = await createWebchat(api)
    const contactId = await createWebchatContact(api, webchat.inboxId, { firstName: "Web" })
    const inboxes = await api.get<{ data: Array<{ id: string }> }>(contactPath(idOf(contactId), "/inboxes"))
    const response = await refresh(contactId, inboxes.json.data[0]?.id)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ status: "skipped", reason: "channelNotCapable" })
  })

  it("messenger with a name: skipped / profileComplete; without a name and no integration: failed, then coolingDown", async () => {
    const fx = await ext()
    const { inboxId } = await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "messenger", name: "No integration" })
    const named = await fx.createContact({ workspaceId: tenant.workspaceId, firstName: "Named", inboxes: [{ inboxId, channel: "messenger" }] })
    expect((await refresh(named.contactId, named.contactInboxIds[0])).json).toEqual({ status: "skipped", reason: "profileComplete" })

    const unnamed = await fx.createContact({ workspaceId: tenant.workspaceId, inboxes: [{ inboxId, channel: "messenger" }] })
    const failed = await refresh(unnamed.contactId, unnamed.contactInboxIds[0])
    expect(failed.status, failed.text).toBe(200)
    expect(failed.json).toEqual({ status: "failed" })
    expect((await refresh(unnamed.contactId, unnamed.contactInboxIds[0])).json).toEqual({ status: "skipped", reason: "coolingDown" })
  })

  it("errors: another contact's inbox, unknown inbox, bad input, unknown contact", async () => {
    const fx = await ext()
    const { inboxId } = await fx.createInbox({ workspaceId: tenant.workspaceId, channel: "messenger", name: "Errors" })
    const a = await fx.createContact({ workspaceId: tenant.workspaceId, firstName: "A", inboxes: [{ inboxId, channel: "messenger" }] })
    const b = await fx.createContact({ workspaceId: tenant.workspaceId, firstName: "B", inboxes: [{ inboxId, channel: "messenger" }] })
    expectError(await refresh(a.contactId, b.contactInboxIds[0]), 404, "notFound", { message: "Contact inbox not found" })
    expectError(await refresh(a.contactId, "1"), 404, "notFound", { message: "Contact inbox not found" })
    expectInvalid(await refresh(a.contactId, undefined), "contactInboxId")
    expectInvalid(await refresh(a.contactId, "abc"), "contactInboxId")
    expectError(await refresh("1", a.contactInboxIds[0]), 404, "notFound", { message: "Contact not found" })
  })
})
