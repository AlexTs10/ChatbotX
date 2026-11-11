// What the worker does with an accepted delivery: resolve the inbox, find or
// create the contact, write the mapped fields, run the actions (tags, custom
// fields, start a published flow, hand over to a human), and record the
// outcome. Observed through the public API (contacts, conversations, request
// log) and the fake upstream (the API channel's signed callback).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { uniqueId } from "../../src/unique.ts"
import {
  activeWebhook,
  configure,
  contactFieldValues,
  contactTagNames,
  createApiChannel,
  createCustomField,
  createFlow,
  createTag,
  createWebchat,
  createWebchatContact,
  deliverAccepted,
  expectError,
  expectInvalid,
  expectIso,
  getContact,
  REQUEST_DETAIL_KEYS,
  REQUEST_SUMMARY_KEYS,
  tenantApi,
  uniqueName,
  type WebhookConfig,
  type WebhookRequest,
  waitForRequest,
} from "./helpers-webhooks.ts"

const BASE = "/v1/inbound-webhooks"

describe("delivery processing: API channel contact, mapped fields and actions", () => {
  it(
    "creates the contact, writes the fields, runs the actions and sends the published flow to the channel callback",
    async () => {
      const start = Date.now()
      const { api } = await tenantApi("p109-proc-flow")
      const channel = await createApiChannel(api, { callback: true })
      const tagName = uniqueName("vip")
      const tagId = await createTag(api, tagName)
      const tierName = uniqueName("tier")
      const tier = await createCustomField(api, tierName, "shortText")
      const scoreName = uniqueName("score")
      const score = await createCustomField(api, scoreName, "number")
      const text = `Welcome from the webhook ${uniqueId()}`
      const flowId = await createFlow(api, { text })
      const webhook = await activeWebhook(api, {
        inboxResolution: { mode: "inbox", inboxId: channel.inboxId },
        identifiers: [
          { kind: "channelUserId", jsonPath: "$.contact.id" },
          { kind: "fullName", jsonPath: "$.contact.name" },
        ],
        fieldMappings: [
          { jsonPath: "$.contact.email", target: "email" },
          { jsonPath: "$.contact.first", target: "first_name" },
          { jsonPath: "$.order.tier", target: tier },
          { jsonPath: "$.order.gender", target: "gender" },
        ],
        actions: [
          { type: "addTag", tagIds: [tagId] },
          { type: "setCustomField", customFieldId: score, operation: "O01", value: "7" },
          { type: "startAnotherFlow", flowId },
        ],
      })
      const sourceId = `ext-${uniqueId()}`
      const since = await fake.cursor()
      const payload = {
        contact: { id: sourceId, name: "Augusta Lovelace", first: "Ada", email: "ada@parity.test" },
        order: { tier: "gold", gender: "Female" },
      }
      const requestId = await deliverAccepted(webhook.endpointUrl as string, payload)

      const request = await waitForRequest(api, webhook.id, requestId, { includePayload: true })
      expect(Object.keys(request).sort()).toEqual([...REQUEST_DETAIL_KEYS, "payload"].sort())
      expect(request).toEqual({
        id: requestId,
        status: "processed",
        skipReason: null,
        errorCode: null,
        errorMessage: null,
        inboxId: channel.inboxId,
        contactId: expect.stringMatching(/^\d+$/),
        attempts: 1,
        receivedAt: expect.any(String),
        processedAt: expect.any(String),
        fieldErrors: null,
        actionResults: [
          { index: 0, type: "addTag", status: "succeeded" },
          { index: 1, type: "setCustomField", status: "succeeded" },
          { index: 2, type: "startAnotherFlow", status: "succeeded" },
        ],
        payload,
      })
      expectIso(request.receivedAt, start)
      expectIso(request.processedAt, start)
      expect(Date.parse(request.processedAt as string)).toBeGreaterThanOrEqual(Date.parse(request.receivedAt))
      const contactId = request.contactId as string

      // The contact: name from the full name with the mapped first name laid over it.
      const contact = await getContact(api, contactId)
      expect(contact).toMatchObject({
        id: contactId,
        firstName: "Ada",
        lastName: "Lovelace",
        fullName: "Ada Lovelace",
        email: "ada@parity.test",
        gender: "female",
      })
      expect(contact.contactInboxes).toEqual([
        expect.objectContaining({ inboxId: channel.inboxId, channel: "api", source: "inboundWebhook", sourceId }),
      ])
      expect(await contactTagNames(api, contactId)).toEqual([tagName])
      expect(await contactFieldValues(api, contactId)).toEqual({ [tierName]: "gold", [scoreName]: "7" })

      // The flow's text step reaches the API channel's callback, signed.
      const [callback] = await fake.waitFor(
        { kind: "http", host: channel.callbackHost as string, method: "POST", since },
        { timeoutMs: 45_000 },
      )
      expect(callback?.path).toBe("/hook")
      expect(callback?.headers["content-type"]).toMatch(/^application\/json/)
      expect(callback?.headers["x-chatbotx-signature"]).toMatch(/^sha256=[0-9a-f]{64}$/)
      expect(callback?.headers["x-chatbotx-timestamp"]).toMatch(/^\d{10}$/)
      expect(callback?.headers["x-chatbotx-delivery"]).toMatch(/^[0-9a-f-]{36}$/)
      const envelope = JSON.parse(callback?.bodyText ?? "{}") as Record<string, unknown>
      expect(envelope).toMatchObject({
        event: "message_created",
        contact: { sourceId },
        message: { text, messageType: "outgoing", contentType: "text" },
      })
      expectIso(envelope.timestamp, start)

      // The webhook counts the request.
      const summary = await api.get<{ data: Array<{ id: string; requestCount: number }> }>(BASE)
      expect(summary.json.data).toEqual([expect.objectContaining({ id: webhook.id, requestCount: 1 })])
    },
    120_000,
  )

  it(
    "finds the same contact again: updates mapped fields, removes tags, clears fields, hands over to a human",
    async () => {
      const { api } = await tenantApi("p109-proc-update")
      const channel = await createApiChannel(api)
      const tagName = uniqueName("lead")
      const tagId = await createTag(api, tagName)
      const fieldName = uniqueName("stage")
      const field = await createCustomField(api, fieldName, "shortText")
      const base: WebhookConfig = {
        inboxResolution: { mode: "inbox", inboxId: channel.inboxId },
        identifiers: [{ kind: "channelUserId", jsonPath: "$.id" }],
        fieldMappings: [
          { jsonPath: "$.email", target: "email" },
          { jsonPath: "$.stage", target: field },
        ],
        actions: [{ type: "addTag", tagIds: [tagId] }],
      }
      const webhook = await activeWebhook(api, base)
      const sourceId = `ext-${uniqueId()}`
      const first = await waitForRequest(
        api,
        webhook.id,
        await deliverAccepted(webhook.endpointUrl as string, { id: sourceId, email: "one@parity.test", stage: "new" }),
      )
      expect(first.status).toBe("processed")
      const contactId = first.contactId as string
      expect(await contactTagNames(api, contactId)).toEqual([tagName])
      const before = await getContact(api, contactId)
      // No name in the payload: the contact is created without one.
      expect(before).toMatchObject({ firstName: null, lastName: null, email: "one@parity.test" })
      const conversationId = before.conversation?.id as string
      const conversation = await api.get<{ data: { botEnabled: boolean } }>(`/v1/conversations/${conversationId}`)
      expect(conversation.json.data.botEnabled).toBe(true)

      await configure(api, webhook.id, {
        ...base,
        actions: [
          { type: "removeTag", tagIds: [tagId] },
          { type: "clearCustomField", customFieldId: field },
          { type: "transferConversationToHuman", notifyAdmins: false },
        ],
      })
      const second = await waitForRequest(
        api,
        webhook.id,
        await deliverAccepted(webhook.endpointUrl as string, { id: sourceId, email: "two@parity.test", stage: "won" }),
      )
      expect(second).toMatchObject({
        status: "processed",
        contactId,
        inboxId: channel.inboxId,
        fieldErrors: null,
        actionResults: [
          { index: 0, type: "removeTag", status: "succeeded" },
          { index: 1, type: "clearCustomField", status: "succeeded" },
          { index: 2, type: "transferConversationToHuman", status: "succeeded" },
        ],
      })
      const after = await getContact(api, contactId)
      expect(after.email).toBe("two@parity.test")
      expect(after.contactInboxes).toHaveLength(1)
      expect(await contactTagNames(api, contactId)).toEqual([])
      // The mapped value was written, then the action cleared it.
      expect(await contactFieldValues(api, contactId)).toEqual({})
      const handedOver = await api.get<{ data: { id: string; botEnabled: boolean } }>(`/v1/conversations/${conversationId}`)
      expect(handedOver.json.data).toMatchObject({ id: conversationId, botEnabled: false })
    },
    120_000,
  )

  it(
    "records rejected field values and skipped actions as completedWithErrors",
    async () => {
      const { api } = await tenantApi("p109-proc-errors")
      const channel = await createApiChannel(api)
      const countName = uniqueName("count")
      const count = await createCustomField(api, countName, "number")
      const draftFlow = await createFlow(api, { text: "never sent", publish: false })
      const webhook = await activeWebhook(api, {
        inboxResolution: { mode: "inbox", inboxId: channel.inboxId },
        identifiers: [{ kind: "channelUserId", jsonPath: "$.id" }],
        fieldMappings: [
          { jsonPath: "$.count", target: count },
          { jsonPath: "$.email", target: "email" },
          { jsonPath: "$.gender", target: "gender" },
          { jsonPath: "$.first", target: "first_name" },
        ],
        actions: [{ type: "startAnotherFlow", flowId: draftFlow }],
      })
      const request = await waitForRequest(
        api,
        webhook.id,
        await deliverAccepted(webhook.endpointUrl as string, {
          id: `ext-${uniqueId()}`,
          count: "many",
          email: "broken",
          gender: "robot",
          first: "Kept",
        }),
      )
      expect(request).toMatchObject({
        status: "completedWithErrors",
        skipReason: null,
        errorCode: null,
        fieldErrors: [
          { target: count, code: "invalidValue" },
          { target: "email", code: "invalidValue" },
          { target: "gender", code: "invalidValue" },
        ],
        actionResults: [{ index: 0, type: "startAnotherFlow", status: "skipped", reason: "flowInactive" }],
      })
      const contact = await getContact(api, request.contactId as string)
      expect(contact).toMatchObject({ firstName: "Kept", email: "", gender: "unknown" })
      expect(await contactFieldValues(api, request.contactId as string)).toEqual({})
    },
    90_000,
  )
})

