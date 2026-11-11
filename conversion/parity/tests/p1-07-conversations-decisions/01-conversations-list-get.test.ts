// conversations.get and conversations.list (scope `inbox`): the full and the
// compact item shape, the order (lastActivityAt desc, NULL last, id desc),
// cursor paging, every filter (tags, botCategory, botEnabled, assignedId,
// channel, keyword, contactFilter) and the input errors. Rows are seeded
// with the P1.7 fixture extension (contact + contact inbox + conversation);
// the message preview comes from `contacts.sendMessage` on a webchat contact.

import { beforeAll, describe, expect, it } from "vitest"
import type { Tenant } from "../../src/fixtures/index.ts"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  type Api,
  contactPath,
  createWebchat,
  createWebchatContact,
  expectError,
  expectInvalid,
  ext,
  idOf,
  newTenant,
  UNKNOWN_ID,
} from "./helpers.ts"
import {
  ago,
  COMPACT_CONTACT_INBOX_KEYS,
  CONTACT_INBOX_KEYS,
  CONTACT_KEYS,
  CONVERSATION_KEYS,
  type ConversationItem,
  type ConversationPage,
  DAY,
  getConversation,
  HOUR,
  idsOf,
  keysOf,
  listConversations,
  seedConversation,
} from "./helpers-conversations.ts"

