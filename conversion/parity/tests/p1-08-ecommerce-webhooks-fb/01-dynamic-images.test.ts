// dynamicImages.* — templates whose static layers are rendered to a PNG
// background in storage on every save, plus the `/dynamic-images` trigger URL
// that each resource publishes (`imageUrl`). Rendered images are checked over
// HTTP: status, media type, PNG size (2x the canvas: supersampling) and single
// pixels of a solid-color source image served by the fake upstream.

import { describe, expect, it } from "vitest"
import { config } from "../../src/config.ts"
import { fake } from "../../src/fake.ts"
import { uniqueHost } from "../../src/unique.ts"
import {
  type Api,
  appClient,
  dohLookups,
  expectError,
  expectInvalid,
  ext,
  keysOf,
  messengerPage,
  qs,
  tenantApi,
  uniqueDigits,
  uniqueName,
  upstreamCalls,
} from "./helpers.ts"
import { decodePng, download, type Rgba, solidPng } from "./helpers-png.ts"

const RED: Rgba = [255, 0, 0, 255]
const BLUE: Rgba = [0, 0, 255, 255]
const TRANSPARENT: Rgba = [0, 0, 0, 0]

const RESOURCE_KEYS = [
  "backgroundUrl",
  "createdAt",
  "customFieldId",
  "data",
  "enabled",
  "id",
  "imageUrl",
  "name",
  "updatedAt",
]

type DynamicImage = {
  id: string
  name: string
  customFieldId: string | null
  data: { width: number; height: number; elements: Array<Record<string, unknown>> }
  backgroundUrl: string | null
  enabled: boolean
  imageUrl: string
  createdAt: string
  updatedAt: string
}

/** Scripts `GET https://<host>/<file>` to answer a solid-color PNG. */
const servePng = async (host: string, file: string, color: Rgba, size = 4) => {
  await fake.script({
    match: { host, method: "GET", path: `/${file}` },
    respond: { status: 200, headers: { "content-type": "image/png" }, base64: solidPng(size, size, color).toString("base64") },
  })
  return `https://${host}/${file}`
}

/** A 100x80 canvas with one fixed (`url`) image in the box (10,10)-(50,50). */
const urlImageDocument = (url: string) => ({
  width: 100,
  height: 80,
  elements: [{ id: "img-1", type: "image", imageType: "url", url, x: 10, y: 10, width: 40, height: 40 }],
})

const createImage = async (api: Api, body: Record<string, unknown>): Promise<DynamicImage> => {
  const response = await api.post<DynamicImage>("/v1/dynamic-images", body)
  expect(response.status, response.text).toBe(201)
  return response.json
}

const backgroundPrefix = (workspaceId: string, id: string) =>
  `${config.baseUrl}/storage/public/space/${workspaceId}/dynamic-images/${id}/background_`

const triggerUrl = (id: string) => `${config.baseUrl}/dynamic-images?dynamicImageId=${id}&userId={{user_id}}`

/** `GET /dynamic-images` (the trigger route, outside `/api`), no redirect follow. */
const trigger = (params: Record<string, string | undefined>) => appClient().get(`/dynamic-images${qs(params)}`)

