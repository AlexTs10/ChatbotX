// Messaging-ad campaign lifecycle on Meta (fake Graph): ads.createCampaign,
// ads.retryCampaign, ads.publishCampaign, ads.pauseCampaign,
// ads.deleteCampaign, ads.listCampaigns, ads.getCampaignsInsights.

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { createTenant, fixtures } from "../../src/fixtures/index.ts"
import { apiClient, HttpClient } from "../../src/http.ts"
import { Normalizer } from "../../src/normalize.ts"
import {
  adsFixtures,
  callSummary,
  expectError,
  expectValidationIssue,
  graphCalls,
  graphError,
  graphScript,
  jsonBody,
  MISSING_ID,
  type MessengerAdsSetup,
  messengerAdsSetup,
  PNG_1X1,
  uniqueAccessToken,
  uniqueAdAccountId,
  uniqueDigits,
} from "./ads-helpers.ts"

type Operation = Record<string, unknown> & {
  id: string
  createState: string
  publishState: string
  metaCampaignId: string | null
  metaAdSetId: string | null
  metaAdCreativeId: string | null
  metaAdId: string | null
}

type MetaIds = { campaignId: string; adSetId: string; creativeId: string; adId: string }

const newMetaIds = (): MetaIds => ({
  campaignId: uniqueDigits(),
  adSetId: uniqueDigits(),
  creativeId: uniqueDigits(),
  adId: uniqueDigits(),
})

const videoCampaign = (setup: MessengerAdsSetup, adAccountId: string, extra: Record<string, unknown> = {}) => ({
  channel: "messenger",
  integrationId: setup.integrationId,
  adAccountId,
  name: "Parity campaign",
  campaign: { specialAdCategories: ["NONE"] },
  adSet: { dailyBudgetMinorUnits: 500, targeting: { countries: ["US", "DE"], ageMin: 21, ageMax: 40 } },
  creative: {
    media: { kind: "video", videoId: "778899", title: "Watch", message: "Hello there" },
    welcomeMessage: { type: "single", message: "Hi! How can we help?", quickReplies: [{ title: "Prices" }] },
  },
  ...extra,
})

/** Scripts the four create steps; each answers once. */
const scriptCreateChain = async (adAccountId: string, ids: MetaIds) => {
  await graphScript("POST", `/${adAccountId}/campaigns`, { status: 200, json: { id: ids.campaignId } }, { times: 1 })
  await graphScript("POST", `/${adAccountId}/adsets`, { status: 200, json: { id: ids.adSetId } }, { times: 1 })
  await graphScript("POST", `/${adAccountId}/adcreatives`, { status: 200, json: { id: ids.creativeId } }, { times: 1 })
  await graphScript("POST", `/${adAccountId}/ads`, { status: 200, json: { id: ids.adId } }, { times: 1 })
}

/** Status updates (`POST /<id>` with {status}) for every level answer `{success:true}`. */
const scriptStatusUpdates = async (ids: MetaIds) => {
  for (const id of [ids.campaignId, ids.adSetId, ids.adId]) {
    await graphScript("POST", `/${id}`, { status: 200, json: { success: true } })
  }
}

const createdCampaign = async (label: string) => {
  const setup = await messengerAdsSetup({ label })
  const adAccountId = uniqueAdAccountId()
  const ids = newMetaIds()
  await scriptCreateChain(adAccountId, ids)
  const created = await setup.api.post<Operation>("/v1/ads/campaigns", videoCampaign(setup, adAccountId))
  expect(created.status, created.text).toBe(201)
  return { setup, adAccountId, ids, operation: created.json }
}

const statusBodies = (records: Awaited<ReturnType<typeof graphCalls>>) =>
  records.map((record) => ({ path: record.path, status: jsonBody(record).status }))

