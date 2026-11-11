// Auth matrix of every P1.3 operation (49): no token, unknown token, a token
// without the operation's scope, a token with only that scope, and a
// read-only token (writes refused, reads let through). Auth runs before
// input validation, so the requests carry no valid input.

import { beforeAll, describe, expect, it } from "vitest"
import { createTenant, type Tenant } from "../../src/fixtures/index.ts"
import { apiClient } from "../../src/http.ts"
import { expectError } from "./helpers.ts"

type Scope = "integrations" | "automation" | "analytics"
type Op = { id: string; method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE"; path: string; scope: Scope }

const op = (id: string, method: Op["method"], path: string, scope: Scope): Op => ({ id, method, path, scope })

const OPERATIONS: Op[] = [
  op("aiHandover.getSettings", "GET", "/v1/inboxes/1/ai-handover/settings", "integrations"),
  op("aiHandover.listHistory", "GET", "/v1/inboxes/1/ai-handover/history", "integrations"),
  op("aiHandover.patchSettings", "PATCH", "/v1/inboxes/1/ai-handover/settings", "integrations"),
  op("aiHandover.retryApplyToAll", "POST", "/v1/inboxes/1/ai-handover/apply-to-all/retry", "integrations"),
  op("aiHandover.saveSettings", "PUT", "/v1/inboxes/1/ai-handover/settings", "integrations"),
  op("aiHandover.setApplyToAll", "POST", "/v1/inboxes/1/ai-handover/apply-to-all", "integrations"),
  op("aiMcpServers.create", "POST", "/v1/ai-mcp-servers", "automation"),
  op("aiMcpServers.delete", "DELETE", "/v1/ai-mcp-servers/1", "automation"),
  op("aiMcpServers.get", "GET", "/v1/ai-mcp-servers/1", "automation"),
  op("aiMcpServers.list", "GET", "/v1/ai-mcp-servers", "automation"),
  op("aiMcpServers.update", "PUT", "/v1/ai-mcp-servers/1", "automation"),
  op("analytics.activeContactsCount", "GET", "/v1/analytics/active-contacts-count", "analytics"),
  op("analytics.agentResponseSeries", "GET", "/v1/analytics/agent-response-series", "analytics"),
  op("analytics.blockedContactsCount", "GET", "/v1/analytics/blocked-contacts-count", "analytics"),
  op("analytics.blockedContactsPerDay", "GET", "/v1/analytics/blocked-contacts-per-day", "analytics"),
  op("analytics.botMessagesAiProviders", "GET", "/v1/analytics/bot-messages-ai-providers", "analytics"),
  op("analytics.botMessagesByResult", "GET", "/v1/analytics/bot-messages-by-result", "analytics"),
  op("analytics.botMessagesNoResponse", "GET", "/v1/analytics/bot-messages-no-response", "analytics"),
  op("analytics.botMessagesWithResponse", "GET", "/v1/analytics/bot-messages-with-response", "analytics"),
  op("analytics.broadcastStats", "GET", "/v1/analytics/broadcasts/1/stats", "analytics"),
  op("analytics.commentAutomationBotReplies", "GET", "/v1/analytics/comment-automation/bot-replies", "analytics"),
  op("analytics.commentAutomationContacts", "GET", "/v1/analytics/comment-automation/contacts", "analytics"),
  op("analytics.commentAutomationErrors", "GET", "/v1/analytics/comment-automation/errors", "analytics"),
  op("analytics.commentAutomationReplyStats", "GET", "/v1/analytics/comment-automation/replies", "analytics"),
  op("analytics.commentAutomationUserComments", "GET", "/v1/analytics/comment-automation/user-comments", "analytics"),
  op("analytics.contactCountsPerDay", "GET", "/v1/analytics/contact-counts-per-day", "analytics"),
  op("analytics.contactsByDimension", "GET", "/v1/analytics/contacts-by-dimension", "analytics"),
  op("analytics.contactsCount", "GET", "/v1/analytics/contacts-count", "analytics"),
  op("analytics.conversationArchived", "GET", "/v1/analytics/conversation-archived", "analytics"),
  op("analytics.conversationAssigned", "GET", "/v1/analytics/conversation-assigned", "analytics"),
  op("analytics.conversationAssignedByAdmin", "GET", "/v1/analytics/conversation-assigned-by-admin", "analytics"),
  op("analytics.conversationFollowUps", "GET", "/v1/analytics/conversation-followups", "analytics"),
  op("analytics.conversationHandoffs", "GET", "/v1/analytics/conversation-handoffs", "analytics"),
  op("analytics.filterOptions", "GET", "/v1/analytics/filter-options", "analytics"),
  op("analytics.flowSmartDelayStats", "GET", "/v1/analytics/flows/1/smart-delay-stats", "analytics"),
  op("analytics.flowStats", "GET", "/v1/analytics/flows/1/stats", "analytics"),
  op("analytics.humanAgentStats", "GET", "/v1/analytics/human-agent-stats", "analytics"),
  op("analytics.macActiveContactCount", "GET", "/v1/analytics/mac/active-count", "analytics"),
  op("analytics.magicLinkContacts", "GET", "/v1/analytics/magic-links/contacts", "analytics"),
  op("analytics.magicLinkStats", "GET", "/v1/analytics/magic-links/stats", "analytics"),
  op("analytics.messagesByAdmin", "GET", "/v1/analytics/messages-by-admin", "analytics"),
  op("analytics.messagesBySender", "GET", "/v1/analytics/messages-by-sender", "analytics"),
  op("analytics.newContactCountsPerDay", "GET", "/v1/analytics/new-contact-counts-per-day", "analytics"),
  op("analytics.newContactsCount", "GET", "/v1/analytics/new-contacts-count", "analytics"),
  op("analytics.refLinkContacts", "GET", "/v1/analytics/ref-links/contacts", "analytics"),
  op("analytics.refLinkStats", "GET", "/v1/analytics/ref-links/stats", "analytics"),
  op("analytics.resetFlowStats", "DELETE", "/v1/analytics/flows/1/stats", "analytics"),
  op("analytics.sequenceStepStats", "GET", "/v1/analytics/sequences/1/steps/1/stats", "analytics"),
  op("analytics.uniqueConversationsByAdmin", "GET", "/v1/analytics/unique-conversations-by-admin", "analytics"),
]

const WRITES = OPERATIONS.filter((o) => o.method !== "GET")
const READS = OPERATIONS.filter((o) => o.method === "GET")

const call = (token: string, o: Op) => {
  const api = apiClient(token)
  const body = o.method === "GET" || o.method === "DELETE" ? undefined : {}
  return api.http.request(o.method, `/api${o.path}`, {
    ...(token ? { bearer: token } : {}),
    noCookies: true,
    ...(body ? { json: body } : {}),
  })
}

describe("P1.3 auth matrix", () => {
  it("covers the 49 operations of the unit", () => {
    expect(OPERATIONS).toHaveLength(49)
    expect(new Set(OPERATIONS.map((o) => o.id)).size).toBe(49)
    expect(WRITES.map((o) => o.id).sort()).toEqual([
      "aiHandover.patchSettings",
      "aiHandover.retryApplyToAll",
      "aiHandover.saveSettings",
      "aiHandover.setApplyToAll",
      "aiMcpServers.create",
      "aiMcpServers.delete",
      "aiMcpServers.update",
      "analytics.resetFlowStats",
    ])
  })

  describe("no token -> 401 INVALID_CHATBOT_TOKEN", () => {
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call("", o)
      expectError(response, 401, "INVALID_CHATBOT_TOKEN", { defined: true })
      expect((response.json as { message?: string }).message).toBe("Invalid or missing workspace API token")
    })
  })

  describe("unknown workspace token -> 401 INVALID_CHATBOT_TOKEN", () => {
    it.each(
      ["aiHandover.getSettings", "aiMcpServers.list", "analytics.filterOptions", "analytics.resetFlowStats"].map(
        (id) => [id, OPERATIONS.find((o) => o.id === id) as Op] as const,
      ),
    )("%s", async (_id, o) => {
      const response = await call("cbx_ws_parityUnknownTokenValue000000000000000", o)
      expectError(response, 401, "INVALID_CHATBOT_TOKEN", { defined: true })
    })
  })

  describe("token without the operation's scope -> 403 FORBIDDEN", () => {
    let tenant: Tenant
    beforeAll(async () => {
      // `contacts` is none of the three scopes this unit's operations need.
      tenant = await createTenant({ label: "p103-wrong-scope", scopes: ["contacts"] })
    })
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call(tenant.token, o)
      expectError(response, 403, "FORBIDDEN", { defined: true })
      expect((response.json as { message?: string }).message).toBe(
        `Token is not authorized for the '${o.scope}' scope`,
      )
    })
  })

  describe("token with only the operation's scope passes auth", () => {
    const tenants = new Map<Scope, Tenant>()
    beforeAll(async () => {
      for (const scope of ["integrations", "automation", "analytics"] as const) {
        tenants.set(scope, await createTenant({ label: `p103-scope-${scope}`, scopes: [scope] }))
      }
    })
    it.each(OPERATIONS.map((o) => [o.id, o] as const))("%s", async (_id, o) => {
      const response = await call((tenants.get(o.scope) as Tenant).token, o)
      expect([401, 403]).not.toContain(response.status)
      expect(response.status).toBeLessThan(500)
    })
  })

  describe("read-only token", () => {
    let tenant: Tenant
    beforeAll(async () => {
      tenant = await createTenant({ label: "p103-read-only", permission: "read_only" })
    })
    it.each(WRITES.map((o) => [o.id, o] as const))("write %s -> 403 FORBIDDEN", async (_id, o) => {
      const response = await call(tenant.token, o)
      expectError(response, 403, "FORBIDDEN", { defined: true })
      expect((response.json as { message?: string }).message).toBe("Read-only token cannot perform this operation")
    })
    it.each(READS.map((o) => [o.id, o] as const))("read %s passes auth", async (_id, o) => {
      const response = await call(tenant.token, o)
      expect([401, 403]).not.toContain(response.status)
      expect(response.status).toBeLessThan(500)
    })
  })
})
