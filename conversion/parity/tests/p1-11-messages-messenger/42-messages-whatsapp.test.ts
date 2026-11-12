// WhatsApp-only message routes (scope `inbox`):
// - requestWhatsappCallPermission: reads Meta's counter
//   (`GET /<phoneNumberId>/call_permissions?user_wa_id=<digits>`), refuses when
//   Meta says no request is left, else stores an outgoing message that the
//   worker sends as the `call_permission_request` interactive
//   (`POST /<phoneNumberId>/messages`).
// - sendWhatsappTemplate: resolves `templateParams` against the stored
//   template (422 lists missing / unknown keys), then the worker sends the
//   approved template (`POST /<phoneNumberId>/messages`, type template).
// - getFlowResponse: the answers of a WhatsApp Flow response message,
//   humanized keys when the Flow cannot be identified (`labelSource: fallback`).
// Phone number ids are unique per test, so every Graph path is test-owned.
// Templates and inbound Flow responses come from the fixture extension (they
// arrive by template sync and by Meta webhook in the app).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import type { Tenant } from "../../src/fixtures/index.ts"
import {
  type Api,
  type Conversation,
  expectError,
  expectInvalid,
  expectNotFound,
  expectSystemError,
  ext,
  GRAPH,
  graphError,
  graphPath,
  listMessages,
  messagesPath,
  newTenant,
  parseBody,
  seedConversation,
  UNKNOWN_ID,
  uniqueDigits,
  uniqueName,
  upstreamCalls,
  wait,
  waitForCalls,
  webchatConversation,
} from "./helpers.ts"

type Whatsapp = { integrationId: string; inboxId: string; accessToken: string; phoneNumberId: string }
type Setup = { tenant: Tenant; api: Api; wa: Whatsapp; conversation: Conversation }

/** A unique phone-like WhatsApp id (digits). */
const uniquePhone = (): string => `849${uniqueDigits().slice(-9)}`

const setup = async (label: string): Promise<Setup> => {
  const { tenant, api } = await newTenant(label)
  const phoneNumberId = uniqueDigits()
  const created = await (await ext()).createWhatsappChannel({
    workspaceId: tenant.workspaceId,
    phoneNumberId,
    name: uniqueName("WA"),
  })
  const wa = { ...created, phoneNumberId }
  const conversation = await seedConversation(tenant, {
    inboxId: wa.inboxId,
    channel: "whatsapp",
    sourceId: uniquePhone(),
    lastIncomingMessageAt: new Date().toISOString(),
  })
  return { tenant, api, wa, conversation }
}

const permissions = (canSend: boolean) => ({
  messaging_product: "whatsapp",
  permission: { status: "no_permission" },
  actions: [
    { action_name: "send_call_permission_request", can_perform_action: canSend },
    { action_name: "start_call", can_perform_action: false },
  ],
})

const answerPermissions = (wa: Whatsapp, respond: { status: number; json: unknown }) =>
  fake.script({ match: { host: GRAPH, method: "GET", pathRegex: graphPath(`${wa.phoneNumberId}/call_permissions`) }, respond })

const answerSend = (wa: Whatsapp, wamid: string) =>
  fake.script({
    match: { host: GRAPH, method: "POST", pathRegex: graphPath(`${wa.phoneNumberId}/messages`) },
    respond: {
      status: 200,
      json: { messaging_product: "whatsapp", contacts: [{ input: "x", wa_id: "x" }], messages: [{ id: wamid }] },
    },
  })

const sendCalls = (wa: Whatsapp, since: number) =>
  upstreamCalls({ host: GRAPH, since, path: new RegExp(graphPath(`${wa.phoneNumberId}/messages`)), method: "POST" })

