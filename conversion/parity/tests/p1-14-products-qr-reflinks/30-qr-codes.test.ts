// qrCodes.list / get / create / update / delete. QR codes are Reflink rows
// of type "qrCode" whose stored name is `qr_<name>`; create answers only the
// id. The list is cached per query (invalidated on every write).

import { describe, expect, it } from "vitest"
import { uniqueId } from "../../src/unique.ts"
import {
  type Api,
  createPublishedFlow,
  expectInvalid,
  expectNotFound,
  expectValidation,
  ISO,
  newTenant,
  SNOWFLAKE,
  sortQuery,
  UNKNOWN_ID,
} from "./helpers.ts"

type QrCode = Record<string, unknown> & {
  id: string
  name: string
  type: string
  flowId: string
  qrStyles: { size: number } | null
}

const qrName = () => `Qr${uniqueId()}`.slice(0, 20)

const create = async (api: Api, body: Record<string, unknown>): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/qr-codes", body)
  expect(response.status, response.text).toBe(201)
  expect(response.json).toEqual({ id: expect.stringMatching(SNOWFLAKE) })
  return response.json.id
}

const get = async (api: Api, id: string): Promise<QrCode> => {
  const response = await api.get<QrCode>(`/v1/qr-codes/${id}`)
  expect(response.status, response.text).toBe(200)
  return response.json
}

describe("qrCodes.create / get", () => {
  it("stores the name with a qr_ prefix and the size in qrStyles (600 by default)", async () => {
    const { api } = await newTenant("p114-qr-create")
    const flowId = await createPublishedFlow(api)
    const name = qrName()
    const id = await create(api, { name, flowId })
    const qr = await get(api, id)
    expect(qr).toMatchObject({
      id,
      name: `qr_${name}`,
      type: "qrCode",
      flowId,
      customFieldId: null,
      qrStyles: { size: 600 },
      createdAt: expect.stringMatching(ISO),
      updatedAt: expect.stringMatching(ISO),
    })
    const sized = await get(api, await create(api, { name: qrName(), flowId, size: 64 }))
    expect(sized.qrStyles).toEqual({ size: 64 })
  })

  it("422 for a taken name, a flow of another workspace and bad input; 404 unknown", async () => {
    const { api } = await newTenant("p114-qr-refused")
    const { api: other } = await newTenant("p114-qr-refused-other")
    const flowId = await createPublishedFlow(api)
    const foreignFlow = await createPublishedFlow(other)
    const name = qrName()
    await create(api, { name, flowId })
    expectValidation(await api.post("/v1/qr-codes", { name, flowId }), "QR Code name already exists")
    // A flow of another workspace fails the reference check first (404, undeclared).
    expectNotFound(await api.post("/v1/qr-codes", { name: qrName(), flowId: foreignFlow }), "Flow not found", false)
    expectInvalid(await api.post("/v1/qr-codes", { name: "has space", flowId }), "name")
    expectInvalid(await api.post("/v1/qr-codes", { name: "x".repeat(51), flowId }), "name")
    expectInvalid(await api.post("/v1/qr-codes", { name: qrName(), flowId, size: 63 }), "size")
    expectInvalid(await api.post("/v1/qr-codes", { name: qrName(), flowId, size: 1025 }), "size")
    expectInvalid(await api.post("/v1/qr-codes", { name: qrName() }), "flowId")
    expectNotFound(await api.get(`/v1/qr-codes/${UNKNOWN_ID}`), "QR Code not found")
  })
})

