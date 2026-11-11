// Wizard reads and uploads: ads.listCampaignAdAccounts,
// ads.getCampaignAdAccountDetails, ads.createCampaignImageUpload,
// ads.uploadCampaignVideo, ads.getCampaignVideoStatus,
// ads.listCampaignMessengerPages.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { fixtures } from "../../src/fixtures/index.ts"
import { apiClient, HttpClient } from "../../src/http.ts"
import { Normalizer } from "../../src/normalize.ts"
import {
  adsFixtures,
  expectError,
  expectValidationIssue,
  graphCalls,
  graphError,
  graphScript,
  messengerAdsSetup,
  PNG_1X1,
  RECONNECT_MESSAGE,
  uniqueAdAccountId,
  uniqueDigits,
} from "./ads-helpers.ts"

describe("ads.listCampaignAdAccounts (GET /v1/ads/campaigns/{channel}/{integrationId}/ad-accounts)", () => {
  it("lists the connection's ad accounts across Graph pages and caches them", async () => {
    const setup = await messengerAdsSetup({ label: "ad-accounts" })
    const since = await fake.cursor()
    const path = `/v1/ads/campaigns/messenger/${setup.integrationId}/ad-accounts`
    // Older script = second page; the newer one answers once (first page).
    await graphScript("GET", "/me/adaccounts", { status: 200, json: { data: [{ id: "act_2" }] } }, { times: 1 })
    await graphScript(
      "GET",
      "/me/adaccounts",
      {
        status: 200,
        json: {
          data: [{ id: "act_1", name: "First account" }],
          paging: { cursors: { after: "CURSOR1" }, next: "https://graph.facebook.com/v23.0/me/adaccounts?after=CURSOR1" },
        },
      },
      { times: 1 },
    )

    const response = await setup.api.get(path)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ data: [{ id: "act_1", name: "First account" }, { id: "act_2" }] })

    const calls = await graphCalls(setup.accessToken, since)
    expect(calls.map((call) => ({ method: call.method, path: call.path, query: call.query }))).toEqual([
      {
        method: "GET",
        path: "/v23.0/me/adaccounts",
        query: { fields: "id,name", limit: "499", access_token: setup.accessToken },
      },
      {
        method: "GET",
        path: "/v23.0/me/adaccounts",
        query: { fields: "id,name", limit: "499", access_token: setup.accessToken, after: "CURSOR1" },
      },
    ])

    // Served from the cache: no new Graph call.
    const cached = await setup.api.get(path)
    expect(cached.json).toEqual(response.json)
    expect(await graphCalls(setup.accessToken, since)).toHaveLength(2)

    // A read-only token's refresh=true is ignored (still cached).
    const driver = await fixtures()
    const readOnly = await driver.createWorkspaceToken({
      workspaceId: setup.tenant.workspaceId,
      name: "ro",
      permission: "read_only",
      scopes: null,
    })
    const readOnlyResponse = await apiClient(readOnly.token).get(path, { query: { refresh: true } })
    expect(readOnlyResponse.status, readOnlyResponse.text).toBe(200)
    expect(readOnlyResponse.json).toEqual(response.json)
    expect(await graphCalls(setup.accessToken, since)).toHaveLength(2)

    // A full token's refresh=true reads Graph again.
    await graphScript("GET", "/me/adaccounts", { status: 200, json: { data: [{ id: "act_3", name: "Third" }] } })
    const refreshed = await setup.api.get(path, { query: { refresh: true } })
    expect(refreshed.json).toEqual({ data: [{ id: "act_3", name: "Third" }] })
    expect(await graphCalls(setup.accessToken, since)).toHaveLength(3)
  })

  it("answers 409 messagingAdsReconnectRequired without an active connection", async () => {
    const none = await messengerAdsSetup({ label: "ad-accounts-none", connect: false })
    expectError(
      await none.api.get(`/v1/ads/campaigns/messenger/${none.integrationId}/ad-accounts`),
      409,
      "messagingAdsReconnectRequired",
      RECONNECT_MESSAGE,
    )
    const invalid = await messengerAdsSetup({ label: "ad-accounts-invalid", status: "invalid" })
    expectError(
      await invalid.api.get(`/v1/ads/campaigns/messenger/${invalid.integrationId}/ad-accounts`),
      409,
      "messagingAdsReconnectRequired",
    )
    expect(await graphCalls(invalid.accessToken)).toEqual([])
  })

  it("marks a connection with undecryptable auth invalid", async () => {
    const setup = await messengerAdsSetup({ label: "ad-accounts-corrupt", connect: false })
    const ext = await adsFixtures()
    await ext.connectMessagingAds({
      workspaceId: setup.tenant.workspaceId,
      channel: "messenger",
      integrationId: setup.integrationId,
      accessToken: setup.accessToken,
      corruptAuth: true,
    })
    expectError(
      await setup.api.get(`/v1/ads/campaigns/messenger/${setup.integrationId}/ad-accounts`),
      409,
      "messagingAdsReconnectRequired",
    )
    const prerequisites = await setup.api.get("/v1/ads/campaigns/prerequisites", {
      query: { channel: "messenger", integrationId: setup.integrationId },
    })
    expect(prerequisites.json).toEqual({ connected: false, reconnectNeeded: true })
  })

  it("marks the connection invalid on a Graph 190 error", async () => {
    const setup = await messengerAdsSetup({ label: "ad-accounts-190" })
    await graphScript("GET", "/me/adaccounts", graphError(400, 190, "Error validating access token: Session has expired"), {
      times: 1,
    })
    const response = await setup.api.get(`/v1/ads/campaigns/messenger/${setup.integrationId}/ad-accounts`)
    expect(response.status, response.text).toBe(400)
    expect(response.json).toMatchObject({ code: "BAD_REQUEST", status: 400 })
    const prerequisites = await setup.api.get("/v1/ads/campaigns/prerequisites", {
      query: { channel: "messenger", integrationId: setup.integrationId },
    })
    expect(prerequisites.json).toEqual({ connected: false, reconnectNeeded: true })
  })
})