describe("messages.requestWhatsappCallPermission", () => {
  it("checks Meta's counter, then the worker sends the call_permission_request interactive (202)", async () => {
    const { api, wa, conversation } = await setup("p111-wa-callperm")
    await answerPermissions(wa, { status: 200, json: permissions(true) })
    const wamid = `wamid.${uniqueName("cp")}`
    await answerSend(wa, wamid)
    const since = await fake.cursor()
    const response = await api.post(`/v1/conversations/${conversation.conversationId}/whatsapp-call-permission`, {
      text: "  May we call you?  ",
    })
    expect(response.status, response.text).toBe(202)
    expect(response.json).toEqual({ queued: true })

    const checks = await upstreamCalls({
      host: GRAPH,
      since,
      path: new RegExp(graphPath(`${wa.phoneNumberId}/call_permissions`)),
    })
    expect(checks).toHaveLength(1)
    expect(checks[0]?.query).toEqual({ user_wa_id: conversation.sourceId })
    expect(checks[0]?.headers.authorization).toBe(`Bearer ${wa.accessToken}`)

    const [send] = await waitForCalls({
      host: GRAPH,
      since,
      path: new RegExp(graphPath(`${wa.phoneNumberId}/messages`)),
      method: "POST",
    })
    expect(parseBody(send as NonNullable<typeof send>)).toEqual({
      messaging_product: "whatsapp",
      recipient_type: "individual",
      to: conversation.sourceId,
      type: "interactive",
      interactive: {
        type: "call_permission_request",
        body: { text: "May we call you?" },
        action: { name: "call_permission_request" },
      },
    })
    const [message] = (await listMessages(api, conversation.conversationId)).data
    expect(message).toMatchObject({
      text: "May we call you?",
      messageType: "outgoing",
      senderType: "api",
      contentAttributes: { type: "whatsapp_call_permission_request" },
    })
    await wait(async () => {
      const [row] = (await listMessages(api, conversation.conversationId)).data
      expect(row?.sourceId).toBe(wamid)
    })
  })

  it("refuses when Meta says the limit is reached, or relays Meta's error; nothing is sent", async () => {
    const limited = await setup("p111-wa-callperm-limit")
    await answerPermissions(limited.wa, { status: 200, json: permissions(false) })
    const since = await fake.cursor()
    expectSystemError(
      await limited.api.post(`/v1/conversations/${limited.conversation.conversationId}/whatsapp-call-permission`, {
        text: "Call?",
      }),
      "Meta allows one call-permission request per 24 hours and two per 7 days for each customer: the limit is reached.",
    )
    expect(await sendCalls(limited.wa, since)).toEqual([])
    expect((await listMessages(limited.api, limited.conversation.conversationId)).data).toEqual([])

    const failing = await setup("p111-wa-callperm-meta")
    await answerPermissions(failing.wa, { status: 400, json: graphError("(#138006) Calling is not enabled", 138006) })
    const response = await failing.api.post(
      `/v1/conversations/${failing.conversation.conversationId}/whatsapp-call-permission`,
      { text: "Call?" },
    )
    expectError(response, 400, "systemError")
    expect((response.json as { message: string }).message).toContain("Calling is not enabled")
    expect((await listMessages(failing.api, failing.conversation.conversationId)).data).toEqual([])
  })

  it("errors: not a WhatsApp conversation 400, inboxId not the contact's 404, unknown conversation 404, bad text 422", async () => {
    const { tenant, api, conversation } = await setup("p111-wa-callperm-err")
    const webchat = await webchatConversation(tenant, api)
    const since = await fake.cursor()
    expectSystemError(
      await api.post(`/v1/conversations/${webchat.conversationId}/whatsapp-call-permission`, { text: "Call?" }),
      "This conversation is not on WhatsApp.",
    )
    expectNotFound(
      await api.post(`/v1/conversations/${conversation.conversationId}/whatsapp-call-permission`, {
        text: "Call?",
        inboxId: UNKNOWN_ID,
      }),
      "The contact has no WhatsApp inbox with this `inboxId`.",
    )
    expectNotFound(
      await api.post(`/v1/conversations/${UNKNOWN_ID}/whatsapp-call-permission`, { text: "Call?" }),
      "Conversation not found",
    )
    const path = `/v1/conversations/${conversation.conversationId}/whatsapp-call-permission`
    expectInvalid(await api.post(path, { text: "   " }), "text")
    expectInvalid(await api.post(path, { text: "x".repeat(1025) }), "text")
    expectInvalid(await api.post(path, {}), "text")
    expect(await upstreamCalls({ host: GRAPH, since, path: /call_permissions/ })).toEqual([])
  })
})

const BODY_TEMPLATE = [{ type: "BODY", text: "Hello {{1}}, your order is ready", example: { body_text: [["Ada"]] } }]