describe("ads.createCampaign (POST /v1/ads/campaigns)", () => {
  it("creates campaign, ad set, creative and ad (all PAUSED) and returns the draft", async () => {
    const normalizer = new Normalizer()
    const setup = await messengerAdsSetup({ label: "campaign-create" })
    const adAccountId = uniqueAdAccountId()
    const ids = newMetaIds()
    const since = await fake.cursor()
    await scriptCreateChain(adAccountId, ids)

    const response = await setup.api.post<Operation>("/v1/ads/campaigns", videoCampaign(setup, adAccountId))
    expect(response.status, response.text).toBe(201)
    expect(normalizer.normalize(response.json, { keep: ["metaCampaignId", "metaAdSetId", "metaAdCreativeId", "metaAdId"] })).toEqual({
      id: "<id:1>",
      channel: "messenger",
      adAccountId,
      name: "Parity campaign",
      createState: "adCreated",
      publishState: "draft",
      metaCampaignId: ids.campaignId,
      metaAdSetId: ids.adSetId,
      metaAdCreativeId: ids.creativeId,
      metaAdId: ids.adId,
      lastError: null,
      cleanupError: null,
      effectiveStatus: null,
      createdAt: "<ts>",
    })

    const calls = await graphCalls(setup.accessToken, since)
    expect(callSummary(calls)).toEqual([
      `POST /v23.0/${adAccountId}/campaigns`,
      `POST /v23.0/${adAccountId}/adsets`,
      `POST /v23.0/${adAccountId}/adcreatives`,
      `POST /v23.0/${adAccountId}/ads`,
    ])
    for (const call of calls) {
      expect(call.headers["content-type"]).toMatch(/^application\/json/)
    }
    expect(jsonBody(calls[0])).toEqual({
      access_token: setup.accessToken,
      name: "Parity campaign",
      objective: "OUTCOME_ENGAGEMENT",
      buying_type: "AUCTION",
      special_ad_categories: ["NONE"],
      is_adset_budget_sharing_enabled: false,
      status: "PAUSED",
    })
    expect(jsonBody(calls[1])).toEqual({
      access_token: setup.accessToken,
      name: "Parity campaign",
      campaign_id: ids.campaignId,
      daily_budget: 500,
      billing_event: "IMPRESSIONS",
      optimization_goal: "CONVERSATIONS",
      bid_strategy: "LOWEST_COST_WITHOUT_CAP",
      destination_type: "MESSENGER",
      promoted_object: { page_id: setup.pageId },
      targeting: {
        geo_locations: { countries: ["US", "DE"] },
        targeting_automation: { advantage_audience: 0 },
        age_min: 21,
        age_max: 40,
      },
      status: "PAUSED",
    })
    const creative = jsonBody(calls[2]) as { object_story_spec: { video_data: { page_welcome_message: string } } }
    expect(creative).toEqual({
      access_token: setup.accessToken,
      name: "Parity campaign",
      object_story_spec: {
        page_id: setup.pageId,
        video_data: {
          video_id: "778899",
          title: "Watch",
          message: "Hello there",
          call_to_action: { type: "MESSAGE_PAGE", value: { app_destination: "MESSENGER" } },
          page_welcome_message: expect.any(String),
        },
      },
    })
    expect(JSON.parse(creative.object_story_spec.video_data.page_welcome_message)).toEqual({
      type: "VISUAL_EDITOR",
      version: 1,
      landing_screen_type: "welcome_message",
      media_type: "text",
      text_format: {
        customer_action_type: "VISUAL_EDITOR_CUSTOMER_ACTIONS",
        content: { title: "Hi! How can we help?", quick_replies: [{ title: "Prices" }] },
      },
    })
    expect(jsonBody(calls[3])).toEqual({
      access_token: setup.accessToken,
      name: "Parity campaign",
      adset_id: ids.adSetId,
      creative: { creative_id: ids.creativeId },
      status: "PAUSED",
    })
  })

  it("strips age targeting for a restricted special ad category and sends the country list", async () => {
    const setup = await messengerAdsSetup({ label: "campaign-housing" })
    const adAccountId = uniqueAdAccountId()
    const ids = newMetaIds()
    const since = await fake.cursor()
    await scriptCreateChain(adAccountId, ids)
    const response = await setup.api.post<Operation>(
      "/v1/ads/campaigns",
      videoCampaign(setup, adAccountId, {
        campaign: { specialAdCategories: ["HOUSING", "NONE"], specialAdCategoryCountry: ["US"] },
        adSet: {
          dailyBudgetMinorUnits: "750",
          targeting: { countries: ["US"], ageMin: 30, genders: [1], locales: [1033] },
          startTime: "2030-01-01T00:00:00Z",
          endTime: "2030-02-01T00:00:00Z",
        },
        creative: { media: { kind: "video", videoId: "1" }, welcomeMessage: { type: "default" } },
      }),
    )
    expect(response.status, response.text).toBe(201)
    const calls = await graphCalls(setup.accessToken, since)
    expect(jsonBody(calls[0])).toMatchObject({
      special_ad_categories: ["HOUSING"],
      special_ad_category_country: ["US"],
    })
    const adSet = jsonBody(calls[1])
    expect(adSet).toMatchObject({ daily_budget: 750, start_time: "2030-01-01T00:00:00Z", end_time: "2030-02-01T00:00:00Z" })
    expect(adSet.targeting).toEqual({
      geo_locations: { countries: ["US"] },
      targeting_automation: { advantage_audience: 0 },
      locales: [1033],
    })
    expect(jsonBody(calls[2]).object_story_spec).toEqual({
      page_id: setup.pageId,
      video_data: { video_id: "1", call_to_action: { type: "MESSAGE_PAGE", value: { app_destination: "MESSENGER" } } },
    })
  })

  it("creates an image creative from an uploaded image (preflight + adimages upload)", async () => {
    const setup = await messengerAdsSetup({ label: "campaign-image" })
    const upload = await setup.api.post<{ fileId: string; imageKey: string; presignedPostUrl: string; publicUrl: string }>(
      "/v1/ads/campaigns/upload-image",
      { fileName: "banner.png", mimeType: "image/png", fileSize: PNG_1X1.length },
    )
    expect(upload.status, upload.text).toBe(201)
    expect((await fetch(upload.json.presignedPostUrl, { method: "PUT", body: PNG_1X1 })).status).toBe(200)

    const adAccountId = uniqueAdAccountId()
    const ids = newMetaIds()
    const imageFile = `${upload.json.fileId}.png`
    const since = await fake.cursor()
    await scriptCreateChain(adAccountId, ids)
    await graphScript("POST", `/${adAccountId}/adimages`, {
      status: 200,
      json: { images: { [imageFile]: { hash: "parityhash123", url: "https://scontent.fbcdn.net/x.png" } } },
    })
    const response = await setup.api.post<Operation>(
      "/v1/ads/campaigns",
      videoCampaign(setup, adAccountId, {
        creative: {
          media: {
            kind: "image",
            imageKey: upload.json.imageKey,
            fileId: upload.json.fileId,
            link: "https://shop.parity.test/offer",
            message: "Buy now",
            headline: "Offer",
            description: "Best price",
            caption: "parity.test",
          },
          welcomeMessage: { type: "templates", templates: [{ heading: "Hey", message: "Pick one" }, { message: "Or this" }] },
        },
      }),
    )
    expect(response.status, response.text).toBe(201)
    expect(response.json.createState).toBe("adCreated")

    const calls = await graphCalls(setup.accessToken, since)
    expect(callSummary(calls)).toEqual([
      `POST /v23.0/${adAccountId}/campaigns`,
      `POST /v23.0/${adAccountId}/adsets`,
      `POST /v23.0/${adAccountId}/adimages`,
      `POST /v23.0/${adAccountId}/adcreatives`,
      `POST /v23.0/${adAccountId}/ads`,
    ])
    const imageUpload = calls[2]
    expect(imageUpload?.query).toEqual({ access_token: setup.accessToken })
    expect(imageUpload?.headers["content-type"]).toMatch(/^multipart\/form-data; boundary=/)
    const multipart = Buffer.from(imageUpload?.bodyBase64 ?? "", "base64")
    expect(multipart.toString("latin1")).toContain(`name="${imageFile}"; filename="${imageFile}"`)
    expect(multipart.toString("latin1")).toContain("Content-Type: image/png")
    expect(multipart.includes(PNG_1X1)).toBe(true)

    const creative = jsonBody(calls[3]) as {
      object_story_spec: { link_data: Record<string, unknown> & { page_welcome_message: string } }
    }
    expect(creative.object_story_spec.link_data).toEqual({
      link: "https://shop.parity.test/offer",
      image_hash: "parityhash123",
      message: "Buy now",
      name: "Offer",
      description: "Best price",
      caption: "parity.test",
      call_to_action: { type: "MESSAGE_PAGE", value: { app_destination: "MESSENGER" } },
      page_welcome_message: expect.any(String),
    })
    expect(JSON.parse(creative.object_story_spec.link_data.page_welcome_message)).toEqual([
      {
        type: "VISUAL_EDITOR",
        version: 1,
        landing_screen_type: "welcome_message",
        media_type: "text",
        text_format: {
          customer_action_type: "VISUAL_EDITOR_CUSTOMER_ACTIONS",
          content: { title: "Hey", message: "Pick one" },
        },
      },
      {
        type: "VISUAL_EDITOR",
        version: 1,
        landing_screen_type: "welcome_message",
        media_type: "text",
        text_format: { customer_action_type: "VISUAL_EDITOR_CUSTOMER_ACTIONS", content: { message: "Or this" } },
      },
    ])
  })

  it("rejects an image that was never uploaded before any Graph call (400 invalidRequest)", async () => {
    const setup = await messengerAdsSetup({ label: "campaign-image-missing" })
    const upload = await setup.api.post<{ fileId: string; imageKey: string }>("/v1/ads/campaigns/upload-image", {
      fileName: "banner.png",
      mimeType: "image/png",
      fileSize: 10,
    })
    const since = await fake.cursor()
    const response = await setup.api.post<Operation>(
      "/v1/ads/campaigns",
      videoCampaign(setup, uniqueAdAccountId(), {
        creative: {
          media: { kind: "image", imageKey: upload.json.imageKey, fileId: upload.json.fileId, link: "https://parity.test/" },
          welcomeMessage: { type: "default" },
        },
      }),
    )
    expectError(response, 400, "invalidRequest", "This image could not be found in storage.")
    expect(await graphCalls(setup.accessToken, since)).toEqual([])
    const list = await setup.api.get<{ data: Operation[] }>("/v1/ads/campaigns", {
      query: { channel: "messenger", integrationId: setup.integrationId },
    })
    expect(list.json.data).toHaveLength(1)
    expect(list.json.data[0]).toMatchObject({
      createState: "failed",
      lastError: "This image could not be found in storage.",
      metaCampaignId: null,
    })
  })

  it("rejects invalid campaign input with 422 invalidRequestData", async () => {
    const setup = await messengerAdsSetup({ label: "campaign-422" })
    const adAccountId = uniqueAdAccountId()
    const body = videoCampaign(setup, adAccountId)
    expectValidationIssue(await setup.api.post("/v1/ads/campaigns", { ...body, adAccountId: "12345" }), ["adAccountId"])
    expectValidationIssue(await setup.api.post("/v1/ads/campaigns", { ...body, name: "" }), ["name"])
    expectValidationIssue(
      await setup.api.post("/v1/ads/campaigns", { ...body, campaign: { specialAdCategories: ["CREDIT"] } }),
      ["campaign", "specialAdCategories"],
    )
    expectValidationIssue(
      await setup.api.post("/v1/ads/campaigns", { ...body, campaign: { specialAdCategories: ["ISSUES_ELECTIONS_POLITICS"] } }),
      ["campaign", "specialAdCategoryCountry"],
    )
    expectValidationIssue(
      await setup.api.post("/v1/ads/campaigns", {
        ...body,
        adSet: { ...body.adSet, startTime: "2030-02-01T00:00:00Z", endTime: "2030-01-01T00:00:00Z" },
      }),
      ["adSet", "endTime"],
    )
    expectValidationIssue(
      await setup.api.post("/v1/ads/campaigns", { ...body, adSet: { ...body.adSet, targeting: { countries: ["US"], ageMin: 50, ageMax: 30 } } }),
      ["adSet", "targeting", "ageMax"],
    )
    expectValidationIssue(
      await setup.api.post("/v1/ads/campaigns", {
        ...body,
        creative: {
          media: { kind: "image", imageKey: "public/space/1/ads-campaign/creatives/x.png", fileId: "1", link: "https://parity.test/" },
          welcomeMessage: { type: "default" },
        },
      }),
      ["creative", "media", "imageKey"],
    )
    expect(await graphCalls(setup.accessToken)).toEqual([])
  })

  it("answers 404 for an unknown integration and 409 without a connection (the draft row stays)", async () => {
    const setup = await messengerAdsSetup({ label: "campaign-404" })
    expectError(
      await setup.api.post("/v1/ads/campaigns", { ...videoCampaign(setup, uniqueAdAccountId()), integrationId: MISSING_ID }),
      404,
      "notFound",
      "Messenger integration not found",
    )

    const unconnected = await messengerAdsSetup({ label: "campaign-409", connect: false })
    expectError(
      await unconnected.api.post("/v1/ads/campaigns", videoCampaign(unconnected, uniqueAdAccountId())),
      409,
      "messagingAdsReconnectRequired",
    )
    const list = await unconnected.api.get<{ data: Operation[] }>("/v1/ads/campaigns", {
      query: { channel: "messenger", integrationId: unconnected.integrationId },
    })
    expect(list.json.data).toHaveLength(1)
    expect(list.json.data[0]).toMatchObject({ createState: "pending", publishState: "draft", metaCampaignId: null })
  })
})