describe("conversations.get", () => {
  it("returns the full conversation: every column, the contact, contact inboxes with routing and ad data", async () => {
    const n = new Normalizer()
    const { tenant, api } = await newTenant("p107-conv-get")
    const lastActivityAt = ago(2 * HOUR)
    const referralAt = ago(3 * HOUR)
    const seeded = await seedConversation(tenant.workspaceId, {
      channel: "telegram",
      firstName: "Getty",
      lastName: "Shape",
      email: "getty@parity.test",
      phoneNumber: "+12025550111",
      gender: "female",
      conversation: { lastActivityAt, agentLastReadAt: lastActivityAt, followed: true, botEnabled: false },
      contactInbox: {
        sourceId: "tg-getty-1",
        language: "en",
        lastMessageAt: lastActivityAt,
        lastIncomingMessageAt: lastActivityAt,
        threadControlState: "owned",
        threadOwnerRole: "escalation",
        threadControlUpdatedAt: lastActivityAt,
        referral: {
          source: "ADS",
          adId: "ad-777",
          adTitle: "Spring sale",
          sourceUrl: "https://ads.example/spring",
          gclid: "gclid-secret-value",
          googleClickReceivedAt: referralAt,
        },
      },
    })

    const item = await getConversation(api, seeded.conversationId)
    expect(keysOf(item)).toEqual(CONVERSATION_KEYS)
    expect(item).toMatchObject({
      id: seeded.conversationId,
      contactId: seeded.contactId,
      workspaceId: tenant.workspaceId,
      archivedAt: null,
      followed: true,
      botEnabled: false,
      botResumeAt: null,
      lastActivityAt: new Date(lastActivityAt).toISOString(),
      agentLastReadAt: new Date(lastActivityAt).toISOString(),
      contactLastReadAt: null,
      assignedUserId: null,
      assignedInboxTeamId: null,
      assignedUserAt: null,
      assignedUser: null,
      assignedInboxTeam: null,
      additionalAttributes: null,
      aiContextLastMessageId: null,
      sourceId: null,
      lastStep: null,
      currentStep: null,
      adminRepliedAt: null,
      contactRepliedAt: null,
      awaitingHumanReplySince: null,
      responseDueAt: null,
      responseSlaBreachingAt: null,
      responseSlaTargetSeconds: null,
      messages: [],
    })
    expect(n.normalize({ createdAt: item.createdAt, updatedAt: item.updatedAt })).toEqual({
      createdAt: "<ts>",
      updatedAt: "<ts>",
    })

    expect(keysOf(item.contact)).toEqual(CONTACT_KEYS)
    expect(item.contact).toMatchObject({
      id: seeded.contactId,
      workspaceId: tenant.workspaceId,
      firstName: "Getty",
      lastName: "Shape",
      fullName: "Getty Shape",
      email: "getty@parity.test",
      phoneNumber: "+12025550111",
      gender: "female",
      avatar: null,
      blockedAt: null,
      emailVerified: false,
      emailOptIn: true,
    })

    expect(item.contactInboxes).toHaveLength(1)
    const [inbox] = item.contactInboxes
    expect(keysOf(inbox)).toEqual(CONTACT_INBOX_KEYS)
    expect(inbox).toEqual({
      id: seeded.contactInboxId,
      contactId: seeded.contactId,
      inboxId: seeded.inboxId,
      channel: "telegram",
      source: "direct",
      sourceId: "tg-getty-1",
      sourceUserId: null,
      sourceUsername: null,
      language: "en",
      lastMessageAt: new Date(lastActivityAt).toISOString(),
      lastIncomingMessageAt: new Date(lastActivityAt).toISOString(),
      contactLastReadAt: null,
      // Only the inbox name rides along in the full view.
      inbox: { name: expect.stringMatching(/^Parity telegram /) },
      // Derived from the raw referral; the referral itself and the gclid value never leave the server.
      adReferral: { adTitle: "Spring sale", sourceUrl: "https://ads.example/spring" },
      googleAdsClick: { clickIdType: "gclid", receivedAt: referralAt },
      threadControlState: "owned",
      threadOwnerRole: "escalation",
      threadOwnerAppId: null,
      threadControlUpdatedAt: new Date(lastActivityAt).toISOString(),
      threadOwnerExpiresAt: null,
      threadControlLastEvent: null,
    })
    expect(JSON.stringify(item)).not.toContain("gclid-secret-value")
  })

  it("a referral that is not a paid ad gives adReferral null; no referral gives both null", async () => {
    const { tenant, api } = await newTenant("p107-conv-referral")
    const organic = await seedConversation(tenant.workspaceId, {
      contactInbox: { referral: { source: "SHORTLINK", ref: "promo", adId: "ad-1" } },
    })
    const none = await seedConversation(tenant.workspaceId)
    const ctwa = await seedConversation(tenant.workspaceId, {
      contactInbox: { referral: { ctwaClid: "ctwa-1", gbraid: "gb-1" } },
    })
    const [o, x, c] = await Promise.all(
      [organic, none, ctwa].map(async (s) => (await getConversation(api, s.conversationId)).contactInboxes[0]),
    )
    expect(o).toMatchObject({ adReferral: null, googleAdsClick: null })
    expect(x).toMatchObject({ adReferral: null, googleAdsClick: null })
    expect(c).toMatchObject({
      adReferral: { adTitle: null, sourceUrl: null },
      googleAdsClick: { clickIdType: "gbraid", receivedAt: null },
    })
  })

  it("shows the last incoming/outgoing message as `messages[0]`", async () => {
    const { api } = await newTenant("p107-conv-preview")
    const webchat = await createWebchat(api)
    const name = `Preview${uniqueId()}`
    const contactId = await createWebchatContact(api, webchat.inboxId, { firstName: name })
    const page = await listConversations(api, { keyword: name })
    expect(idsOf(page)).toHaveLength(1)
    const conversationId = (page.json.data[0] as ConversationItem).id
    expect((page.json.data[0] as ConversationItem).messages).toEqual([])

    for (const text of ["first reply", "second reply"]) {
      const sent = await api.post(contactPath(idOf(contactId), "/messages"), { text })
      expect(sent.status, sent.text).toBe(204)
    }
    const item = await getConversation(api, conversationId)
    expect(item.messages).toHaveLength(1)
    expect(item.messages[0]).toMatchObject({
      text: "second reply",
      messageType: "outgoing",
      senderType: "api",
      conversationId,
      workspaceId: item.workspaceId,
    })
    const listed = await listConversations(api, { keyword: name })
    expect((listed.json.data[0] as ConversationItem).messages).toEqual([item.messages[0]])
  })

  it("404 notFound for an unknown id and for a conversation of another workspace", async () => {
    const { api } = await newTenant("p107-conv-get-404")
    const other = await newTenant("p107-conv-get-foreign")
    const foreign = await seedConversation(other.tenant.workspaceId)
    for (const id of [UNKNOWN_ID, foreign.conversationId]) {
      expectError(await api.get(`/v1/conversations/${id}`), 404, "notFound", { message: "Conversation not found" })
    }
    // The owner still sees it.
    expect((await getConversation(other.api, foreign.conversationId)).id).toBe(foreign.conversationId)
  })

  it("422 for a non-numeric id", async () => {
    const { api } = await newTenant("p107-conv-get-422")
    expectInvalid(await api.get("/v1/conversations/abc"), "id")
  })
})

