// inboundWebhooks.previewMapping: what a delivery of a sample payload would
// extract (inbox identifier, identities, lookup key, per-field verdict),
// without writing anything.

import { describe, expect, it } from "vitest"
import {
  createApiChannel,
  createCustomField,
  createWebchat,
  createWebhook,
  expectInvalid,
  tenantApi,
  uniqueName,
  type WebhookSummary,
} from "./helpers-webhooks.ts"

const PREVIEW = "/v1/inbound-webhooks/preview-mapping"

describe("inboundWebhooks.previewMapping", () => {
  it("extracts identities, the lookup key and every mapped value with its verdict", async () => {
    const { api } = await tenantApi("p109-iwh-preview")
    const { inboxId } = await createApiChannel(api)
    const tier = await createCustomField(api, uniqueName("tier"), "shortText")
    const score = await createCustomField(api, uniqueName("score"), "number")
    const payload = {
      contact: {
        id: "ext-42",
        email: "ada@parity.test",
        phone: "+14155552671",
        name: "  Ada \t  Lovelace ",
        first: "Ada",
      },
      order: { total: 42, tier: "gold", score: "not a number", gender: "FEMALE", badPhone: "abc", badEmail: "nope" },
      items: [{ sku: "A-1" }],
    }
    const response = await api.post(PREVIEW, {
      payload,
      inboxResolution: { mode: "inbox", inboxId },
      identifiers: [
        { kind: "fullName", jsonPath: "$.contact.name" },
        { kind: "email", jsonPath: "contact.email" },
        { kind: "phone", jsonPath: "$.contact.phone" },
        { kind: "channelUserId", jsonPath: "$.contact.id" },
      ],
      fieldMappings: [
        { jsonPath: "$.contact.first", target: "first_name" },
        { jsonPath: "$.order.tier", target: tier },
        { jsonPath: "$.order.score", target: score },
        { jsonPath: "$.order.total", target: score },
        { jsonPath: "$.order.gender", target: "gender" },
        { jsonPath: "$.order.badPhone", target: "phone" },
        { jsonPath: "$.order.badEmail", target: "email" },
        { jsonPath: "$.items[0]", target: "last_name" },
        { jsonPath: "$.missing", target: "full_name" },
        { jsonPath: "$.order.tier", target: "nickname" },
      ],
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      inboxIdentifier: null,
      identifiers: [
        { kind: "fullName", raw: "Ada \t  Lovelace", normalized: "Ada Lovelace" },
        { kind: "email", raw: "ada@parity.test", normalized: "ada@parity.test" },
        { kind: "phone", raw: "+14155552671", normalized: "+14155552671" },
        { kind: "channelUserId", raw: "ext-42", normalized: "ext-42" },
      ],
      identities: {
        channelUserId: "ext-42",
        phoneE164: "+14155552671",
        email: "ada@parity.test",
        fullName: "Ada Lovelace",
      },
      // The first usable identity in the fixed order channelUserId, phone, email, fullName.
      contactLookup: { kind: "channelUserId", value: "ext-42" },
      fields: [
        { target: "first_name", kind: "richSystem", raw: "Ada", value: "Ada", error: null },
        { target: tier, kind: "customField", raw: "gold", value: "gold", error: null },
        { target: score, kind: "customField", raw: "not a number", value: null, error: "invalidValue" },
        { target: score, kind: "customField", raw: "42", value: "42", error: null },
        { target: "gender", kind: "gender", raw: "FEMALE", value: "female", error: null },
        { target: "phone", kind: "richSystem", raw: "abc", value: null, error: "invalidPhone" },
        { target: "email", kind: "richSystem", raw: "nope", value: null, error: "invalidValue" },
        // A non-string value is stored as its JSON text.
        { target: "last_name", kind: "richSystem", raw: '{"sku":"A-1"}', value: '{"sku":"A-1"}', error: null },
        { target: "full_name", kind: "richSystem", raw: null, value: null, error: null },
        { target: "nickname", kind: "customField", raw: "gold", value: null, error: "unknownField" },
      ],
    })
  })

  it("reads the inbox identifier in payload mode and drops malformed identities", async () => {
    const { api } = await tenantApi("p109-iwh-preview-payload")
    const response = await api.post(PREVIEW, {
      payload: { entry: [{ id: 1234567890 }], sender: { id: 9007199254740993 }, email: "not-an-email", phone: "12" },
      inboxResolution: { mode: "payload", channel: "messenger", identifierKind: "pageId", jsonPath: "entry[0].id" },
      identifiers: [
        { kind: "channelUserId", jsonPath: "$.sender.id" },
        { kind: "email", jsonPath: "$.email" },
        { kind: "phone", jsonPath: "$.phone" },
      ],
      fieldMappings: [],
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      inboxIdentifier: "1234567890",
      identifiers: [
        // A number beyond 2^53 was rounded by JSON.parse: treated as absent.
        { kind: "channelUserId", raw: null, normalized: null },
        { kind: "email", raw: "not-an-email", normalized: null },
        { kind: "phone", raw: "12", normalized: null },
      ],
      identities: {},
      contactLookup: null,
      fields: [],
    })
  })

  it("never uses a channel user id the inbox channel cannot address (webchat lookup by email)", async () => {
    const { api } = await tenantApi("p109-iwh-preview-webchat")
    const { inboxId } = await createWebchat(api)
    const response = await api.post<{ contactLookup: unknown; identities: unknown }>(PREVIEW, {
      payload: { name: "Grace Hopper", mail: "grace@parity.test" },
      inboxResolution: { mode: "inbox", inboxId },
      identifiers: [
        { kind: "fullName", jsonPath: "$.name" },
        { kind: "email", jsonPath: "$.mail" },
      ],
      fieldMappings: [{ jsonPath: "$.name", target: "full_name" }],
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({
      inboxIdentifier: null,
      identities: { email: "grace@parity.test", fullName: "Grace Hopper" },
      contactLookup: { kind: "email", value: "grace@parity.test" },
    })
  })

  it("works without an inbox rule and with an unknown inbox id; writes nothing", async () => {
    const { api } = await tenantApi("p109-iwh-preview-noinbox")
    const body = {
      payload: [{ id: "x-1" }],
      identifiers: [{ kind: "channelUserId", jsonPath: "$[0].id" }],
      fieldMappings: [{ jsonPath: "$[0].id", target: "first_name" }],
    }
    for (const inboxResolution of [null, { mode: "inbox", inboxId: "1" }]) {
      const response = await api.post(PREVIEW, { ...body, inboxResolution })
      expect(response.status, response.text).toBe(200)
      expect(response.json).toEqual({
        inboxIdentifier: null,
        identifiers: [{ kind: "channelUserId", raw: "x-1", normalized: "x-1" }],
        identities: { channelUserId: "x-1" },
        contactLookup: { kind: "channelUserId", value: "x-1" },
        fields: [{ target: "first_name", kind: "richSystem", raw: "x-1", value: "x-1", error: null }],
      })
    }
    const list = await api.get<{ data: WebhookSummary[] }>("/v1/inbound-webhooks")
    expect(list.json.data).toEqual([])
    const contacts = await api.get<{ data: unknown[] }>("/v1/contacts")
    expect(contacts.json.data).toEqual([])
  })

  it("validates the input (422 invalidRequestData)", async () => {
    const { api } = await tenantApi("p109-iwh-preview-bad")
    await createWebhook(api)
    const base = { payload: {}, inboxResolution: null, identifiers: [], fieldMappings: [] }
    const { identifiers: _i, ...noIdentifiers } = base
    expectInvalid(await api.post(PREVIEW, noIdentifiers), "identifiers")
    const { fieldMappings: _f, ...noMappings } = base
    expectInvalid(await api.post(PREVIEW, noMappings), "fieldMappings")
    const { inboxResolution: _r, ...noResolution } = base
    expectInvalid(await api.post(PREVIEW, noResolution), "inboxResolution")
    expectInvalid(
      await api.post(PREVIEW, {
        ...base,
        identifiers: [
          { kind: "phone", jsonPath: "$.a" },
          { kind: "phone", jsonPath: "$.b" },
        ],
      }),
      "identifiers",
    )
    expectInvalid(await api.post(PREVIEW, { ...base, identifiers: [{ kind: "phone", jsonPath: "$" }] }), "identifiers.0.jsonPath")
    expectInvalid(await api.post(PREVIEW, { ...base, payload: { text: "a\u0000b" } }), "payload")
    expectInvalid(await api.post(PREVIEW, { ...base, payload: { blob: "x".repeat(256 * 1024) } }), "payload")
  })
})