describe("ads.retryCampaign (POST /v1/ads/campaigns/{id}/retry)", () => {
  it("resumes a failed draft from the persisted step without re-creating the campaign", async () => {
    const setup = await messengerAdsSetup({ label: "campaign-retry" })
    const adAccountId = uniqueAdAccountId()
    const ids = newMetaIds()
    const since = await fake.cursor()
    await graphScript("POST", `/${adAccountId}/campaigns`, { status: 200, json: { id: ids.campaignId } }, { times: 1 })
    await graphScript("POST", `/${adAccountId}/adsets`, graphError(400, 100, "Invalid parameter"), { times: 1 })

    const failed = await setup.api.post("/v1/ads/campaigns", videoCampaign(setup, adAccountId))
    expect(failed.status, failed.text).toBe(400)
    expect(failed.json).toMatchObject({ code: "BAD_REQUEST", status: 400, message: "(#100) Invalid parameter" })

    const list = await setup.api.get<{ data: Operation[] }>("/v1/ads/campaigns", {
      query: { channel: "messenger", integrationId: setup.integrationId },
    })
    expect(list.json.data).toHaveLength(1)
    const draft = list.json.data[0] as Operation
    expect(draft).toMatchObject({
      createState: "failed",
      publishState: "draft",
      metaCampaignId: ids.campaignId,
      metaAdSetId: null,
      lastError: "(#100) Invalid parameter",
    })

    await graphScript("POST", `/${adAccountId}/adsets`, { status: 200, json: { id: ids.adSetId } }, { times: 1 })
    await graphScript("POST", `/${adAccountId}/adcreatives`, { status: 200, json: { id: ids.creativeId } }, { times: 1 })
    await graphScript("POST", `/${adAccountId}/ads`, { status: 200, json: { id: ids.adId } }, { times: 1 })
    const retried = await setup.api.post<Operation>(`/v1/ads/campaigns/${draft.id}/retry`, {})
    expect(retried.status, retried.text).toBe(200)
    expect(retried.json).toMatchObject({
      id: draft.id,
      createState: "adCreated",
      publishState: "draft",
      metaCampaignId: ids.campaignId,
      metaAdSetId: ids.adSetId,
      metaAdCreativeId: ids.creativeId,
      metaAdId: ids.adId,
      lastError: null,
      effectiveStatus: null,
    })
    expect(callSummary(await graphCalls(setup.accessToken, since))).toEqual([
      `POST /v23.0/${adAccountId}/campaigns`,
      `POST /v23.0/${adAccountId}/adsets`,
      `POST /v23.0/${adAccountId}/adsets`,
      `POST /v23.0/${adAccountId}/adcreatives`,
      `POST /v23.0/${adAccountId}/ads`,
    ])

    // Not failed any more: 409.
    expectError(
      await setup.api.post(`/v1/ads/campaigns/${draft.id}/retry`, {}),
      409,
      "messagingAdNotRetryable",
      "This ad is already being retried or is not in a retryable state.",
    )
  })

  it("answers 404 for an unknown or foreign operation and 422 for a bad id", async () => {
    const { setup, operation } = await createdCampaign("campaign-retry-404")
    expectError(await setup.api.post(`/v1/ads/campaigns/${MISSING_ID}/retry`, {}), 404, "notFound", "Messaging ad operation not found")
    const other = await messengerAdsSetup({ label: "campaign-retry-other" })
    expectError(await other.api.post(`/v1/ads/campaigns/${operation.id}/retry`, {}), 404, "notFound")
    expectValidationIssue(await setup.api.post("/v1/ads/campaigns/abc/retry", {}), ["id"])
  })
})