describe("ads.getCampaignAdAccountDetails (GET /v1/ads/campaigns/ad-accounts/{adAccountId})", () => {
  it("maps the Graph ad account fields", async () => {
    const setup = await messengerAdsSetup({ label: "ad-account-details" })
    const adAccountId = uniqueAdAccountId()
    const since = await fake.cursor()
    await graphScript("GET", `/${adAccountId}`, {
      status: 200,
      json: {
        id: adAccountId,
        name: "Parity account",
        currency: "EUR",
        timezone_name: "Europe/Berlin",
        account_status: 1,
        min_daily_budget: 100,
      },
    })
    const response = await setup.api.get(`/v1/ads/campaigns/ad-accounts/${adAccountId}`, {
      query: { channel: "messenger", integrationId: setup.integrationId },
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      id: adAccountId,
      name: "Parity account",
      currency: "EUR",
      timezoneName: "Europe/Berlin",
      accountStatus: 1,
      minDailyBudgetMinorUnits: 100,
    })
    const calls = await graphCalls(setup.accessToken, since)
    expect(calls.map((call) => ({ method: call.method, path: call.path, query: call.query }))).toEqual([
      {
        method: "GET",
        path: `/v23.0/${adAccountId}`,
        query: {
          fields: "id,name,currency,timezone_name,account_status,min_daily_budget",
          access_token: setup.accessToken,
        },
      },
    ])
  })

  it("omits optional fields Meta does not send", async () => {
    const setup = await messengerAdsSetup({ label: "ad-account-details-min" })
    const adAccountId = uniqueAdAccountId()
    await graphScript("GET", `/${adAccountId}`, {
      status: 200,
      json: { id: adAccountId, currency: "USD", timezone_name: "UTC", account_status: 2 },
    })
    const response = await setup.api.get(`/v1/ads/campaigns/ad-accounts/${adAccountId}`, {
      query: { channel: "messenger", integrationId: setup.integrationId },
    })
    expect(response.json).toEqual({ id: adAccountId, currency: "USD", timezoneName: "UTC", accountStatus: 2 })
  })

  it("rejects a bad ad account id or a missing integration with 422; 409 without connection", async () => {
    const setup = await messengerAdsSetup({ label: "ad-account-details-422", connect: false })
    expectValidationIssue(
      await setup.api.get("/v1/ads/campaigns/ad-accounts/123", {
        query: { channel: "messenger", integrationId: setup.integrationId },
      }),
      ["adAccountId"],
    )
    expectValidationIssue(
      await setup.api.get("/v1/ads/campaigns/ad-accounts/act_1", { query: { channel: "messenger" } }),
      ["integrationId"],
    )
    expectError(
      await setup.api.get("/v1/ads/campaigns/ad-accounts/act_1", {
        query: { channel: "messenger", integrationId: setup.integrationId },
      }),
      409,
      "messagingAdsReconnectRequired",
    )
  })
})

