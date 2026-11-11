// botSimulator.getLink

import { describe, expect, it } from "vitest"
import { createWebchat, errorShape, expectError, expectInvalid, newTenant, patchWebchat } from "./helpers.ts"

describe("botSimulator.getLink", () => {
  it("returns a /bs link on the app URL for an allowed website", async () => {
    const { tenant, api } = await newTenant("bs-link")
    const webchat = await createWebchat(api, { authorizedDomains: ["example.com"] })
    const response = await api.get<{ url: string }>("/v1/bot-simulator/link", {
      query: { webchatId: webchat.id, websiteUrl: "  https://shop.example.com/pricing?a=1  " },
    })
    expect(response.status, response.text).toBe(200)
    expect(Object.keys(response.json)).toEqual(["url"])
    const url = new URL(response.json.url)
    expect(`${url.origin}${url.pathname}`).toBe(`http://app/bs/${tenant.workspaceId}/${webchat.id}`)
    expect([...url.searchParams.entries()]).toEqual([["url", "https://shop.example.com/pricing?a=1"]])
  })

  it("allows any website when the webchat has no allowed domains", async () => {
    const { api } = await newTenant("bs-link-any")
    const webchat = await createWebchat(api, { authorizedDomains: [] })
    const response = await api.get<{ url: string }>("/v1/bot-simulator/link", {
      query: { webchatId: webchat.id, websiteUrl: "http://anything.parity.test/" },
    })
    expect(response.status, response.text).toBe(200)
    expect(new URL(response.json.url).searchParams.get("url")).toBe("http://anything.parity.test/")
  })

  it("rejects a website off the allowed domains, a non-http URL and a disabled webchat (422)", async () => {
    const { api } = await newTenant("bs-link-invalid")
    const webchat = await createWebchat(api, { authorizedDomains: ["example.com"] })
    const patched = await patchWebchat(api, webchat.id, { authorizedDomains: ["example.com"] })
    expect(patched.authorizedDomains).toEqual(["example.com"])
    const offDomain = await api.get("/v1/bot-simulator/link", {
      query: { webchatId: webchat.id, websiteUrl: "https://other.parity.test/" },
    })
    const body = expectError(offDomain, 422, "validation")
    expect(body.message).toBe("Website is not on the webchat's allowed domains: example.com")
    expect(errorShape(offDomain)).toMatchSnapshot("off domain")

    const notHttp = await api.get("/v1/bot-simulator/link", {
      query: { webchatId: webchat.id, websiteUrl: "javascript:alert(1)" },
    })
    expect(expectError(notHttp, 422, "validation").message).toBe("Website URL must be an absolute http(s) URL")
    const relative = await api.get("/v1/bot-simulator/link", { query: { webchatId: webchat.id, websiteUrl: "/pricing" } })
    expectError(relative, 422, "validation")

    const disabled = await createWebchat(api, { name: "Disabled webchat", authorizedDomains: [] })
    expect((await patchWebchat(api, disabled.id, { enable: false })).enable).toBe(false)
    const off = await api.get("/v1/bot-simulator/link", {
      query: { webchatId: disabled.id, websiteUrl: "https://example.com/" },
    })
    expect(expectError(off, 422, "validation").message).toBe("Webchat is disabled")
  })

  it("validates the query and is 404 for an unknown webchat", async () => {
    const { api } = await newTenant("bs-link-404")
    expectInvalid(await api.get("/v1/bot-simulator/link", { query: { websiteUrl: "https://example.com/" } }), "webchatId")
    expectInvalid(await api.get("/v1/bot-simulator/link", { query: { webchatId: "1" } }), "websiteUrl")
    expectInvalid(
      await api.get("/v1/bot-simulator/link", { query: { webchatId: "abc", websiteUrl: "https://example.com/" } }),
      "webchatId",
    )
    expectInvalid(
      await api.get("/v1/bot-simulator/link", { query: { webchatId: "1", websiteUrl: `https://example.com/${"a".repeat(2000)}` } }),
      "websiteUrl",
    )
    const unknown = await api.get("/v1/bot-simulator/link", {
      query: { webchatId: "1", websiteUrl: "https://example.com/" },
    })
    expect(unknown.status).toBe(404)
    expect(errorShape(unknown)).toMatchSnapshot("unknown webchat")
  })
})