describe("ads.publishCampaign (POST /v1/ads/campaigns/{id}/publish)", () => {
  it("activates campaign, ad set and ad in that order", async () => {
    const { setup, ids, operation } = await createdCampaign("campaign-publish")
    const since = await fake.cursor()
    await scriptStatusUpdates(ids)
    const response = await setup.api.post<Operation>(`/v1/ads/campaigns/${operation.id}/publish`, {})
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ ...operation, publishState: "published" })
    expect(statusBodies(await graphCalls(setup.accessToken, since))).toEqual([
      { path: `/v23.0/${ids.campaignId}`, status: "ACTIVE" },
      { path: `/v23.0/${ids.adSetId}`, status: "ACTIVE" },
      { path: `/v23.0/${ids.adId}`, status: "ACTIVE" },
    ])
  })

  it("pauses the activated levels again when Meta refuses the ad (200 publishFailed)", async () => {
    const { setup, ids, operation } = await createdCampaign("campaign-publish-fail")
    const since = await fake.cursor()
    await scriptStatusUpdates(ids)
    await graphScript("POST", `/${ids.adId}`, graphError(400, 100, "Ad is under review"))
    const response = await setup.api.post<Operation>(`/v1/ads/campaigns/${operation.id}/publish`, {})
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      ...operation,
      publishState: "publishFailed",
      lastError: "(#100) Ad is under review",
      cleanupError: null,
    })
    expect(statusBodies(await graphCalls(setup.accessToken, since))).toEqual([
      { path: `/v23.0/${ids.campaignId}`, status: "ACTIVE" },
      { path: `/v23.0/${ids.adSetId}`, status: "ACTIVE" },
      { path: `/v23.0/${ids.adId}`, status: "ACTIVE" },
      { path: `/v23.0/${ids.adSetId}`, status: "PAUSED" },
      { path: `/v23.0/${ids.campaignId}`, status: "PAUSED" },
    ])
  })

  it("records a failed compensation in cleanupError", async () => {
    const { setup, ids, operation } = await createdCampaign("campaign-publish-cleanup")
    await graphScript("POST", `/${ids.campaignId}`, { status: 200, json: { success: true } }, { bodyIncludes: "ACTIVE" })
    await graphScript("POST", `/${ids.campaignId}`, graphError(400, 2, "Service temporarily unavailable"), {
      bodyIncludes: "PAUSED",
    })
    await graphScript("POST", `/${ids.adSetId}`, graphError(400, 100, "Budget too low"))
    const response = await setup.api.post<Operation>(`/v1/ads/campaigns/${operation.id}/publish`, {})
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({
      publishState: "publishFailed",
      lastError: "(#100) Budget too low",
      cleanupError: `campaign(${ids.campaignId}): (#2) Service temporarily unavailable`,
    })
  })

  it("answers 409 before all levels exist and 404 for an unknown id", async () => {
    const setup = await messengerAdsSetup({ label: "campaign-publish-409" })
    const adAccountId = uniqueAdAccountId()
    await graphScript("POST", `/${adAccountId}/campaigns`, graphError(400, 100, "Invalid parameter"), { times: 1 })
    expect((await setup.api.post("/v1/ads/campaigns", videoCampaign(setup, adAccountId))).status).toBe(400)
    const list = await setup.api.get<{ data: Operation[] }>("/v1/ads/campaigns", {
      query: { channel: "messenger", integrationId: setup.integrationId },
    })
    const draft = list.json.data[0] as Operation
    expectError(
      await setup.api.post(`/v1/ads/campaigns/${draft.id}/publish`, {}),
      409,
      "messagingAdNotPublishable",
      "Cannot publish before campaign/ad set/ad have all been created",
    )
    expectError(await setup.api.post(`/v1/ads/campaigns/${MISSING_ID}/publish`, {}), 404, "notFound")
  })
})