describe("conversations.list item shapes", () => {
  it("view full (default) lists the same item as get", async () => {
    const { tenant, api } = await newTenant("p107-conv-list-full")
    const seeded = await seedConversation(tenant.workspaceId, {
      firstName: "Listed",
      conversation: { lastActivityAt: ago(HOUR) },
      contactInbox: { threadControlState: "standby", threadOwnerAppId: "app-x" },
    })
    const page = await listConversations(api)
    expect(page.status, page.text).toBe(200)
    expect(keysOf(page.json)).toEqual(["data", "nextCursor", "prevCursor"])
    expect(page.json).toMatchObject({ nextCursor: null, prevCursor: null })
    expect(page.json.data).toHaveLength(1)
    const [item] = page.json.data as [ConversationItem]
    expect(keysOf(item)).toEqual(CONVERSATION_KEYS)
    expect(item).toEqual(await getConversation(api, seeded.conversationId))
    const full = await listConversations(api, { view: "full" })
    expect(full.json).toEqual(page.json)
  })

  it("view compact drops routing and ad data and carries the inbox id, name and channel", async () => {
    const { tenant, api } = await newTenant("p107-conv-list-compact")
    const seeded = await seedConversation(tenant.workspaceId, {
      channel: "messenger",
      firstName: "Compact",
      contactInbox: {
        sourceId: "psid-compact",
        threadControlState: "owned",
        referral: { source: "ADS", adId: "ad-1", adTitle: "T" },
      },
    })
    const full = (await listConversations(api)).json.data[0] as ConversationItem
    const page = await listConversations(api, { view: "compact" })
    expect(page.status, page.text).toBe(200)
    const [item] = page.json.data as [ConversationItem]
    expect(keysOf(item)).toEqual(CONVERSATION_KEYS)
    const [inbox] = item.contactInboxes
    expect(keysOf(inbox)).toEqual(COMPACT_CONTACT_INBOX_KEYS)
    expect(inbox).toEqual({
      id: seeded.contactInboxId,
      contactId: seeded.contactId,
      inboxId: seeded.inboxId,
      channel: "messenger",
      source: "direct",
      sourceId: "psid-compact",
      sourceUserId: null,
      sourceUsername: null,
      language: null,
      lastMessageAt: null,
      lastIncomingMessageAt: null,
      contactLastReadAt: null,
      inbox: { id: seeded.inboxId, name: expect.stringMatching(/^Parity messenger /), channel: "messenger" },
    })
    // Everything but the contact inboxes (and the avatar proxy token, which is
    // signed with a random IV per read) matches the full view.
    const strip = (value: ConversationItem) => ({
      ...value,
      contactInboxes: undefined,
      contact: { ...value.contact, avatar: undefined },
    })
    expect(strip(item)).toEqual(strip(full))
    // A messenger contact without a stored avatar gets the media proxy URL.
    expect(item.contact?.avatar).toMatch(/^https?:\/\/[^/]+\/media\/avatar\/[A-Za-z0-9_-]+$/)
  })

  it("422 for an unknown view", async () => {
    const { api } = await newTenant("p107-conv-list-view")
    expectInvalid(await listConversations(api, { view: "tiny" }), "view")
  })
})

describe("conversations.list order and cursor paging", () => {
  it("orders by lastActivityAt desc (NULL last), then id desc, and pages with nextCursor / prevCursor", async () => {
    const { tenant, api } = await newTenant("p107-conv-order")
    const w = tenant.workspaceId
    const same = ago(3 * HOUR)
    const a = await seedConversation(w, { firstName: "A", conversation: { lastActivityAt: ago(5 * HOUR) } })
    const b = await seedConversation(w, { firstName: "B", conversation: { lastActivityAt: null } })
    const c = await seedConversation(w, { firstName: "C", conversation: { lastActivityAt: same } })
    const d = await seedConversation(w, { firstName: "D", conversation: { lastActivityAt: same } })
    const e = await seedConversation(w, { firstName: "E", conversation: { lastActivityAt: ago(HOUR) } })
    const f = await seedConversation(w, { firstName: "F", conversation: { lastActivityAt: null } })
    const expected = [e, d, c, a, f, b].map((s) => s.conversationId)

    expect(idsOf(await listConversations(api))).toEqual(expected)

    const seen: string[] = []
    let cursor: string | undefined
    const pages: ConversationPage[] = []
    for (let i = 0; i < 4; i += 1) {
      const page = await listConversations(api, { perPage: 2, ...(cursor ? { cursor } : {}) })
      expect(page.status, page.text).toBe(200)
      pages.push(page.json)
      seen.push(...page.json.data.map((item) => item.id))
      if (!page.json.nextCursor) {
        break
      }
      cursor = page.json.nextCursor
    }
    expect(seen).toEqual(expected)
    expect(pages.map((p) => p.data.length)).toEqual([2, 2, 2])
    expect(pages.map((p) => p.nextCursor === null)).toEqual([false, false, true])
    expect(pages.map((p) => p.prevCursor === null)).toEqual([true, false, false])
    for (const value of pages.flatMap((p) => [p.nextCursor, p.prevCursor])) {
      if (value !== null) {
        expect(typeof value).toBe("string")
      }
    }
  })

  it("422 for perPage 0 or a non-integer perPage", async () => {
    const { api } = await newTenant("p107-conv-perpage")
    expectInvalid(await listConversations(api, { perPage: 0 }), "perPage")
    expectInvalid(await listConversations(api, { perPage: "x" }), "perPage")
  })
})