describe("dynamicImages.create", () => {
  it("stores the template, renders the static background to storage and publishes the trigger URL", async () => {
    const { tenant, api } = await tenantApi("p108-di-create")
    const host = uniqueHost("img")
    const url = await servePng(host, "logo.png", RED)
    const name = uniqueName("Banner")
    const since = await fake.cursor()

    const created = await createImage(api, { name, data: urlImageDocument(url) })
    expect(keysOf(created)).toEqual(RESOURCE_KEYS)
    expect(created).toMatchObject({
      name,
      customFieldId: null,
      enabled: true,
      imageUrl: triggerUrl(created.id),
      // zod defaults are applied and stored: priority false, imageStyle square.
      data: {
        width: 100,
        height: 80,
        elements: [
          {
            id: "img-1",
            type: "image",
            imageType: "url",
            url,
            x: 10,
            y: 10,
            width: 40,
            height: 40,
            priority: false,
            imageStyle: "square",
          },
        ],
      },
    })
    expect(created.id).toMatch(/^\d{15,20}$/)
    expect(Date.parse(created.createdAt)).not.toBeNaN()
    expect(created.backgroundUrl ?? "").toMatch(
      new RegExp(`^${backgroundPrefix(tenant.workspaceId, created.id).replace(/[.?]/g, "\\$&")}\\d+\\.png$`),
    )

    // The static `url` image was fetched once at save time ...
    const fetched = await upstreamCalls({ host, since, path: /^\/logo\.png$/ })
    expect(fetched).toHaveLength(1)
    expect(fetched[0]).toMatchObject({ scheme: "https", method: "GET" })

    // ... and composited into a 200x160 PNG (2x supersampling) in storage.
    const background = await download(created.backgroundUrl ?? "")
    expect(background.status).toBe(200)
    expect(background.contentType).toBe("image/png")
    const png = decodePng(background.bytes)
    expect({ width: png.width, height: png.height }).toEqual({ width: 200, height: 160 })
    expect(png.pixel(60, 60)).toEqual(RED)
    expect(png.pixel(25, 25)).toEqual(RED)
    expect(png.pixel(5, 5)).toEqual(TRANSPARENT)
    expect(png.pixel(150, 120)).toEqual(TRANSPARENT)
  })

  it("renders text and QR code layers without variables into the background", async () => {
    const { api } = await tenantApi("p108-di-create-text")
    const created = await createImage(api, {
      name: uniqueName("Text"),
      data: {
        width: 120,
        height: 90,
        elements: [
          {
            id: "t-1",
            type: "text",
            text: "Hello",
            x: 0,
            y: 0,
            width: 120,
            height: 30,
            fontSize: 20,
            fontFamily: "roboto",
            align: "center",
            color: "#111111",
          },
          { id: "q-1", type: "qrCode", text: "parity", x: 30, y: 30, width: 64, height: 64, size: 64, color: "#000000" },
        ],
      },
    })
    expect(created.data.elements).toEqual([
      {
        id: "t-1",
        type: "text",
        text: "Hello",
        x: 0,
        y: 0,
        width: 120,
        height: 30,
        fontSize: 20,
        fontFamily: "roboto",
        align: "center",
        color: "#111111",
        priority: false,
        bold: false,
        italic: false,
        uppercase: false,
      },
      { id: "q-1", type: "qrCode", text: "parity", x: 30, y: 30, width: 64, height: 64, size: 64, color: "#000000", priority: false },
    ])
    const background = await download(created.backgroundUrl ?? "")
    expect(background.status).toBe(200)
    expect(background.contentType).toBe("image/png")
    const png = decodePng(background.bytes)
    expect({ width: png.width, height: png.height }).toEqual({ width: 240, height: 180 })
    // The QR code has a white quiet zone (margin 1) at its top-left corner.
    expect(png.pixel(62, 62)).toEqual([255, 255, 255, 255])
    // Below the text box and left of the QR code nothing is drawn.
    expect(png.pixel(10, 150)).toEqual(TRANSPARENT)
  })

  it("keeps a layer with a {{variable}} and every layer after it out of the background", async () => {
    const { api } = await tenantApi("p108-di-create-dynamic")
    const host = uniqueHost("img-dyn")
    const url = await servePng(host, "late.png", RED)
    const since = await fake.cursor()
    const created = await createImage(api, {
      name: uniqueName("Dynamic"),
      data: {
        width: 50,
        height: 50,
        elements: [
          {
            id: "t-1",
            type: "text",
            text: "Hi {{first_name}}",
            x: 0,
            y: 0,
            width: 50,
            height: 20,
            fontSize: 12,
            fontFamily: "arial",
            align: "left",
            color: "#000000",
          },
          // Static by itself, but drawn after a dynamic layer: not baked in.
          { id: "img-1", type: "image", imageType: "url", url, x: 0, y: 0, width: 50, height: 50 },
        ],
      },
    })
    expect(await upstreamCalls({ host, since })).toHaveLength(0)
    const png = decodePng((await download(created.backgroundUrl ?? "")).bytes)
    expect({ width: png.width, height: png.height }).toEqual({ width: 100, height: 100 })
    expect(png.pixel(50, 50)).toEqual(TRANSPARENT)
  })

  it("rejects an invalid template (422)", async () => {
    const { api } = await tenantApi("p108-di-create-invalid")
    const data = { width: 10, height: 10, elements: [] }
    expectInvalid(await api.post("/v1/dynamic-images", { name: "", data }), "name")
    expectInvalid(await api.post("/v1/dynamic-images", { name: "x".repeat(101), data }), "name")
    expectInvalid(await api.post("/v1/dynamic-images", { name: "n" }), "data")
    expectInvalid(await api.post("/v1/dynamic-images", { name: "n", data: { ...data, width: 0 } }), "data.width")
    expectInvalid(await api.post("/v1/dynamic-images", { name: "n", data: { ...data, height: 1.5 } }), "data.height")
    expectInvalid(
      await api.post("/v1/dynamic-images", {
        name: "n",
        data: { ...data, elements: [{ id: "x", type: "video", x: 0, y: 0, width: 1, height: 1 }] },
      }),
      "data.elements.0.type",
    )
    expectInvalid(
      await api.post("/v1/dynamic-images", {
        name: "n",
        data: {
          ...data,
          elements: [{ id: "q", type: "qrCode", text: "t", x: 0, y: 0, width: 1, height: 1, size: 63, color: "#000" }],
        },
      }),
      "data.elements.0.size",
    )
    expectInvalid(await api.post("/v1/dynamic-images", { name: "n", customFieldId: "abc", data }), "customFieldId")
  })

  it("answers 500 for a name already used in the workspace (unique index, not mapped)", async () => {
    const { api } = await tenantApi("p108-di-create-dup")
    const name = uniqueName("Same")
    const data = { width: 10, height: 10, elements: [] }
    await createImage(api, { name, data })
    expectError(await api.post("/v1/dynamic-images", { name, data }), 500, "INTERNAL_SERVER_ERROR")
  })
})