describe("ads.createCampaignImageUpload (POST /v1/ads/campaigns/upload-image)", () => {
  it("returns a presigned PUT URL in the workspace's creative prefix; the bytes are then public", async () => {
    const normalizer = new Normalizer()
    const setup = await messengerAdsSetup({ label: "image-upload", connect: false })
    const prefix = `public/space/${setup.tenant.workspaceId}/ads-campaign/creatives/`
    const response = await setup.api.post<{ fileId: string; imageKey: string; presignedPostUrl: string; publicUrl: string }>(
      "/v1/ads/campaigns/upload-image",
      { fileName: "banner.png", mimeType: "image/png", fileSize: PNG_1X1.length },
    )
    expect(response.status, response.text).toBe(201)
    const { fileId, imageKey, presignedPostUrl, publicUrl } = response.json
    expect(Object.keys(response.json).sort()).toEqual(["fileId", "imageKey", "presignedPostUrl", "publicUrl"])
    expect(normalizer.normalize({ fileId })).toEqual({ fileId: "<id:1>" })
    expect(imageKey).toMatch(new RegExp(`^${prefix}\\d{15,20}\\.png$`))
    expect(publicUrl).toBe(`http://app/storage/${imageKey}`)
    expect(() => new URL(presignedPostUrl)).not.toThrow()

    const put = await fetch(presignedPostUrl, { method: "PUT", body: PNG_1X1 })
    expect(put.status).toBe(200)
    const stored = await new HttpClient().get(publicUrl.replace("http://app", ""))
    expect(stored.status).toBe(200)

    const jpeg = await setup.api.post<{ imageKey: string }>("/v1/ads/campaigns/upload-image", {
      fileName: "Photo.JPEG",
      mimeType: "image/jpeg",
      fileSize: 10 * 1024 * 1024,
    })
    expect(jpeg.status, jpeg.text).toBe(201)
    expect(jpeg.json.imageKey).toMatch(/\.jpeg$/)
  })

  it("rejects unsupported types, mismatched extensions and oversize files", async () => {
    const setup = await messengerAdsSetup({ label: "image-upload-err", connect: false })
    expectError(
      await setup.api.post("/v1/ads/campaigns/upload-image", { fileName: "banner.png", mimeType: "image/jpeg", fileSize: 10 }),
      400,
      "adsCreativeUnsupportedImage",
      "Unsupported ad image: banner.png (image/jpeg). Use JPEG, PNG, GIF or WebP.",
    )
    expectError(
      await setup.api.post("/v1/ads/campaigns/upload-image", { fileName: "icon.svg", mimeType: "image/svg+xml", fileSize: 10 }),
      400,
      "adsCreativeUnsupportedImage",
    )
    expectError(
      await setup.api.post("/v1/ads/campaigns/upload-image", { fileName: "noext", mimeType: "image/png", fileSize: 10 }),
      400,
      "adsCreativeUnsupportedImage",
    )
    expectError(
      await setup.api.post("/v1/ads/campaigns/upload-image", {
        fileName: "big.png",
        mimeType: "image/png",
        fileSize: 10 * 1024 * 1024 + 1,
      }),
      400,
      "adsCreativeImageTooLarge",
      "Ad images are limited to 10485760 bytes.",
    )
    expectValidationIssue(
      await setup.api.post("/v1/ads/campaigns/upload-image", { fileName: "a.png", mimeType: "image/png", fileSize: 0 }),
      ["fileSize"],
    )
    expectValidationIssue(
      await setup.api.post("/v1/ads/campaigns/upload-image", { fileName: "", mimeType: "image/png", fileSize: 1 }),
      ["fileName"],
    )
  })
})

