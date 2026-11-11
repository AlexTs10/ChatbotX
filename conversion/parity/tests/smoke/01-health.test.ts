import { describe, expect, it } from "vitest"
import { HttpClient } from "../../src/http.ts"
import { comparableHeaders, Normalizer } from "../../src/normalize.ts"

describe("smoke: health through the edge", () => {
  const http = new HttpClient()

  it("GET /api/health answers healthy", async () => {
    const normalizer = new Normalizer()
    const response = await http.get("/api/health")
    expect(response.status).toBe(200)
    expect(comparableHeaders(response.headers)["content-type"]).toBe("application/json")
    expect(normalizer.normalize(response.json)).toEqual({
      service: "builder",
      status: "healthy",
      timestamp: "<ts>",
    })
  })

  it("GET /ws/health and /ws/ready answer from realtime", async () => {
    const health = await http.get("/ws/health")
    expect(health.status).toBe(200)
    expect(health.text.trim()).toBe("ok")

    const ready = await http.get("/ws/ready")
    expect(ready.status).toBe(200)
    expect(ready.text.trim()).toBe("ready")
  })
})