describe("dynamicImages.get / list", () => {
  it("returns one image; 404 for an unknown or foreign id; 422 for a non-numeric id", async () => {
    const { api } = await tenantApi("p108-di-get")
    const { api: other } = await tenantApi("p108-di-get-other")
    const created = await createImage(api, { name: uniqueName("Get"), data: { width: 10, height: 10, elements: [] } })
    const foreign = await createImage(other, { name: uniqueName("Foreign"), data: { width: 10, height: 10, elements: [] } })

    const got = await api.get<DynamicImage>(`/v1/dynamic-images/${created.id}`)
    expect(got.status).toBe(200)
    expect(got.json).toEqual(created)

    expectError(await api.get("/v1/dynamic-images/1"), 404, "notFound", {
      defined: true,
      message: "Dynamic image not found",
    })
    expectError(await api.get(`/v1/dynamic-images/${foreign.id}`), 404, "notFound")
    expectInvalid(await api.get("/v1/dynamic-images/abc"), "id")
  })

  it("lists the workspace's images, filters by name (case-insensitive substring) and pages", async () => {
    const { api } = await tenantApi("p108-di-list")
    const { api: other } = await tenantApi("p108-di-list-other")
    const tag = uniqueDigits()
    const data = { width: 10, height: 10, elements: [] }
    const a = await createImage(api, { name: `Alpha ${tag}`, data })
    const b = await createImage(api, { name: `beta ${tag}`, data })
    await createImage(other, { name: `Alpha ${tag}`, data })

    const all = await api.get<{ data: DynamicImage[]; pageCount: number }>("/v1/dynamic-images")
    expect(all.status).toBe(200)
    expect(all.json.pageCount).toBe(1)
    expect(all.json.data.map((row) => row.id).sort()).toEqual([a.id, b.id].sort())
    expect(all.json.data.find((row) => row.id === a.id)).toEqual(a)

    const filtered = await api.get<{ data: DynamicImage[]; pageCount: number }>(`/v1/dynamic-images${qs({ name: "ALPHA" })}`)
    expect(filtered.json).toEqual({ data: [a], pageCount: 1 })

    const none = await api.get(`/v1/dynamic-images${qs({ name: "zzz-none" })}`)
    expect(none.json).toEqual({ data: [], pageCount: 0 })

    const page1 = await api.get<{ data: DynamicImage[]; pageCount: number }>(`/v1/dynamic-images${qs({ perPage: 1, page: 1 })}`)
    const page2 = await api.get<{ data: DynamicImage[]; pageCount: number }>(`/v1/dynamic-images${qs({ perPage: "1", page: "2" })}`)
    expect(page1.json.pageCount).toBe(2)
    expect(page2.json.pageCount).toBe(2)
    expect([...page1.json.data, ...page2.json.data].map((row) => row.id).sort()).toEqual([a.id, b.id].sort())

    expectInvalid(await api.get(`/v1/dynamic-images${qs({ perPage: 51 })}`), "perPage")
    expectInvalid(await api.get(`/v1/dynamic-images${qs({ page: 0 })}`), "page")
  })
})