describe("messages.sendWhatsappTemplate", () => {
  it("fills templateParams and the worker sends the approved template (202)", async () => {
    const { api, wa, conversation } = await setup("p111-wa-template")
    const name = `order_ready_${uniqueDigits().slice(-6)}`
    const { templateId } = await (await ext()).createWhatsappTemplate({
      integrationWhatsappId: wa.integrationId,
      name,
      language: "en_US",
      status: "APPROVED",
      components: BODY_TEMPLATE,
    })
    await answerSend(wa, `wamid.${uniqueName("tpl")}`)
    const since = await fake.cursor()
    const response = await api.post(`/v1/conversations/${conversation.conversationId}/whatsapp-template`, {
      templateId,
      templateParams: { "body.1": "Grace" },
    })
    expect(response.status, response.text).toBe(202)
    expect(response.json).toEqual({ queued: true })
    const [send] = await waitForCalls({
      host: GRAPH,
      since,
      path: new RegExp(graphPath(`${wa.phoneNumberId}/messages`)),
      method: "POST",
    })
    const body = parseBody<Record<string, unknown>>(send as NonNullable<typeof send>)
    expect(body).toMatchObject({
      messaging_product: "whatsapp",
      to: conversation.sourceId,
      type: "template",
      template: { name, language: { code: "en_US" } },
    })
    expect(JSON.stringify(body.template)).toContain("Grace")
    expect(send?.headers.authorization).toBe(`Bearer ${wa.accessToken}`)
    await wait(async () => {
      const messages = (await listMessages(api, conversation.conversationId)).data
      expect(messages.filter((message) => message.messageType === "outgoing")).toHaveLength(1)
    })
  })

  it("templateParams that do not match the template -> 422 validation listing the keys", async () => {
    const { api, wa, conversation } = await setup("p111-wa-template-params")
    const { templateId } = await (await ext()).createWhatsappTemplate({
      integrationWhatsappId: wa.integrationId,
      name: `params_${uniqueDigits().slice(-6)}`,
      language: "en_US",
      status: "APPROVED",
      components: BODY_TEMPLATE,
    })
    const path = `/v1/conversations/${conversation.conversationId}/whatsapp-template`
    const since = await fake.cursor()
    expectError(await api.post(path, { templateId, templateParams: { "body.9": "x" } }), 422, "validation", {
      message: "Template parameters do not match the template (missing: body.1; unknown: body.9)",
    })
    expectError(await api.post(path, { templateId, templateParams: {} }), 422, "validation", {
      message: "Template parameters do not match the template (missing: body.1)",
    })
    expectNotFound(await api.post(path, { templateId: UNKNOWN_ID, templateParams: { "body.1": "x" } }), "Template not found")
    expectInvalid(
      await api.post(path, { templateId, templateParams: { "body.1": "x" }, templateData: { body: [] } }),
      "templateParams",
    )
    expectInvalid(await api.post(path, { templateParams: { "body.1": "x" } }), "templateId")
    expect(await sendCalls(wa, since)).toEqual([])
  })

  it("a conversation without a WhatsApp inbox is refused (404 / 400); unknown conversation 404", async () => {
    const { tenant, api } = await newTenant("p111-wa-template-nowa")
    const webchat = await webchatConversation(tenant, api)
    const path = `/v1/conversations/${webchat.conversationId}/whatsapp-template`
    expectNotFound(await api.post(path, { templateId: "1" }), "Inbox not found")
    expectSystemError(await api.post(path, { templateId: "1", inboxId: webchat.inboxId }), "This conversation has no WhatsApp inbox")
    expectNotFound(await api.post(`/v1/conversations/${UNKNOWN_ID}/whatsapp-template`, { templateId: "1" }), "Conversation not found")
  })
})

describe("messages.getFlowResponse", () => {
  it("returns the answers with humanized labels when the Flow is unknown (labelSource fallback)", async () => {
    const { tenant, api, conversation } = await setup("p111-wa-flowresp")
    // Keys in jsonb order (shorter first, then bytewise), so insertion order and stored order agree.
    const flowResponse = {
      age: 42,
      topics: ["shoes", "bags"],
      address: { street: "Main" },
      documents: [{ file_name: "id.pdf", mime_type: "application/pdf", id: "media-1" }],
      full_name: "Ada Lovelace",
      flow_token: "unknown-flow-token",
      newsletterOptIn: true,
    }
    const message = await (await ext()).createMessage({
      workspaceId: tenant.workspaceId,
      conversationId: conversation.conversationId,
      contactInboxId: conversation.contactInboxId,
      messageType: "incoming",
      senderType: "contact",
      text: "Sent",
      sourceId: `wamid.${uniqueName("flow")}`,
      contentAttributes: {
        type: "whatsapp_flow_response",
        name: "flow",
        flowResponse,
        flowToken: "unknown-flow-token",
        decoded: null,
      },
    })
    const response = await api.get(messagesPath(conversation.conversationId, `/${message.messageId}/flow-response`), {
      query: { createdAt: message.createdAt },
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      labelSource: "fallback",
      fields: [
        { key: "age", label: "Age", kind: "text", value: "42" },
        { key: "topics", label: "Topics", kind: "list", value: ["shoes", "bags"] },
        { key: "address", label: "Address", kind: "json", value: '{"street":"Main"}' },
        {
          key: "documents",
          label: "Documents",
          kind: "files",
          value: [{ status: "unprocessed", fileName: "id.pdf", mimeType: "application/pdf", mediaId: "media-1" }],
        },
        { key: "full_name", label: "Full name", kind: "text", value: "Ada Lovelace" },
        { key: "newsletterOptIn", label: "Newsletter Opt In", kind: "boolean", value: true },
      ],
    })
  })
})
