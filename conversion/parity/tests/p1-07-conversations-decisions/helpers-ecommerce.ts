// Helpers of the P1.7 ecommerce tests (coupons, discountCodes). Not a test
// file. Setup goes through public routes only.

import { expect } from "vitest"
import { type Api, createWebchat, createWebchatContact, uniqueName } from "./helpers.ts"

export type Topic = {
  id: string
  workspaceId: string
  name: string
  description: string | null
  expiresAt: string | null
  status: "active" | "archived"
  hasEverHadCoupon: boolean
  createdAt: string
  updatedAt: string
}

export type IssuedCoupon = {
  id: string
  workspaceId: string
  topicId: string
  code: string
  issuedContactId: string | null
  issuedAt: string | null
  usedAt: string | null
  createdAt: string
  updatedAt: string
}

export type ListedCoupon = IssuedCoupon & {
  topicName: string
  issueStatus: "published" | "unpublished"
  usageStatus: "used" | "notUsed"
}

export type BulkResult = {
  processed: number
  created: number
  existing: number
  allowedRemaining: number
  currentCount: number
}

export const TOPIC_KEYS = [
  "createdAt",
  "description",
  "expiresAt",
  "hasEverHadCoupon",
  "id",
  "name",
  "status",
  "updatedAt",
  "workspaceId",
]

export const createTopic = async (
  api: Api,
  input: { name?: string; description?: string | null; expiresAt?: string | null } = {},
): Promise<Topic> => {
  const response = await api.post<Topic>("/v1/coupon-topics", { name: uniqueName("Topic"), ...input })
  expect(response.status, response.text).toBe(200)
  return response.json
}

export const bulkCreate = async (api: Api, topicId: string, codes: string[]): Promise<BulkResult> => {
  const response = await api.post<BulkResult>(`/v1/coupon-topics/${topicId}/coupons/bulk`, { codes })
  expect(response.status, response.text).toBe(200)
  return response.json
}

/** A webchat inbox per call plus one webchat contact; returns the contact id. */
export const newContact = async (api: Api, firstName = "Buyer"): Promise<string> => {
  const { inboxId } = await createWebchat(api, uniqueName("Shop chat"))
  return await createWebchatContact(api, inboxId, { firstName })
}

/** `n` webchat contacts on one inbox. */
export const newContacts = async (api: Api, n: number): Promise<string[]> => {
  const { inboxId } = await createWebchat(api, uniqueName("Shop chat"))
  const ids: string[] = []
  for (let index = 0; index < n; index += 1) {
    ids.push(await createWebchatContact(api, inboxId, { firstName: `Buyer${index}` }))
  }
  return ids
}

/** End of the given UTC day (workspace time zone UTC), as the service stores `expiresAt`. */
export const endOfUtcDay = (date: Date): string => `${date.toISOString().slice(0, 10)}T23:59:59.999Z`

export const unique = (prefix: string): string => uniqueName(prefix).replace(/[^A-Za-z0-9_-]/g, "")

/** A product made with the public `products.create` (setup only). */
export const createProduct = async (api: Api, input: { name?: string; price: number }): Promise<string> => {
  const response = await api.post<{ id: string }>("/v1/products", { name: input.name ?? uniqueName("Product"), price: input.price })
  expect(response.status, response.text).toBe(200)
  return response.json.id
}
