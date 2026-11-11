// connections.* with a `token` channel provider (Telegram): the bot token is
// checked with `getMe` on create and verify; disconnect deletes the webhook.
// Every Telegram call path carries the unique bot token (`/bot<token>/...`).

import { describe, expect, it } from "vitest"
import { fake } from "../../src/fake.ts"
import { uniqueId } from "../../src/unique.ts"
import { type Api, expectError, reEscape, script, tenantApi, upstreamCalls } from "./helpers.ts"

const HOST = "api.telegram.org"

type Connection = Record<string, unknown> & { id: string; inboxId: string | null; status: string; sourceId: string }
type Envelope = { connection: Connection | null; session: unknown; secret: unknown }

const newBot = () => {
  const botId = `7${String(Math.floor(Math.random() * 1e9)).padStart(9, "0")}`
  const token = `${botId}:AA${uniqueId()}`
  return { botId, token, prefix: `/bot${token}/` }
}

/** Every call of this bot answers `{ok:true,result:true}`; `getMe` answers the bot profile (or `getMeStatus`). */
const scriptBot = async (bot: ReturnType<typeof newBot>, getMeStatus = 200) => {
  await script({
    match: { host: HOST, pathPrefix: bot.prefix },
    respond: { status: 200, json: { ok: true, result: true } },
  })
  await script({
    match: { host: HOST, path: `${bot.prefix}getMe` },
    respond:
      getMeStatus === 200
        ? {
            status: 200,
            json: {
              ok: true,
              result: { id: Number(bot.botId), is_bot: true, first_name: "Parity Bot", username: `parity_${bot.botId}_bot` },
            },
          }
        : { status: getMeStatus, json: { ok: false, error_code: getMeStatus, description: "Unauthorized" } },
  })
}

const connectTelegram = async (api: Api, bot = newBot()) => {
  await scriptBot(bot)
  const response = await api.post<Envelope>("/v1/connections", { provider: "telegram", config: { secretText: bot.token } })
  expect(response.status).toBe(201)
  return { bot, connection: response.json.connection as Connection, response }
}

const botCalls = async (bot: ReturnType<typeof newBot>, since: number) =>
  (await upstreamCalls({ host: HOST, since, path: new RegExp(`^${reEscape(bot.prefix)}`) })).map(
    (record) => `${record.method} ${record.path.slice(bot.prefix.length)}`,
  )

describe("connections.create (telegram, token)", () => {
  it("checks the bot token with getMe and connects the channel (201)", async () => {
    const { api } = await tenantApi("p105-tg-create")
    const bot = newBot()
    const since = await fake.cursor()
    const { connection, response } = await connectTelegram(api, bot)
    expect(response.json.session).toBeNull()
    expect(response.json.secret).toBeNull()
    expect(connection).toMatchObject({
      kind: "channel",
      provider: "telegram",
      channel: "telegram",
      sourceId: bot.botId,
      displayName: "Telegram bot",
      strategy: "token",
      status: "connected",
      statusReason: null,
      integrationId: null,
      inboxId: expect.any(String),
      capabilities: { multiAccount: false, refreshable: false, verifiable: true },
    })
    expect(await botCalls(bot, since)).toContain("GET getMe")
  })

  it("refuses a rejected bot token (400 connectionCredentialsRejected)", async () => {
    const { api } = await tenantApi("p105-tg-reject")
    const bot = newBot()
    await scriptBot(bot, 401)
    expectError(
      await api.post("/v1/connections", { provider: "telegram", config: { secretText: bot.token } }),
      400,
      "connectionCredentialsRejected",
      { defined: true },
    )
    expect((await api.get<{ data: unknown[] }>("/v1/connections")).json.data).toEqual([])
  })

  it("requires secretText (422 validation)", async () => {
    const { api } = await tenantApi("p105-tg-cfg")
    expectError(await api.post("/v1/connections", { provider: "telegram", config: {} }), 422, "validation", {
      message: "secretText is required",
    })
  })

  it("refuses the same bot twice in a workspace (409 connectionAlreadyConnected)", async () => {
    const { api } = await tenantApi("p105-tg-dup")
    const { bot } = await connectTelegram(api)
    await scriptBot(bot)
    expectError(
      await api.post("/v1/connections", { provider: "telegram", config: { secretText: bot.token } }),
      409,
      "connectionAlreadyConnected",
      { defined: true },
    )
  })
})

describe("connections lifecycle (telegram)", () => {
  it("verify calls getMe; a revoked token (401) marks needs_reauth", async () => {
    const { api } = await tenantApi("p105-tg-verify")
    const { bot, connection } = await connectTelegram(api)
    const since = await fake.cursor()
    const ok = await api.post<Connection>(`/v1/connections/${connection.id}/verify`, {})
    expect(ok.status).toBe(200)
    expect(ok.json).toMatchObject({ id: connection.id, status: "connected" })
    expect(await botCalls(bot, since)).toEqual(["GET getMe"])

    await script({
      match: { host: HOST, path: `${bot.prefix}getMe` },
      respond: { status: 401, json: { ok: false, error_code: 401, description: "Unauthorized" } },
    })
    const revoked = await api.post<Connection>(`/v1/connections/${connection.id}/verify`, {})
    expect(revoked.status).toBe(200)
    expect(revoked.json).toMatchObject({ status: "needs_reauth", statusReason: "token_revoked" })
  })

  it("refresh is refused (400 connectionNotRefreshable)", async () => {
    const { api } = await tenantApi("p105-tg-refresh")
    const { connection } = await connectTelegram(api)
    expectError(await api.post(`/v1/connections/${connection.id}/refresh`, {}), 400, "connectionNotRefreshable")
  })

  it("disconnect deletes the bot webhook upstream and marks the connection disconnected", async () => {
    const { api } = await tenantApi("p105-tg-disc")
    const { bot, connection } = await connectTelegram(api)
    const since = await fake.cursor()
    const response = await api.delete<Connection>(`/v1/connections/${connection.id}`)
    expect(response.status).toBe(200)
    expect(response.json).toMatchObject({
      id: connection.id,
      status: "disconnected",
      disconnectedAt: expect.any(String),
      lastError: null,
    })
    expect(await botCalls(bot, since)).toContain("POST deleteWebhook")
  })

  it("disconnect still succeeds when the provider teardown fails, and records lastError", async () => {
    const { api } = await tenantApi("p105-tg-disc-fail")
    const { bot, connection } = await connectTelegram(api)
    await script({
      match: { host: HOST, path: `${bot.prefix}deleteWebhook` },
      respond: { status: 500, json: { ok: false, error_code: 500, description: "Internal" } },
    })
    const response = await api.delete<Connection>(`/v1/connections/${connection.id}`)
    expect(response.status).toBe(200)
    expect(response.json.status).toBe("disconnected")
    const stored = await api.get<Connection>(`/v1/connections/${connection.id}`)
    expect(stored.json).toMatchObject({ status: "disconnected", lastError: expect.any(String) })
  })
})
