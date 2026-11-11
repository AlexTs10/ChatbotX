// channelIntegrations.list

import { describe, expect, it } from "vitest"
import { Normalizer } from "../../src/normalize.ts"
import { uniqueId } from "../../src/unique.ts"
import { createWebchat, expectInvalid, ext, newTenant } from "./helpers.ts"

describe("channelIntegrations.list", () => {
  it("is an empty array for a workspace without Meta/Zalo/TikTok channels", async () => {
    const { api } = await newTenant("chint-empty")
    // A webchat is not a channel integration of this list.
    await createWebchat(api)
    const response = await api.get("/v1/channel-integrations")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual([])
  })

  it("lists a connected Zalo OA without credentials; the channel filter narrows", async () => {
    const normalizer = new Normalizer()
    const { tenant, api } = await newTenant("chint-zalo")
    const oaId = `oa-${uniqueId()}`
    const { integrationId, inboxId } = await (await ext()).createZaloIntegration({
      workspaceId: tenant.workspaceId,
      name: "Parity OA",
      oaId,
    })
    const response = await api.get<Array<Record<string, unknown>>>("/v1/channel-integrations")
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual([
      {
        id: integrationId,
        channel: "zalo",
        inboxId,
        name: "Parity OA",
        externalId: oaId,
        wabaId: null,
        displayPhoneNumber: null,
        username: null,
        coexistEnabled: false,
        isCoexist: null,
        hasCapiScope: false,
        capiConnected: false,
        datasetId: null,
        capiTestEventCode: null,
        capiDisconnected: false,
        syncTagEnabledAt: null,
        handoverResumeFlowId: null,
        tokenRefreshError: null,
        adsEligible: false,
      },
    ])
    expect(JSON.stringify(response.json)).not.toContain("parity-zalo-token")
    expect(normalizer.normalize(response.json, { replace: { [oaId]: "<oaId>" } })).toMatchSnapshot("zalo")

    const zalo = await api.get("/v1/channel-integrations", { query: { channel: "zalo" } })
    expect(zalo.json).toEqual(response.json)
    const whatsapp = await api.get("/v1/channel-integrations", { query: { channel: "whatsapp" } })
    expect(whatsapp.status).toBe(200)
    expect(whatsapp.json).toEqual([])
  })

  it("rejects a channel outside the list with 422", async () => {
    const { api } = await newTenant("chint-invalid")
    expectInvalid(await api.get("/v1/channel-integrations", { query: { channel: "webchat" } }), "channel")
    expectInvalid(await api.get("/v1/channel-integrations", { query: { channel: "fax" } }), "channel")
  })
})