describe("ads.pauseCampaign (POST /v1/ads/campaigns/{id}/pause)", () => {
  it("pauses ad, ad set and campaign", async () => {
    const { setup, ids, operation } = await createdCampaign("campaign-pause")
    const since = await fake.cursor()
    await scriptStatusUpdates(ids)
    const response = await setup.api.post<Operation>(`/v1/ads/campaigns/${operation.id}/pause`, {})
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ ...operation, publishState: "paused", lastError: null, cleanupError: null })
    expect(statusBodies(await graphCalls(setup.accessToken, since))).toEqual([
      { path: `/v23.0/${ids.adId}`, status: "PAUSED" },
      { path: `/v23.0/${ids.adSetId}`, status: "PAUSED" },
      { path: `/v23.0/${ids.campaignId}`, status: "PAUSED" },
    ])
  })

  it("is best effort: a refused level is reported and the state is still paused", async () => {
    const { setup, ids, operation } = await createdCampaign("campaign-pause-fail")
    await scriptStatusUpdates(ids)
    await graphScript("POST", `/${ids.adSetId}`, graphError(400, 100, "Cannot pause"))
    const response = await setup.api.post<Operation>(`/v1/ads/campaigns/${operation.id}/pause`, {})
    expect(response.status, response.text).toBe(200)
    const reason = `adSet(${ids.adSetId}): (#100) Cannot pause`
    expect(response.json).toMatchObject({ publishState: "paused", lastError: reason, cleanupError: reason })
    expectError(await setup.api.post(`/v1/ads/campaigns/${MISSING_ID}/pause`, {}), 404, "notFound")
  })
})

