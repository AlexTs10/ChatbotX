// capabilities.get

import { describe, expect, it } from "vitest"
import { Normalizer } from "../../src/normalize.ts"
import { createPublishedFlow, createWebchat, expectInvalid, newTenant } from "./helpers.ts"

type Capabilities = {
  inboxes?: Array<{ id: string; name: string; channel: string }>
  templates?: unknown[]
  customFields?: unknown[]
  botFields?: Array<{ id: string; name: string; type: string }>
  tags?: unknown[]
  aiAgents?: unknown[]
  sequences?: unknown[]
  flows?: Array<{ id: string; name: string }>
  flowSpec?: { selectedChannelPolicy?: unknown; channels: string[]; stepTypes: unknown[] }
}

describe("capabilities.get", () => {
  it("returns the default sets for a new workspace", async () => {
    const normalizer = new Normalizer()
    const { api } = await newTenant("caps-default")
    const response = await api.get<Capabilities>("/v1/capabilities")
    expect(response.status, response.text).toBe(200)
    expect(Object.keys(response.json).sort()).toEqual(
      ["botFields", "customFields", "flowSpec", "flows", "inboxes", "sequences", "tags", "templates"].sort(),
    )
    expect(response.json).toMatchObject({
      inboxes: [],
      templates: [],
      customFields: [],
      botFields: [],
      tags: [],
      sequences: [],
      flows: [],
    })
    expect(response.json.flowSpec?.selectedChannelPolicy).toBeUndefined()
    expect(normalizer.normalize(response.json)).toMatchSnapshot("default")
  })

  it("lists the workspace's inboxes, bot fields and flows", async () => {
    const { api } = await newTenant("caps-content")
    const webchat = await createWebchat(api, { name: "Caps webchat" })
    const field = await api.post<{ id: string }>("/v1/bot-fields", { name: "caps_field", type: "number" })
    expect(field.status).toBe(201)
    const flowId = await createPublishedFlow(api, "Caps flow")

    const response = await api.get<Capabilities>("/v1/capabilities", {
      query: { include: "inboxes,botFields,flows" },
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      inboxes: [{ id: webchat.inboxId, name: "Caps webchat", channel: "webchat" }],
      botFields: [{ id: field.json.id, name: "caps_field", type: "number" }],
      flows: [{ id: flowId, name: "Caps flow" }],
    })
  })

  it("narrows with include and projects a channel policy", async () => {
    const normalizer = new Normalizer()
    const { api } = await newTenant("caps-channel")
    const aiOnly = await api.get<Capabilities>("/v1/capabilities", { query: { include: " aiAgents , tags " } })
    expect(aiOnly.status, aiOnly.text).toBe(200)
    expect(aiOnly.json).toEqual({ aiAgents: [], tags: [] })

    const policy = await api.get<Capabilities>("/v1/capabilities", {
      query: { include: "flowSpec", channel: "whatsapp" },
    })
    expect(policy.status, policy.text).toBe(200)
    expect(Object.keys(policy.json)).toEqual(["flowSpec"])
    expect(policy.json.flowSpec?.selectedChannelPolicy).toBeDefined()
    expect(normalizer.normalize(policy.json)).toMatchSnapshot("whatsapp policy")
  })

  it("rejects an unknown include or channel with 422", async () => {
    const { api } = await newTenant("caps-invalid")
    expectInvalid(await api.get("/v1/capabilities", { query: { include: "inboxes,secrets" } }), "include.1")
    expectInvalid(await api.get("/v1/capabilities", { query: { channel: "fax" } }), "channel")
  })
})
