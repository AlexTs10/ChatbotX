// analytics.filterOptions: the connected, known-channel inboxes (not smtp)
// of the workspace, sorted by name.

import { describe, expect, it } from "vitest"
import { ext, tenantApi, uniqueName } from "./helpers.ts"

describe("analytics.filterOptions", () => {
  it("lists connected inboxes of known channels except smtp, by name", async () => {
    const { tenant, api } = await tenantApi("an-filter")
    const fx = await ext()
    const suffix = uniqueName("")
    const inbox = (name: string, channel: string, status?: "connected" | "disconnected") =>
      fx.createInbox({ workspaceId: tenant.workspaceId, name: `${name}${suffix}`, channel, status })
    const beta = await inbox("Beta", "telegram")
    const alpha = await inbox("Alpha", "messenger")
    await inbox("Gamma", "smtp")
    await inbox("Delta", "instagram", "disconnected")
    await inbox("Epsilon", "not-a-channel")
    const charlie = await inbox("Charlie", "whatsapp")

    const response = await api.get("/v1/analytics/filter-options")
    expect(response.status).toBe(200)
    expect(response.json).toEqual({
      data: [
        { id: alpha.inboxId, name: `Alpha${suffix}`, channel: "messenger" },
        { id: beta.inboxId, name: `Beta${suffix}`, channel: "telegram" },
        { id: charlie.inboxId, name: `Charlie${suffix}`, channel: "whatsapp" },
      ],
    })
  })

  it("is empty for a workspace without inboxes and ignores query input", async () => {
    const { api } = await tenantApi("an-filter-empty")
    const response = await api.get("/v1/analytics/filter-options", { query: { channels: "messenger" } })
    expect(response.status).toBe(200)
    expect(response.json).toEqual({ data: [] })
  })

  it("does not list another workspace's inboxes", async () => {
    const owner = await tenantApi("an-filter-owner")
    const other = await tenantApi("an-filter-other")
    await (await ext()).createInbox({ workspaceId: owner.tenant.workspaceId, name: uniqueName("Own"), channel: "messenger" })
    expect((await other.api.get("/v1/analytics/filter-options")).json).toEqual({ data: [] })
  })
})