describe("ads.uploadCampaignVideo (POST /v1/ads/campaigns/upload-video)", () => {
  it("uploads the decoded bytes as multipart to Graph advideos and returns the video id", async () => {
    const setup = await messengerAdsSetup({ label: "video-upload" })
    const adAccountId = uniqueAdAccountId()
    const videoId = uniqueDigits()
    const since = await fake.cursor()
    await graphScript("POST", `/${adAccountId}/advideos`, { status: 200, json: { id: videoId } })
    const bytes = Buffer.from(`parity-video-bytes-${videoId}`)

    const response = await setup.api.post("/v1/ads/campaigns/upload-video", {
      channel: "messenger",
      integrationId: setup.integrationId,
      adAccountId,
      fileName: "clip.mp4",
      mimeType: "video/mp4",
      base64: bytes.toString("base64"),
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ videoId })

    const calls = await graphCalls(setup.accessToken, since)
    expect(calls).toHaveLength(1)
    const [call] = calls
    expect(call).toMatchObject({ method: "POST", path: `/v23.0/${adAccountId}/advideos`, query: { access_token: setup.accessToken } })
    expect(call?.headers["content-type"]).toMatch(/^multipart\/form-data; boundary=/)
    const body = Buffer.from(call?.bodyBase64 ?? "", "base64").toString("utf8")
    expect(body).toContain('name="source"; filename="clip.mp4"')
    expect(body).toContain("Content-Type: video/mp4")
    expect(body).toContain(bytes.toString("utf8"))
  })

  it("rejects bad input with 422 and answers 409 without a connection", async () => {
    const setup = await messengerAdsSetup({ label: "video-upload-err", connect: false })
    const body = {
      channel: "messenger",
      integrationId: setup.integrationId,
      adAccountId: "act_1",
      fileName: "clip.mp4",
      mimeType: "video/mp4",
      base64: "AAAA",
    }
    expectValidationIssue(await setup.api.post("/v1/ads/campaigns/upload-video", { ...body, mimeType: "video/webm" }), [
      "mimeType",
    ])
    expectValidationIssue(await setup.api.post("/v1/ads/campaigns/upload-video", { ...body, adAccountId: "1" }), [
      "adAccountId",
    ])
    expectValidationIssue(await setup.api.post("/v1/ads/campaigns/upload-video", { ...body, base64: "" }), ["base64"])
    expectError(await setup.api.post("/v1/ads/campaigns/upload-video", body), 409, "messagingAdsReconnectRequired")
  })
})