describe("delivery processing: skip reasons", () => {
  it(
    "skips a payload without any usable contact identity (missingIdentifier)",
    async () => {
      const { api } = await tenantApi("p109-proc-missing-id")
      const channel = await createApiChannel(api)
      const webhook = await activeWebhook(api, {
        inboxResolution: { mode: "inbox", inboxId: channel.inboxId },
        identifiers: [{ kind: "channelUserId", jsonPath: "$.contact.id" }],
        fieldMappings: [{ jsonPath: "$.first", target: "first_name" }],
        actions: [],
      })
      const request = await waitForRequest(
        api,
        webhook.id,
        await deliverAccepted(webhook.endpointUrl as string, { first: "Nobody" }),
      )
      expect(request).toEqual({
        id: request.id,
        status: "skipped",
        skipReason: "missingIdentifier",
        errorCode: null,
        errorMessage: null,
        inboxId: channel.inboxId,
        contactId: null,
        attempts: 1,
        receivedAt: expect.any(String),
        processedAt: expect.any(String),
        fieldErrors: null,
        actionResults: null,
      })
    },
    90_000,
  )

  it(
    "webchat: creates from an email, finds by full name, skips an unknown or ambiguous name",
    async () => {
      const { api } = await tenantApi("p109-proc-webchat")
      const { inboxId } = await createWebchat(api)
      const twinA = await createWebchatContact(api, inboxId, { firstName: "Twin", lastName: "Parity" })
      const twinB = await createWebchatContact(api, inboxId, { firstName: "Twin", lastName: "Parity" })
      expect(twinA).not.toBe(twinB)
      const single = await createWebchatContact(api, inboxId, { firstName: "Solo", lastName: "Parity" })
      const webhook = await activeWebhook(api, {
        inboxResolution: { mode: "inbox", inboxId },
        identifiers: [
          { kind: "email", jsonPath: "$.email" },
          { kind: "fullName", jsonPath: "$.name" },
        ],
        fieldMappings: [{ jsonPath: "$.city", target: "last_name" }],
        actions: [],
      })
      const url = webhook.endpointUrl as string

      // A full name is not a creation identity on webchat.
      const unknown = await waitForRequest(api, webhook.id, await deliverAccepted(url, { name: "Nobody Known" }))
      expect(unknown).toMatchObject({ status: "skipped", skipReason: "contactNotFound", inboxId, contactId: null })

      // Whitespace runs are collapsed before the match; case is not folded.
      const ambiguous = await waitForRequest(api, webhook.id, await deliverAccepted(url, { name: "  Twin \t  Parity " }))
      expect(ambiguous).toMatchObject({ status: "skipped", skipReason: "contactAmbiguous", inboxId, contactId: null })
      const otherCase = await waitForRequest(api, webhook.id, await deliverAccepted(url, { name: "solo parity" }))
      expect(otherCase).toMatchObject({ status: "skipped", skipReason: "contactNotFound", contactId: null })

      const found = await waitForRequest(api, webhook.id, await deliverAccepted(url, { name: "Solo Parity", city: "Paris" }))
      expect(found).toMatchObject({ status: "processed", contactId: single, inboxId })
      expect(await getContact(api, single)).toMatchObject({ firstName: "Solo", lastName: "Paris" })

      const email = `new-${uniqueId()}@parity.test`
      const created = await waitForRequest(api, webhook.id, await deliverAccepted(url, { email, name: "Grace Hopper" }))
      expect(created).toMatchObject({ status: "processed", inboxId })
      expect([twinA, twinB, single]).not.toContain(created.contactId)
      const contact = await getContact(api, created.contactId as string)
      expect(contact).toMatchObject({ email, firstName: "Grace", lastName: "Hopper" })
      expect(contact.contactInboxes).toEqual([expect.objectContaining({ inboxId, channel: "webchat", source: "inboundWebhook" })])
      // The email finds that contact next time.
      const again = await waitForRequest(api, webhook.id, await deliverAccepted(url, { email, city: "Arlington" }))
      expect(again).toMatchObject({ status: "processed", contactId: created.contactId })
    },
    120_000,
  )

  it(
    "payload-resolved inbox: missingInboxIdentifier, inboxNotFound",
    async () => {
      const { api } = await tenantApi("p109-proc-payload-inbox")
      const webhook = await activeWebhook(api, {
        inboxResolution: { mode: "payload", channel: "messenger", identifierKind: "pageId", jsonPath: "$.page" },
        identifiers: [{ kind: "channelUserId", jsonPath: "$.psid" }],
        fieldMappings: [{ jsonPath: "$.name", target: "full_name" }],
        actions: [],
      })
      const url = webhook.endpointUrl as string
      const missing = await waitForRequest(api, webhook.id, await deliverAccepted(url, { psid: "1", name: "A" }))
      expect(missing).toMatchObject({ status: "skipped", skipReason: "missingInboxIdentifier", inboxId: null })
      const notFound = await waitForRequest(
        api,
        webhook.id,
        await deliverAccepted(url, { page: "123456789012345", psid: "1", name: "A" }),
      )
      expect(notFound).toMatchObject({ status: "skipped", skipReason: "inboxNotFound", inboxId: null, contactId: null })
    },
    90_000,
  )
})

