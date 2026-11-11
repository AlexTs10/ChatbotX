// ads.checkCampaignPrerequisites, ads.listConnections, ads.disconnectConnection.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { Normalizer } from "../../src/normalize.ts"
import {
  adsFixtures,
  expectValidationIssue,
  GRAPH,
  graphCalls,
  graphError,
  graphScript,
  messengerAdsSetup,
  uniqueAccessToken,
  uniqueDigits,
} from "./ads-helpers.ts"

describe("ads.checkCampaignPrerequisites (GET /v1/ads/campaigns/prerequisites)", () => {
  it("reports no connection, an active one and an invalid one", async () => {
    const none = await messengerAdsSetup({ label: "prereq-none", connect: false })
    const noneResponse = await none.api.get("/v1/ads/campaigns/prerequisites", {
      query: { channel: "messenger", integrationId: none.integrationId },
    })
    expect(noneResponse.status, noneResponse.text).toBe(200)
    expect(noneResponse.json).toEqual({ connected: false, reconnectNeeded: false })

    const active = await messengerAdsSetup({ label: "prereq-active" })
    const activeResponse = await active.api.get("/v1/ads/campaigns/prerequisites", {
      query: { channel: "messenger", integrationId: active.integrationId },
    })
    expect(activeResponse.json).toEqual({ connected: true, reconnectNeeded: false })

    const invalid = await messengerAdsSetup({ label: "prereq-invalid", status: "invalid" })
    const invalidResponse = await invalid.api.get("/v1/ads/campaigns/prerequisites", {
      query: { channel: "messenger", integrationId: invalid.integrationId },
    })
    expect(invalidResponse.json).toEqual({ connected: false, reconnectNeeded: true })

    // The connection belongs to one channel: the same id under another channel is not found.
    const otherChannel = await active.api.get("/v1/ads/campaigns/prerequisites", {
      query: { channel: "instagram", integrationId: active.integrationId },
    })
    expect(otherChannel.json).toEqual({ connected: false, reconnectNeeded: false })
    expect(await graphCalls(active.accessToken)).toEqual([])
  })

  it("rejects missing or bad input with 422", async () => {
    const { api, integrationId } = await messengerAdsSetup({ label: "prereq-422", connect: false })
    expectValidationIssue(await api.get("/v1/ads/campaigns/prerequisites", { query: { channel: "messenger" } }), [
      "integrationId",
    ])
    expectValidationIssue(
      await api.get("/v1/ads/campaigns/prerequisites", { query: { channel: "telegram", integrationId } }),
      ["channel"],
    )
    expectValidationIssue(
      await api.get("/v1/ads/campaigns/prerequisites", { query: { channel: "messenger", integrationId: "abc" } }),
      ["integrationId"],
    )
  })
})

describe("ads.listConnections (GET /v1/ads/connections)", () => {
  it("lists only active connections of the channel", async () => {
    const normalizer = new Normalizer()
    const setup = await messengerAdsSetup({ label: "connections" })
    const ext = await adsFixtures()
    const secondPage = uniqueDigits()
    const second = await ext.createMessengerChannel({
      workspaceId: setup.tenant.workspaceId,
      pageId: secondPage,
      name: `Page ${secondPage}`,
    })
    await ext.connectMessagingAds({
      workspaceId: setup.tenant.workspaceId,
      channel: "messenger",
      integrationId: second.integrationId,
      accessToken: uniqueAccessToken(),
      status: "invalid",
    })

    const response = await setup.api.get("/v1/ads/connections", { query: { channel: "messenger" } })
    expect(response.status, response.text).toBe(200)
    expect(normalizer.normalize(response.json, { replace: { [setup.integrationId]: "<integration>" } })).toEqual({
      data: [
        {
          id: "<id:1>",
          channel: "messenger",
          integrationId: "<integration>",
          status: "active",
          createdAt: "<ts>",
          updatedAt: "<ts>",
        },
      ],
    })

    const whatsapp = await setup.api.get("/v1/ads/connections", { query: { channel: "whatsapp" } })
    expect(whatsapp.json).toEqual({ data: [] })
    expectValidationIssue(await setup.api.get("/v1/ads/connections"), ["channel"])
  })
})

describe("ads.disconnectConnection (DELETE /v1/ads/connections/{channel}/{integrationId})", () => {
  it("revokes the Graph token, deletes the connection and answers 204", async () => {
    const setup = await messengerAdsSetup({ label: "disconnect" })
    const since = await fake.cursor()
    await graphScript("DELETE", "/me/permissions", { status: 200, json: { success: true } })

    const response = await setup.api.delete(`/v1/ads/connections/messenger/${setup.integrationId}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")

    const calls = await graphCalls(setup.accessToken, since)
    expect(calls.map((call) => ({ method: call.method, path: call.path, query: call.query }))).toEqual([
      { method: "DELETE", path: "/v23.0/me/permissions", query: { access_token: setup.accessToken } },
    ])

    const list = await setup.api.get("/v1/ads/connections", { query: { channel: "messenger" } })
    expect(list.json).toEqual({ data: [] })
    const prerequisites = await setup.api.get("/v1/ads/campaigns/prerequisites", {
      query: { channel: "messenger", integrationId: setup.integrationId },
    })
    expect(prerequisites.json).toEqual({ connected: false, reconnectNeeded: false })

    // Nothing left to disconnect: still 204, no Graph call.
    const again = await setup.api.delete(`/v1/ads/connections/messenger/${setup.integrationId}`)
    expect(again.status).toBe(204)
    expect(await graphCalls(setup.accessToken, since)).toHaveLength(1)
  })

  it("still deletes the connection when Meta refuses the revoke", async () => {
    const setup = await messengerAdsSetup({ label: "disconnect-fail" })
    const since = await fake.cursor()
    await graphScript("DELETE", "/me/permissions", graphError(400, 190, "Error validating access token"))

    const response = await setup.api.delete(`/v1/ads/connections/messenger/${setup.integrationId}`)
    expect(response.status, response.text).toBe(204)
    expect(await graphCalls(setup.accessToken, since)).toHaveLength(1)
    const list = await setup.api.get("/v1/ads/connections", { query: { channel: "messenger" } })
    expect(list.json).toEqual({ data: [] })
  })

  it("deletes a connection whose stored auth cannot be decrypted without a Graph call", async () => {
    const setup = await messengerAdsSetup({ label: "disconnect-corrupt", connect: false })
    const ext = await adsFixtures()
    await ext.connectMessagingAds({
      workspaceId: setup.tenant.workspaceId,
      channel: "messenger",
      integrationId: setup.integrationId,
      accessToken: setup.accessToken,
      corruptAuth: true,
    })
    const since = await fake.cursor()
    const response = await setup.api.delete(`/v1/ads/connections/messenger/${setup.integrationId}`)
    expect(response.status, response.text).toBe(204)
    expect((await fake.requests({ kind: "http", host: GRAPH, since })).length).toBe(0)
    const list = await setup.api.get("/v1/ads/connections", { query: { channel: "messenger" } })
    expect(list.json).toEqual({ data: [] })
  })

  it("rejects a bad channel or id with 422", async () => {
    const setup = await messengerAdsSetup({ label: "disconnect-422", connect: false })
    expectValidationIssue(await setup.api.delete(`/v1/ads/connections/telegram/${setup.integrationId}`), ["channel"])
    expectValidationIssue(await setup.api.delete("/v1/ads/connections/messenger/abc"), ["integrationId"])
  })
})