describe("dynamicImages.update", () => {
  it("replaces the template, writes a new background file and deletes the old one", async () => {
    const { tenant, api } = await tenantApi("p108-di-update")
    const host = uniqueHost("img-upd")
    const red = await servePng(host, "red.png", RED)
    const blue = await servePng(host, "blue.png", BLUE)
    const created = await createImage(api, { name: uniqueName("Before"), data: urlImageDocument(red) })
    const since = await fake.cursor()
    const name = uniqueName("After")

    const updated = await api.put<DynamicImage>(`/v1/dynamic-images/${created.id}`, {
      name,
      data: { width: 30, height: 30, elements: [{ id: "b", type: "image", imageType: "url", url: blue, x: 0, y: 0, width: 30, height: 30 }] },
    })
    expect(updated.status, updated.text).toBe(200)
    expect(keysOf(updated.json)).toEqual(RESOURCE_KEYS)
    expect(updated.json).toMatchObject({ id: created.id, name, customFieldId: null, enabled: true, imageUrl: created.imageUrl })
    expect(updated.json.data).toEqual({
      width: 30,
      height: 30,
      elements: [
        { id: "b", type: "image", imageType: "url", url: blue, x: 0, y: 0, width: 30, height: 30, priority: false, imageStyle: "square" },
      ],
    })
    expect(updated.json.backgroundUrl).not.toBe(created.backgroundUrl)
    expect(updated.json.backgroundUrl?.startsWith(backgroundPrefix(tenant.workspaceId, created.id))).toBe(true)
    expect(await upstreamCalls({ host, since, path: /^\/blue\.png$/ })).toHaveLength(1)

    expect((await download(created.backgroundUrl ?? "")).status).toBe(404)
    const png = decodePng((await download(updated.json.backgroundUrl ?? "")).bytes)
    expect({ width: png.width, height: png.height }).toEqual({ width: 60, height: 60 })
    expect(png.pixel(30, 30)).toEqual(BLUE)

    const got = await api.get(`/v1/dynamic-images/${created.id}`)
    expect(got.json).toEqual(updated.json)
  })

  it("is a full replace (name and data required); 404 for an unknown id", async () => {
    const { api } = await tenantApi("p108-di-update-errors")
    const created = await createImage(api, { name: uniqueName("Full"), data: { width: 10, height: 10, elements: [] } })
    expectInvalid(await api.put(`/v1/dynamic-images/${created.id}`, { name: "x" }), "data")
    expectInvalid(await api.put(`/v1/dynamic-images/${created.id}`, { data: { width: 1, height: 1, elements: [] } }), "name")
    expectError(
      await api.put("/v1/dynamic-images/1", { name: "x", data: { width: 1, height: 1, elements: [] } }),
      404,
      "notFound",
      { defined: true, message: "Dynamic image not found" },
    )
  })
})