describe("inboundWebhooks.listRequests / getRequest", () => {
  it(
    "lists deliveries newest first, filters by status, pages; the detail hides the payload by default",
    async () => {
      const start = Date.now()
      const { api } = await tenantApi("p109-proc-requests")
      const channel = await createApiChannel(api)
      const webhook = await activeWebhook(api, {
        inboxResolution: { mode: "inbox", inboxId: channel.inboxId },
        identifiers: [{ kind: "channelUserId", jsonPath: "$.id" }],
        fieldMappings: [{ jsonPath: "$.first", target: "first_name" }],
        actions: [],
      })
      const url = webhook.endpointUrl as string
      const empty = await api.get(`${BASE}/${webhook.id}/requests`)
      expect(empty.status, empty.text).toBe(200)
      expect(empty.json).toEqual({ data: [], pageCount: 0 })

      const processed = await deliverAccepted(url, { id: `ext-${uniqueId()}`, first: "One" })
      await waitForRequest(api, webhook.id, processed)
      const skipped = await deliverAccepted(url, { first: "No id" })
      await waitForRequest(api, webhook.id, skipped)
      const third = await deliverAccepted(url, { id: `ext-${uniqueId()}`, first: "Three" })
      await waitForRequest(api, webhook.id, third)

      const list = await api.get<{ data: WebhookRequest[]; pageCount: number }>(`${BASE}/${webhook.id}/requests`)
      expect(list.status, list.text).toBe(200)
      expect(list.json.pageCount).toBe(1)
      expect(list.json.data.map((request) => request.id)).toEqual([third, skipped, processed])
      for (const item of list.json.data) {
        expect(Object.keys(item).sort()).toEqual(REQUEST_SUMMARY_KEYS)
        expectIso(item.receivedAt, start)
      }
      expect(list.json.data[1]).toEqual({
        id: skipped,
        status: "skipped",
        skipReason: "missingIdentifier",
        errorCode: null,
        errorMessage: null,
        inboxId: channel.inboxId,
        contactId: null,
        attempts: 1,
        receivedAt: expect.any(String),
        processedAt: expect.any(String),
      })

      const onlySkipped = await api.get<{ data: WebhookRequest[]; pageCount: number }>(
        `${BASE}/${webhook.id}/requests?status=skipped`,
      )
      expect(onlySkipped.json).toEqual({ data: [list.json.data[1]], pageCount: 1 })
      const onlyProcessed = await api.get<{ data: WebhookRequest[] }>(`${BASE}/${webhook.id}/requests?status=processed`)
      expect(onlyProcessed.json.data.map((request) => request.id)).toEqual([third, processed])
      const none = await api.get(`${BASE}/${webhook.id}/requests?status=failed`)
      expect(none.json).toEqual({ data: [], pageCount: 0 })
      const paged = await api.get<{ data: WebhookRequest[]; pageCount: number }>(
        `${BASE}/${webhook.id}/requests?perPage=2&page=2`,
      )
      expect({ ids: paged.json.data.map((request) => request.id), pageCount: paged.json.pageCount }).toEqual({
        ids: [processed],
        pageCount: 2,
      })

      const detail = await api.get<WebhookRequest>(`${BASE}/${webhook.id}/requests/${processed}`)
      expect(detail.status, detail.text).toBe(200)
      expect(Object.keys(detail.json).sort()).toEqual(REQUEST_DETAIL_KEYS)
      expect(detail.json).toMatchObject({ id: processed, status: "processed", fieldErrors: null, actionResults: null })
      const explicitFalse = await api.get<WebhookRequest>(`${BASE}/${webhook.id}/requests/${processed}?includePayload=false`)
      expect(explicitFalse.json).toEqual(detail.json)
      const withPayload = await api.get<WebhookRequest>(`${BASE}/${webhook.id}/requests/${skipped}?includePayload=true`)
      expect(withPayload.json).toEqual({ ...list.json.data[1], fieldErrors: null, actionResults: null, payload: { first: "No id" } })
    },
    120_000,
  )

  it("404 for unknown or foreign webhooks and requests; 422 for bad parameters", async () => {
    const { api } = await tenantApi("p109-proc-requests-404")
    const channel = await createApiChannel(api)
    const config: WebhookConfig = {
      inboxResolution: { mode: "inbox", inboxId: channel.inboxId },
      identifiers: [{ kind: "channelUserId", jsonPath: "$.id" }],
      fieldMappings: [{ jsonPath: "$.first", target: "first_name" }],
      actions: [],
    }
    const webhook = await activeWebhook(api, config)
    const otherHook = await activeWebhook(api, config)
    const requestId = await deliverAccepted(webhook.endpointUrl as string, { id: "x" })
    const other = await tenantApi("p109-proc-requests-404-other")

    expectError(await api.get(`${BASE}/1/requests`), 404, "notFound", { defined: true, message: "Inbound webhook not found" })
    expectError(await other.api.get(`${BASE}/${webhook.id}/requests`), 404, "notFound", { defined: true })
    const requestNotFound = { defined: true, message: "Inbound webhook request not found" }
    expectError(await api.get(`${BASE}/${webhook.id}/requests/1`), 404, "notFound", requestNotFound)
    // A request is read through its own webhook only.
    expectError(await api.get(`${BASE}/${otherHook.id}/requests/${requestId}`), 404, "notFound", requestNotFound)
    expectError(await api.get(`${BASE}/1/requests/${requestId}`), 404, "notFound", requestNotFound)
    expectError(await other.api.get(`${BASE}/${webhook.id}/requests/${requestId}`), 404, "notFound", requestNotFound)

    expectInvalid(await api.get(`${BASE}/${webhook.id}/requests?status=done`), "status")
    expectInvalid(await api.get(`${BASE}/${webhook.id}/requests?perPage=0`), "perPage")
    expectInvalid(await api.get(`${BASE}/abc/requests`), "id")
    expectInvalid(await api.get(`${BASE}/${webhook.id}/requests/abc`), "requestId")
    expectInvalid(await api.get(`${BASE}/${webhook.id}/requests/${requestId}?includePayload=yes`), "includePayload")
  })
})