describe("ads.deleteCampaign (DELETE /v1/ads/campaigns/{id})", () => {
  it("sets every level to DELETED on Meta and keeps the record as deleted (200)", async () => {
    const { setup, ids, operation } = await createdCampaign("campaign-delete")
    const since = await fake.cursor()
    await scriptStatusUpdates(ids)
    const response = await setup.api.delete<Operation>(`/v1/ads/campaigns/${operation.id}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({ ...operation, publishState: "deleted", cleanupError: null })
    expect(statusBodies(await graphCalls(setup.accessToken, since))).toEqual([
      { path: `/v23.0/${ids.adId}`, status: "DELETED" },
      { path: `/v23.0/${ids.adSetId}`, status: "DELETED" },
      { path: `/v23.0/${ids.campaignId}`, status: "DELETED" },
    ])
    await graphScript("GET", "", {
      status: 200,
      json: { [ids.adId]: { id: ids.adId, status: "DELETED", effective_status: "DELETED" } },
    })
    const list = await setup.api.get<{ data: Operation[] }>("/v1/ads/campaigns", {
      query: { channel: "messenger", integrationId: setup.integrationId },
    })
    expect(list.json.data.map((row) => [row.id, row.publishState, row.effectiveStatus])).toEqual([
      [operation.id, "deleted", "DELETED"],
    ])
  })

  it("stays deleting with cleanupError when Meta refuses a level; 404 for an unknown id", async () => {
    const { setup, ids, operation } = await createdCampaign("campaign-delete-fail")
    await scriptStatusUpdates(ids)
    await graphScript("POST", `/${ids.campaignId}`, graphError(400, 100, "Cannot delete"))
    const response = await setup.api.delete<Operation>(`/v1/ads/campaigns/${operation.id}`)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({
      publishState: "deleting",
      cleanupError: `campaign(${ids.campaignId}): (#100) Cannot delete`,
    })
    expectError(await setup.api.delete(`/v1/ads/campaigns/${MISSING_ID}`), 404, "notFound")
  })

  it("deletes the stored creative image", async () => {
    const setup = await messengerAdsSetup({ label: "campaign-delete-image" })
    const upload = await setup.api.post<{ fileId: string; imageKey: string; presignedPostUrl: string; publicUrl: string }>(
      "/v1/ads/campaigns/upload-image",
      { fileName: "banner.png", mimeType: "image/png", fileSize: PNG_1X1.length },
    )
    expect((await fetch(upload.json.presignedPostUrl, { method: "PUT", body: PNG_1X1 })).status).toBe(200)
    const adAccountId = uniqueAdAccountId()
    const ids = newMetaIds()
    await scriptCreateChain(adAccountId, ids)
    await graphScript("POST", `/${adAccountId}/adimages`, {
      status: 200,
      json: { images: { [`${upload.json.fileId}.png`]: { hash: "h1" } } },
    })
    const created = await setup.api.post<Operation>(
      "/v1/ads/campaigns",
      videoCampaign(setup, adAccountId, {
        creative: {
          media: { kind: "image", imageKey: upload.json.imageKey, fileId: upload.json.fileId, link: "https://parity.test/" },
          welcomeMessage: { type: "default" },
        },
      }),
    )
    expect(created.status, created.text).toBe(201)
    const publicPath = upload.json.publicUrl.replace("http://app", "")
    expect((await new HttpClient().get(publicPath)).status).toBe(200)

    await scriptStatusUpdates(ids)
    const deleted = await setup.api.delete<Operation>(`/v1/ads/campaigns/${created.json.id}`)
    expect(deleted.json).toMatchObject({ publishState: "deleted", cleanupError: null })
    expect((await new HttpClient().get(publicPath)).status).not.toBe(200)
  })
})