describe("dynamicImages.setEnabled + the trigger URL", () => {
  it("disables and enables the trigger URL without touching the template", async () => {
    const { api } = await tenantApi("p108-di-enabled")
    const created = await createImage(api, { name: uniqueName("Toggle"), data: { width: 10, height: 10, elements: [] } })

    // Enabled, no userId: 302 to the static background.
    const open = await trigger({ dynamicImageId: created.id })
    expect(open.status).toBe(302)
    expect(open.headers.get("location")).toBe(created.backgroundUrl)

    const disabled = await api.patch<DynamicImage>(`/v1/dynamic-images/${created.id}/enabled`, { enabled: false })
    expect(disabled.status, disabled.text).toBe(200)
    expect(disabled.json).toMatchObject({ ...created, enabled: false, updatedAt: expect.any(String) })
    const blocked = await trigger({ dynamicImageId: created.id, userId: "anyone" })
    expect(blocked.status).toBe(404)
    expect(blocked.json).toEqual({ message: "Dynamic image is disabled" })

    const enabled = await api.patch<DynamicImage>(`/v1/dynamic-images/${created.id}/enabled`, { enabled: true })
    expect(enabled.json.enabled).toBe(true)
    expect(enabled.json.backgroundUrl).toBe(created.backgroundUrl)
    expect((await trigger({ dynamicImageId: created.id })).status).toBe(302)

    expectInvalid(await api.patch(`/v1/dynamic-images/${created.id}/enabled`, {}), "enabled")
    expectInvalid(await api.patch(`/v1/dynamic-images/${created.id}/enabled`, { enabled: "no" }), "enabled")
    expectError(await api.patch("/v1/dynamic-images/1/enabled", { enabled: true }), 404, "notFound", {
      message: "Dynamic image not found",
    })
  })

  it("answers 400 without dynamicImageId and 404 for an unknown one", async () => {
    const missing = await trigger({})
    expect(missing.status).toBe(400)
    expect(missing.json).toEqual({ message: "dynamicImageId is required" })
    const unknown = await trigger({ dynamicImageId: "1" })
    expect(unknown.status).toBe(404)
    expect(unknown.json).toEqual({ message: "Dynamic image not found" })
  })
})

describe("dynamicImages.delete", () => {
  it("deletes the template and its background file; 404 afterwards", async () => {
    const { api } = await tenantApi("p108-di-delete")
    const created = await createImage(api, { name: uniqueName("Gone"), data: { width: 10, height: 10, elements: [] } })
    expect((await download(created.backgroundUrl ?? "")).status).toBe(200)

    const deleted = await api.delete(`/v1/dynamic-images/${created.id}`)
    expect(deleted.status).toBe(204)
    expect(deleted.text).toBe("")

    expectError(await api.get(`/v1/dynamic-images/${created.id}`), 404, "notFound")
    expect((await download(created.backgroundUrl ?? "")).status).toBe(404)
    const gone = await trigger({ dynamicImageId: created.id })
    expect(gone.status).toBe(404)
    expect(gone.json).toEqual({ message: "Dynamic image not found" })
    expectError(await api.delete(`/v1/dynamic-images/${created.id}`), 404, "notFound", {
      defined: true,
      message: "Dynamic image not found",
    })
    expectInvalid(await api.delete("/v1/dynamic-images/abc"), "id")
  })
})

