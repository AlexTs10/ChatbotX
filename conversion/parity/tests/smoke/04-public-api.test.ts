import { describe, expect, it } from "vitest"
import { createTenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { comparableHeaders, Normalizer } from "../../src/normalize.ts"

describe("smoke: public API", () => {
  it("GET /api/v1/workspace/settings returns the new workspace's settings", async () => {
    const normalizer = new Normalizer()
    const tenant = await createTenant({ label: "settings" })
    const response = await apiClient(tenant.token).get("/v1/workspace/settings")
    expect(response.status).toBe(200)
    expect(comparableHeaders(response.headers)).toMatchSnapshot("headers")
    expect(normalizer.normalize(response.json)).toMatchSnapshot("body")
  })

  it("GET /api/v1/workspace/settings without a token is 401", async () => {
    const response = await apiClient("").get("/v1/workspace/settings")
    expect(response.status).toBe(401)
  })
})