describe("ads.getCampaignVideoStatus (GET /v1/ads/campaigns/videos/{videoId}/status)", () => {
  it("maps Meta's video_status to isReady/isError", async () => {
    const setup = await messengerAdsSetup({ label: "video-status" })
    const query = { channel: "messenger", integrationId: setup.integrationId }
    const since = await fake.cursor()
    const ready = uniqueDigits()
    const processing = uniqueDigits()
    const failed = uniqueDigits()
    await graphScript("GET", `/${ready}`, { status: 200, json: { id: ready, status: { video_status: "ready" } } })
    await graphScript("GET", `/${processing}`, { status: 200, json: { id: processing } })
    await graphScript("GET", `/${failed}`, { status: 200, json: { id: failed, status: { video_status: "error" } } })

    const readyResponse = await setup.api.get(`/v1/ads/campaigns/videos/${ready}/status`, { query })
    expect(readyResponse.status, readyResponse.text).toBe(200)
    expect(readyResponse.json).toEqual({ videoId: ready, status: "ready", isReady: true, isError: false })
    const processingResponse = await setup.api.get(`/v1/ads/campaigns/videos/${processing}/status`, { query })
    expect(processingResponse.json).toEqual({ videoId: processing, status: "processing", isReady: false, isError: false })
    const failedResponse = await setup.api.get(`/v1/ads/campaigns/videos/${failed}/status`, { query })
    expect(failedResponse.json).toEqual({ videoId: failed, status: "error", isReady: false, isError: true })

    const calls = await graphCalls(setup.accessToken, since)
    expect(calls.map((call) => ({ path: call.path, query: call.query }))).toEqual([
      { path: `/v23.0/${ready}`, query: { fields: "status", access_token: setup.accessToken } },
      { path: `/v23.0/${processing}`, query: { fields: "status", access_token: setup.accessToken } },
      { path: `/v23.0/${failed}`, query: { fields: "status", access_token: setup.accessToken } },
    ])
  })

  it("answers 422 without channel/integrationId and 409 without a connection", async () => {
    const setup = await messengerAdsSetup({ label: "video-status-err", connect: false })
    expectValidationIssue(await setup.api.get("/v1/ads/campaigns/videos/123/status", { query: { channel: "messenger" } }), [
      "integrationId",
    ])
    expectError(
      await setup.api.get("/v1/ads/campaigns/videos/123/status", {
        query: { channel: "messenger", integrationId: setup.integrationId },
      }),
      409,
      "messagingAdsReconnectRequired",
    )
  })
})

describe("ads.listCampaignMessengerPages (GET /v1/ads/campaigns/messenger-pages)", () => {
  it("lists every connected Messenger Page for the whatsapp channel", async () => {
    const normalizer = new Normalizer()
    const setup = await messengerAdsSetup({ label: "messenger-pages", connect: false })
    const ext = await adsFixtures()
    const secondPage = uniqueDigits()
    const second = await ext.createMessengerChannel({
      workspaceId: setup.tenant.workspaceId,
      pageId: secondPage,
      name: "Second page",
    })
    const response = await setup.api.get("/v1/ads/campaigns/messenger-pages", { query: { channel: "whatsapp" } })
    expect(response.status, response.text).toBe(200)
    expect(
      normalizer.normalize(response.json, {
        keep: ["pageId"],
        replace: { [setup.integrationId]: "<first>", [second.integrationId]: "<second>" },
      }),
    ).toEqual({
      data: [
        { id: "<first>", name: `Parity Page ${setup.pageId}`, pageId: setup.pageId },
        { id: "<second>", name: "Second page", pageId: secondPage },
      ],
    })
    // integrationId is accepted and ignored.
    const withId = await setup.api.get("/v1/ads/campaigns/messenger-pages", {
      query: { channel: "whatsapp", integrationId: setup.integrationId },
    })
    expect(withId.json).toEqual(response.json)
  })

  it("answers 400 invalidRequest for another channel and 422 for a bad one", async () => {
    const setup = await messengerAdsSetup({ label: "messenger-pages-err", connect: false })
    expectError(
      await setup.api.get("/v1/ads/campaigns/messenger-pages", { query: { channel: "messenger" } }),
      400,
      "invalidRequest",
      "Messenger pages are only listed for the WhatsApp channel",
    )
    expectValidationIssue(await setup.api.get("/v1/ads/campaigns/messenger-pages", { query: { channel: "sms" } }), [
      "channel",
    ])
  })
})