describe("dynamic image render per contact (trigger URL with userId)", () => {
  it("renders the contact's custom-field image through the SSRF guard, caches the URL and re-renders after an update", async () => {
    const { tenant, api } = await tenantApi("p108-di-contact")
    const fixtures = await ext()
    const host = uniqueHost("img-contact")
    const imageUrl = await servePng(host, "photo.png", RED)

    const field = await api.post<{ id: string }>("/v1/custom-fields", { name: uniqueName("photo_url"), type: "shortText" })
    expect(field.status, field.text).toBe(201)
    const cacheField = await api.post<{ id: string }>("/v1/custom-fields", { name: uniqueName("dyn_cache"), type: "shortText" })
    expect(cacheField.status, cacheField.text).toBe(201)

    const page = await messengerPage(tenant.workspaceId)
    const psid = uniqueDigits()
    const contact = await fixtures.createContact({
      workspaceId: tenant.workspaceId,
      inboxId: page.inboxId,
      channel: "messenger",
      sourceId: psid,
      firstName: "Ada",
    })
    await fixtures.setCustomFieldValue({ contactId: contact.contactId, customFieldId: field.json.id, value: imageUrl })

    const document = {
      width: 60,
      height: 40,
      elements: [{ id: "p", type: "image", imageType: "customField", customFieldId: field.json.id, x: 0, y: 0, width: 20, height: 20 }],
    }
    const created = await createImage(api, { name: uniqueName("Per contact"), customFieldId: cacheField.json.id, data: document })
    expect(created.customFieldId).toBe(cacheField.json.id)
    const version = /background_(\d+)\.png$/.exec(created.backgroundUrl ?? "")?.[1]
    expect(version).toMatch(/^\d+$/)
    const since = await fake.cursor()

    const first = await trigger({ dynamicImageId: created.id, userId: psid })
    expect(first.status).toBe(302)
    const location = first.headers.get("location") ?? ""
    expect(location).toBe(
      `${config.baseUrl}/storage/public/space/${tenant.workspaceId}/dynamic-images/${created.id}/contacts/${contact.contactId}.png?timestamp=${version}`,
    )
    // The custom-field URL passed the SSRF guard (DoH) and was fetched once.
    expect((await dohLookups(host, since)).length).toBeGreaterThanOrEqual(1)
    expect(await upstreamCalls({ host, since, path: /^\/photo\.png$/ })).toHaveLength(1)

    const rendered = await download(location)
    expect(rendered.status).toBe(200)
    expect(rendered.contentType).toBe("image/png")
    const png = decodePng(rendered.bytes)
    expect({ width: png.width, height: png.height }).toEqual({ width: 120, height: 80 })
    expect(png.pixel(20, 20)).toEqual(RED)
    expect(png.pixel(100, 60)).toEqual(TRANSPARENT)

    // Second visit: the URL cached on the contact's custom field, no new render.
    const second = await trigger({ dynamicImageId: created.id, userId: psid })
    expect(second.status).toBe(302)
    expect(second.headers.get("location")).toBe(location)
    expect(await upstreamCalls({ host, since, path: /^\/photo\.png$/ })).toHaveLength(1)

    // An unknown userId falls back to the static background.
    const stranger = await trigger({ dynamicImageId: created.id, userId: `x${uniqueDigits()}` })
    expect(stranger.status).toBe(302)
    expect(stranger.headers.get("location")).toBe(created.backgroundUrl)

    // An update clears the rendered images and the cached values: the next
    // visit renders again against the new background version.
    const updated = await api.put<DynamicImage>(`/v1/dynamic-images/${created.id}`, {
      name: created.name,
      customFieldId: cacheField.json.id,
      data: document,
    })
    expect(updated.status, updated.text).toBe(200)
    expect((await download(location)).status).toBe(404)
    const newVersion = /background_(\d+)\.png$/.exec(updated.json.backgroundUrl ?? "")?.[1]
    const third = await trigger({ dynamicImageId: created.id, userId: psid })
    expect(third.status).toBe(302)
    expect(third.headers.get("location")).toBe(location.replace(`timestamp=${version}`, `timestamp=${newVersion}`))
    expect(await upstreamCalls({ host, since, path: /^\/photo\.png$/ })).toHaveLength(2)
  })

  it("leaves out a custom-field image whose host resolves to a private address", async () => {
    const { tenant, api } = await tenantApi("p108-di-contact-ssrf")
    const fixtures = await ext()
    const host = uniqueHost("img-private")
    await fake.dns({ name: host, a: ["10.0.0.9"] })
    const field = await api.post<{ id: string }>("/v1/custom-fields", { name: uniqueName("photo_url"), type: "shortText" })
    expect(field.status, field.text).toBe(201)
    const page = await messengerPage(tenant.workspaceId)
    const psid = uniqueDigits()
    const contact = await fixtures.createContact({
      workspaceId: tenant.workspaceId,
      inboxId: page.inboxId,
      channel: "messenger",
      sourceId: psid,
      firstName: "Eve",
    })
    await fixtures.setCustomFieldValue({
      contactId: contact.contactId,
      customFieldId: field.json.id,
      value: `https://${host}/secret.png`,
    })
    const created = await createImage(api, {
      name: uniqueName("Private"),
      data: {
        width: 20,
        height: 20,
        elements: [{ id: "p", type: "image", imageType: "customField", customFieldId: field.json.id, x: 0, y: 0, width: 20, height: 20 }],
      },
    })
    const since = await fake.cursor()
    const response = await trigger({ dynamicImageId: created.id, userId: psid })
    expect(response.status).toBe(302)
    const location = response.headers.get("location") ?? ""
    expect(location).toContain(`/dynamic-images/${created.id}/contacts/${contact.contactId}.png?timestamp=`)
    expect(await upstreamCalls({ host, since })).toHaveLength(0)
    const png = decodePng((await download(location)).bytes)
    expect({ width: png.width, height: png.height }).toEqual({ width: 40, height: 40 })
    expect(png.pixel(20, 20)).toEqual(TRANSPARENT)
  })
})