describe("ads.listCampaigns (GET /v1/ads/campaigns)", () => {
  it("lists the integration's ads newest first with Meta's effective_status", async () => {
    const normalizer = new Normalizer()
    const { setup, adAccountId, ids, operation } = await createdCampaign("campaign-list")
    // A second, failed draft without a Meta ad.
    await graphScript("POST", `/${adAccountId}/campaigns`, graphError(400, 100, "Invalid parameter"), { times: 1 })
    expect((await setup.api.post("/v1/ads/campaigns", videoCampaign(setup, adAccountId, { name: "Second" }))).status).toBe(400)

    const since = await fake.cursor()
    await graphScript("GET", "", {
      status: 200,
      json: { [ids.adId]: { id: ids.adId, name: "Parity campaign", status: "PAUSED", effective_status: "IN_PROCESS" } },
    })
    const query = { channel: "messenger", integrationId: setup.integrationId }
    const response = await setup.api.get<{ data: Operation[] }>("/v1/ads/campaigns", { query })
    expect(response.status, response.text).toBe(200)
    expect(
      normalizer.normalize(response.json, { keep: ["metaCampaignId", "metaAdSetId", "metaAdCreativeId", "metaAdId"] }),
    ).toEqual({
      data: [
        {
          id: "<id:1>",
          channel: "messenger",
          adAccountId,
          name: "Second",
          createState: "failed",
          publishState: "draft",
          metaCampaignId: null,
          metaAdSetId: null,
          metaAdCreativeId: null,
          metaAdId: null,
          lastError: "(#100) Invalid parameter",
          cleanupError: null,
          effectiveStatus: null,
          createdAt: "<ts>",
        },
        {
          ...normalizer.normalize(operation, { keep: ["metaCampaignId", "metaAdSetId", "metaAdCreativeId", "metaAdId"] }) as object,
          effectiveStatus: "IN_PROCESS",
        },
      ],
    })
    const calls = await graphCalls(setup.accessToken, since)
    expect(calls.map((call) => ({ method: call.method, path: call.path, query: call.query }))).toEqual([
      {
        method: "GET",
        path: "/v23.0",
        query: { ids: ids.adId, fields: "id,name,status,effective_status", access_token: setup.accessToken },
      },
    ])

    // Cached; refresh=true reads Meta again.
    await setup.api.get("/v1/ads/campaigns", { query })
    expect(await graphCalls(setup.accessToken, since)).toHaveLength(1)
    await setup.api.get("/v1/ads/campaigns", { query: { ...query, refresh: true } })
    expect(await graphCalls(setup.accessToken, since)).toHaveLength(2)
  })

  it("degrades to effectiveStatus null when the status read fails", async () => {
    const { setup, operation } = await createdCampaign("campaign-list-fail")
    await graphScript("GET", "", graphError(400, 100, "Unsupported get request"))
    const response = await setup.api.get<{ data: Operation[] }>("/v1/ads/campaigns", {
      query: { channel: "messenger", integrationId: setup.integrationId },
    })
    expect(response.status, response.text).toBe(200)
    expect(response.json.data).toEqual([{ ...operation, effectiveStatus: null }])
  })

  it("is empty for another integration and needs channel + integrationId", async () => {
    const setup = await messengerAdsSetup({ label: "campaign-list-empty" })
    const response = await setup.api.get("/v1/ads/campaigns", {
      query: { channel: "messenger", integrationId: setup.integrationId },
    })
    expect(response.json).toEqual({ data: [] })
    expectValidationIssue(await setup.api.get("/v1/ads/campaigns", { query: { channel: "messenger" } }), ["integrationId"])
    expectValidationIssue(
      await setup.api.get("/v1/ads/campaigns", { query: { integrationId: setup.integrationId } }),
      ["channel"],
    )
  })
})

describe("ads.getCampaignsInsights (POST /v1/ads/campaigns/insights)", () => {
  it("reads insights only for the workspace's own ads, also with a read-only token", async () => {
    const { setup, adAccountId, ids } = await createdCampaign("campaign-insights")
    const since = await fake.cursor()
    await graphScript("GET", `/${adAccountId}/insights`, {
      status: 200,
      json: {
        data: [
          {
            ad_id: ids.adId,
            account_currency: "USD",
            impressions: "1200",
            reach: "900",
            spend: "12.34",
            clicks: "40",
            actions: [
              { action_type: "link_click", value: "40" },
              { action_type: "onsite_conversion.messaging_conversation_started_7d", value: "8" },
            ],
            cost_per_action_type: [{ action_type: "onsite_conversion.messaging_conversation_started_7d", value: "1.5425" }],
          },
        ],
      },
    })
    const body = {
      channel: "messenger",
      integrationId: setup.integrationId,
      adAccountId,
      adIds: [ids.adId, "123456789"],
      datePreset: "last_7d",
    }
    const response = await setup.api.post("/v1/ads/campaigns/insights", body)
    expect(response.status, response.text).toBe(200)
    expect(response.json).toEqual({
      data: [
        {
          adId: ids.adId,
          currency: "USD",
          impressions: 1200,
          reach: 900,
          spend: 12.34,
          clicks: 40,
          conversations: 8,
          costPerConversation: 1.5425,
        },
      ],
    })
    const calls = await graphCalls(setup.accessToken, since)
    expect(calls.map((call) => ({ method: call.method, path: call.path, query: call.query }))).toEqual([
      {
        method: "GET",
        path: `/v23.0/${adAccountId}/insights`,
        query: {
          level: "ad",
          fields: "ad_id,account_currency,impressions,reach,spend,clicks,actions,cost_per_action_type",
          date_preset: "last_7d",
          filtering: JSON.stringify([{ field: "ad.id", operator: "IN", value: [ids.adId] }]),
          limit: "499",
          access_token: setup.accessToken,
        },
      },
    ])

    const driver = await fixtures()
    const readOnly = await driver.createWorkspaceToken({
      workspaceId: setup.tenant.workspaceId,
      name: "ro",
      permission: "read_only",
      scopes: null,
    })
    const readOnlyResponse = await apiClient(readOnly.token).post("/v1/ads/campaigns/insights", { ...body, refresh: true })
    expect(readOnlyResponse.status, readOnlyResponse.text).toBe(200)
    expect(readOnlyResponse.json).toEqual(response.json)
    // Cached, and refresh is ignored for read-only tokens.
    expect(await graphCalls(setup.accessToken, since)).toHaveLength(1)
  })

  it("returns [] without a Graph call when no requested ad belongs to the workspace", async () => {
    const { setup, adAccountId } = await createdCampaign("campaign-insights-foreign")
    const since = await fake.cursor()
    const foreign = await setup.api.post("/v1/ads/campaigns/insights", {
      channel: "messenger",
      integrationId: setup.integrationId,
      adAccountId,
      adIds: ["123456789"],
    })
    expect(foreign.status, foreign.text).toBe(200)
    expect(foreign.json).toEqual({ data: [] })
    const otherAccount = await setup.api.post("/v1/ads/campaigns/insights", {
      channel: "messenger",
      integrationId: setup.integrationId,
      adAccountId: uniqueAdAccountId(),
      adIds: ["123456789"],
    })
    expect(otherAccount.json).toEqual({ data: [] })
    expect(await graphCalls(setup.accessToken, since)).toEqual([])
  })

  it("rejects bad input with 422", async () => {
    const setup = await messengerAdsSetup({ label: "campaign-insights-422" })
    const body = { channel: "messenger", integrationId: setup.integrationId, adAccountId: "act_1", adIds: ["1"] }
    expectValidationIssue(await setup.api.post("/v1/ads/campaigns/insights", { ...body, adIds: [] }), ["adIds"])
    expectValidationIssue(
      await setup.api.post("/v1/ads/campaigns/insights", { ...body, adIds: Array.from({ length: 501 }, (_, i) => `${i + 1}`) }),
      ["adIds"],
    )
    expectValidationIssue(await setup.api.post("/v1/ads/campaigns/insights", { ...body, datePreset: "today" }), [
      "datePreset",
    ])
    expectValidationIssue(await setup.api.post("/v1/ads/campaigns/insights", { ...body, adAccountId: "1" }), ["adAccountId"])
  })
})