describe("conversations.list filters", () => {
  let tenant: Tenant
  let api: Api
  const ids: Record<string, string> = {}
  const extra: Record<string, string> = {}

  beforeAll(async () => {
    ;({ tenant, api } = await newTenant("p107-conv-filters"))
    const w = tenant.workspaceId
    const fx = await ext()
    const { inboxTeamId } = await fx.createInboxTeam({ workspaceId: w, name: "Parity team" })
    extra.teamId = inboxTeamId
    const { inboxId: telegram } = await fx.createInbox({ workspaceId: w, channel: "telegram", name: "Parity telegram" })
    const { inboxId: messenger } = await fx.createInbox({ workspaceId: w, channel: "messenger", name: "Parity messenger" })
    const read = ago(0)
    const seed = async (
      key: string,
      hoursAgo: number | null,
      input: Parameters<typeof seedConversation>[1] = {},
    ) => {
      const { conversation, ...rest } = input
      const created = await seedConversation(w, {
        channel: "telegram",
        inboxId: telegram,
        firstName: `F${key}`,
        lastName: "Filter",
        ...rest,
        conversation: {
          lastActivityAt: hoursAgo === null ? null : ago(hoursAgo * HOUR),
          agentLastReadAt: read,
          ...(conversation ?? {}),
        },
      })
      ids[key] = created.conversationId
      return created
    }
    await seed("unread", 2, { conversation: { agentLastReadAt: null } })
    await seed("noAdminReply", 3, { conversation: { contactRepliedAt: ago(3 * HOUR), adminRepliedAt: ago(4 * HOUR) } })
    await seed("plain", 5)
    await seed("archived", 6, { conversation: { archivedAt: ago(HOUR) } })
    await seed("blocked", 7, { blockedAt: ago(DAY) })
    await seed("followed", 8, { conversation: { followed: true } })
    await seed("human", 9, { conversation: { botEnabled: false } })
    await seed("assignedUser", 10, { conversation: { assignedUserId: tenant.userId } })
    await seed("assignedTeam", 11, { conversation: { assignedInboxTeamId: inboxTeamId } })
    await seed("slaBreached", 12, { conversation: { responseDueAt: ago(HOUR) } })
    await seed("slaBreaching", 13, {
      conversation: { responseSlaBreachingAt: ago(HOUR), responseDueAt: new Date(Date.now() + HOUR).toISOString() },
    })
    extra.psid = `psid-${uniqueId()}`
    extra.email = `zeta-${uniqueId()}@parity.test`
    await seed("messenger", 14, {
      channel: "messenger",
      inboxId: messenger,
      firstName: "Zetamess",
      email: extra.email,
      gender: "female",
      contactInbox: { sourceId: extra.psid },
    })
    await seed("noActivity", null)
  })

  const DEFAULT = [
    "unread",
    "noAdminReply",
    "plain",
    "followed",
    "human",
    "assignedUser",
    "assignedTeam",
    "slaBreached",
    "slaBreaching",
    "messenger",
    "noActivity",
  ]
  const without = (...keys: string[]) => DEFAULT.filter((key) => !keys.includes(key))
  const expectKeys = async (params: Parameters<typeof listConversations>[1], keys: string[]) => {
    const response = await listConversations(api, params)
    expect(idsOf(response), JSON.stringify(params)).toEqual(keys.map((key) => ids[key]))
  }

  it("default: archived conversations and blocked contacts are hidden", async () => {
    await expectKeys({}, DEFAULT)
  })

  it.each([
    ["archived", ["archived"]],
    ["blocked", ["blocked"]],
    ["followUp", ["followed"]],
    ["unread", ["unread"]],
    ["noAdminReply", ["noAdminReply"]],
    ["slaBreached", ["slaBreached"]],
    ["slaBreaching", ["slaBreaching"]],
  ])("tags [%s]", async (tag, keys) => {
    await expectKeys({ tags: [tag] }, keys)
  })

  it("tags are AND-ed; archived + blocked shows archived blocked contacts only", async () => {
    await expectKeys({ tags: ["followUp", "unread"] }, [])
    await expectKeys({ tags: ["archived", "blocked"] }, [])
  })

  it("an unparsable or bracket-encoded tags value is ignored (no filter)", async () => {
    await expectKeys({ tags: "not-json" }, DEFAULT)
    const bracket = await api.get<ConversationPage>("/v1/conversations?tags[0]=archived")
    expect(idsOf(bracket)).toEqual(DEFAULT.map((key) => ids[key]))
  })

  it("422 for an unknown tag value inside the JSON array", async () => {
    expectInvalid(await listConversations(api, { tags: ["bogus"] }), "tags.0")
  })

  it("`status` is accepted (and validated) but does not filter", async () => {
    await expectKeys({ status: ["archived"] }, DEFAULT)
    await expectKeys({ status: ["unread"] }, DEFAULT)
    expectInvalid(await listConversations(api, { status: ["bogus"] }), "status.0")
  })

  it("botCategory and botEnabled", async () => {
    await expectKeys({ botCategory: "human" }, ["human"])
    await expectKeys({ botCategory: "bot" }, without("human"))
    await expectKeys({ botCategory: "all" }, DEFAULT)
    await expectKeys({ botEnabled: "false" }, ["human"])
    await expectKeys({ botEnabled: "true" }, without("human"))
    // botEnabled overrides botCategory.
    await expectKeys({ botCategory: "human", botEnabled: "true" }, without("human"))
    expectInvalid(await listConversations(api, { botCategory: "robot" }), "botCategory")
    expectInvalid(await listConversations(api, { botEnabled: "yes" }), "botEnabled")
  })

  it("assignedId u_<user>, t_<team>, unassigned; another value does not filter", async () => {
    await expectKeys({ assignedId: `u_${tenant.userId}` }, ["assignedUser"])
    await expectKeys({ assignedId: `t_${extra.teamId}` }, ["assignedTeam"])
    await expectKeys({ assignedId: "unassigned" }, without("assignedUser", "assignedTeam"))
    await expectKeys({ assignedId: "u_1" }, [])
    await expectKeys({ assignedId: "someone" }, DEFAULT)
  })

  it("channel (omnichannel = no restriction)", async () => {
    await expectKeys({ channel: "messenger" }, ["messenger"])
    await expectKeys({ channel: "telegram" }, without("messenger"))
    await expectKeys({ channel: "omnichannel" }, DEFAULT)
    await expectKeys({ channel: "whatsapp" }, [])
    expectInvalid(await listConversations(api, { channel: "pigeon" }), "channel")
  })

  it("keyword: name substring (case-insensitive), email, exact channel source id", async () => {
    await expectKeys({ keyword: "ZETAM" }, ["messenger"])
    await expectKeys({ keyword: extra.email }, ["messenger"])
    await expectKeys({ keyword: extra.psid }, ["messenger"])
    await expectKeys({ keyword: (extra.psid as string).slice(0, -2) }, [])
    await expectKeys({ keyword: "Fplain" }, ["plain"])
    // A keyword does not lift the archived / blocked defaults.
    await expectKeys({ keyword: "Farchived" }, [])
  })

  it("contactFilter (JSON criteria); a `blocked` condition lifts the blocked default", async () => {
    const cond = (field: string, operator: string, value?: unknown) => ({
      operator: "and",
      conditions: [{ field, operator, ...(value === undefined ? {} : { value }) }],
    })
    await expectKeys({ contactFilter: cond("gender", "eq", "female") }, ["messenger"])
    await expectKeys({ contactFilter: cond("blocked", "eq", "true") }, ["blocked"])
    await expectKeys({ contactFilter: cond("blocked", "eq", "false") }, DEFAULT)
    await expectKeys({ contactFilter: "not-json" }, DEFAULT)
    expectError(
      await listConversations(api, { contactFilter: { operator: "xor", conditions: [] } }),
      422,
      "invalidRequestData",
    )
  })
})