describe("qrCodes.list", () => {
  it("lists with the flow, pages with an explicit sort and filters by keyword", async () => {
    const { api } = await newTenant("p114-qr-list")
    const flowName = `QR flow ${uniqueId()}`
    const flowId = await createPublishedFlow(api, flowName)
    const ids: string[] = []
    const names: string[] = []
    for (let index = 0; index < 3; index += 1) {
      const name = `Promo${index}${uniqueId()}`.slice(0, 30)
      names.push(name)
      ids.push(await create(api, { name, flowId }))
    }
    const page1 = await api.get<{ data: Array<QrCode & { flow: Record<string, unknown> }>; pageCount: number }>(
      `/v1/qr-codes?${sortQuery("createdAt", true)}&perPage=2`,
    )
    expect(page1.status, page1.text).toBe(200)
    expect(page1.json.pageCount).toBe(2)
    expect(page1.json.data.map((qr) => qr.id)).toEqual([ids[2], ids[1]])
    expect(page1.json.data[0]).toMatchObject({
      id: ids[2],
      name: `qr_${names[2]}`,
      type: "qrCode",
      flowId,
      qrStyles: { size: 600 },
      flow: { id: flowId, name: flowName },
    })
    const page2 = await api.get<{ data: QrCode[] }>(`/v1/qr-codes?${sortQuery("createdAt", true)}&perPage=2&page=2`)
    expect(page2.json.data.map((qr) => qr.id)).toEqual([ids[0]])
    const ascending = await api.get<{ data: QrCode[] }>(`/v1/qr-codes?${sortQuery("createdAt", false)}`)
    expect(ascending.json.data.map((qr) => qr.id)).toEqual(ids)
    const filtered = await api.get<{ data: QrCode[] }>("/v1/qr-codes", { query: { keyword: names[1]?.toLowerCase() } })
    expect(filtered.json.data.map((qr) => qr.id)).toEqual([ids[1]])
    // A new QR code shows up at once (the cached list is invalidated).
    const fourth = await create(api, { name: qrName(), flowId })
    const again = await api.get<{ data: QrCode[] }>(`/v1/qr-codes?${sortQuery("createdAt", true)}&perPage=2`)
    expect(again.json.data[0]?.id).toBe(fourth)
  })

  it("does not list ref links, and ref links do not list QR codes", async () => {
    const { api } = await newTenant("p114-qr-separate")
    const flowId = await createPublishedFlow(api)
    const qrId = await create(api, { name: qrName(), flowId })
    const ref = await api.post<{ id: string }>("/v1/ref-links", { name: `Ref${uniqueId()}`.slice(0, 20), flowId, customFieldId: null })
    expect(ref.status, ref.text).toBe(201)
    const qrs = await api.get<{ data: QrCode[] }>("/v1/qr-codes")
    expect(qrs.json.data.map((qr) => qr.id)).toEqual([qrId])
    const refs = await api.get<{ data: QrCode[] }>("/v1/ref-links")
    expect(refs.json.data.map((reflink) => reflink.id)).toEqual([ref.json.id])
    expectNotFound(await api.get(`/v1/ref-links/${qrId}`), "Reflink not found")
    expectNotFound(await api.get(`/v1/qr-codes/${ref.json.id}`), "QR Code not found")
  })
})

describe("qrCodes.update / delete", () => {
  it("update renames (prefix kept), resizes and changes the flow", async () => {
    const { api } = await newTenant("p114-qr-update")
    const flowId = await createPublishedFlow(api)
    const otherFlow = await createPublishedFlow(api)
    const id = await create(api, { name: qrName(), flowId, size: 100 })
    const newName = qrName()
    const response = await api.patch<QrCode>(`/v1/qr-codes/${id}`, { name: newName, size: 200, flowId: otherFlow })
    expect(response.status, response.text).toBe(200)
    expect(response.json).toMatchObject({ id, name: `qr_${newName}`, flowId: otherFlow, qrStyles: { size: 200 } })
    const sizeOnly = await api.patch<QrCode>(`/v1/qr-codes/${id}`, { size: 300 })
    expect(sizeOnly.json).toMatchObject({ name: `qr_${newName}`, qrStyles: { size: 300 } })
    const taken = qrName()
    await create(api, { name: taken, flowId })
    expectValidation(await api.patch(`/v1/qr-codes/${id}`, { name: taken }), "QR Code name already exists")
    expectNotFound(await api.patch(`/v1/qr-codes/${UNKNOWN_ID}`, { size: 100 }), "QR Code not found")
    expectValidation(await api.patch(`/v1/qr-codes/${id}`, { flowId: UNKNOWN_ID }), "Flow not found in this workspace")
  })

  it("delete is 204 and then the QR code is gone; an unknown id is also 204", async () => {
    const { api } = await newTenant("p114-qr-delete")
    const flowId = await createPublishedFlow(api)
    const id = await create(api, { name: qrName(), flowId })
    const response = await api.delete(`/v1/qr-codes/${id}`)
    expect(response.status, response.text).toBe(204)
    expect(response.text).toBe("")
    expectNotFound(await api.get(`/v1/qr-codes/${id}`), "QR Code not found")
    expect((await api.get<{ data: unknown[] }>("/v1/qr-codes")).json.data).toEqual([])
    expect((await api.delete(`/v1/qr-codes/${UNKNOWN_ID}`)).status).toBe(204)
  })
})