describe("ads.createCampaign channel variants", () => {
  it("whatsapp: needs a Messenger Page and sends the WhatsApp number in promoted_object", async () => {
    const tenant = await createTenant({ label: "campaign-whatsapp" })
    const ext = await adsFixtures()
    const phoneNumberId = uniqueDigits()
    const whatsapp = await ext.createWhatsappChannel({
      workspaceId: tenant.workspaceId,
      phoneNumberId,
      wabaId: `waba-${phoneNumberId}`,
      name: "WA",
      displayPhoneNumber: "+15550001111",
    })
    const pageId = uniqueDigits()
    const page = await ext.createMessengerChannel({ workspaceId: tenant.workspaceId, pageId, name: "Page" })
    const accessToken = uniqueAccessToken()
    await ext.connectMessagingAds({ workspaceId: tenant.workspaceId, channel: "whatsapp", integrationId: whatsapp.integrationId, accessToken })
    const api = apiClient(tenant.token)
    const adAccountId = uniqueAdAccountId()
    const body = {
      ...videoCampaign({ integrationId: whatsapp.integrationId } as MessengerAdsSetup, adAccountId),
      channel: "whatsapp",
    }
    expectError(
      await api.post("/v1/ads/campaigns", body),
      400,
      "messagingAdWhatsappPageRequired",
      "A connected Messenger Page must be selected to supply page_id for a WhatsApp messaging ad",
    )
    expectError(
      await api.post("/v1/ads/campaigns", { ...body, whatsappPageIntegrationId: MISSING_ID }),
      404,
      "notFound",
      "Messenger (Page) integration not found",
    )

    const ids = newMetaIds()
    const since = await fake.cursor()
    await scriptCreateChain(adAccountId, ids)
    const created = await api.post<Operation>("/v1/ads/campaigns", { ...body, whatsappPageIntegrationId: page.integrationId })
    expect(created.status, created.text).toBe(201)
    expect(created.json).toMatchObject({ channel: "whatsapp", createState: "adCreated" })
    const calls = await graphCalls(accessToken, since)
    expect(jsonBody(calls[1])).toMatchObject({
      destination_type: "WHATSAPP",
      promoted_object: { page_id: pageId, whatsapp_phone_number: "15550001111" },
    })
    expect(jsonBody(calls[2]).object_story_spec).toMatchObject({
      page_id: pageId,
      video_data: { call_to_action: { type: "WHATSAPP_MESSAGE", value: { app_destination: "WHATSAPP" } } },
    })
    const list = await api.get<{ data: Operation[] }>("/v1/ads/campaigns", {
      query: { channel: "whatsapp", integrationId: whatsapp.integrationId },
    })
    expect(list.json.data.map((row) => row.id)).toContain(created.json.id)
  })

  it("instagram: sends instagram_actor_id and INSTAGRAM_DIRECT", async () => {
    const tenant = await createTenant({ label: "campaign-instagram" })
    const ext = await adsFixtures()
    const igId = uniqueDigits()
    const pageId = uniqueDigits()
    const instagram = await ext.createInstagramChannel({
      workspaceId: tenant.workspaceId,
      igId,
      pageId,
      name: "IG",
      username: `ig_${igId}`,
    })
    const accessToken = uniqueAccessToken()
    await ext.connectMessagingAds({ workspaceId: tenant.workspaceId, channel: "instagram", integrationId: instagram.integrationId, accessToken })
    const adAccountId = uniqueAdAccountId()
    const ids = newMetaIds()
    const since = await fake.cursor()
    await scriptCreateChain(adAccountId, ids)
    const created = await apiClient(tenant.token).post<Operation>("/v1/ads/campaigns", {
      ...videoCampaign({ integrationId: instagram.integrationId } as MessengerAdsSetup, adAccountId),
      channel: "instagram",
    })
    expect(created.status, created.text).toBe(201)
    const calls = await graphCalls(accessToken, since)
    expect(jsonBody(calls[1])).toMatchObject({ destination_type: "INSTAGRAM_DIRECT", promoted_object: { page_id: pageId } })
    expect(jsonBody(calls[2]).object_story_spec).toMatchObject({
      page_id: pageId,
      instagram_actor_id: igId,
      video_data: { call_to_action: { type: "INSTAGRAM_MESSAGE", value: { app_destination: "INSTAGRAM_DIRECT" } } },
    })
  })
})
