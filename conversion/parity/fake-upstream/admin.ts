// Admin API for tests (`FAKE_UPSTREAM_ADMIN_URL`, default http://fake-upstream:4010).
//
//   GET  /__admin/health                  -> {"ok":true}
//   POST /__admin/reset                   -> clear records, scripts, DNS overrides
//   POST /__admin/scripts                 body: script or script[] -> {"ids":[...]}
//        script = {match:{host, method?, path?|pathPrefix?|pathRegex?, bodyIncludes?},
//                  respond:{status, headers?, json?|text?|base64?, delayMs?}, times?}
//   POST /__admin/dns                     body: {name, a?:[ip], aaaa?:[ip]} (DNS + DoH answers)
//   GET  /__admin/requests?kind=&host=&method=&pathPrefix=&since=   -> {"records":[...]}
//   GET  /__admin/requests/wait?...&count=1&timeoutMs=10000         -> {"records":[...], "timedOut":bool}
//   GET  /__admin/unmatched?since=         -> {"records":[...]} (no script/default, or unknown SNI)
//   GET  /__admin/ca                       -> test CA certificate (PEM)

import { readFileSync } from "node:fs"
import type { IncomingMessage, ServerResponse } from "node:http"
import { filterRecords, type RecordFilter, store } from "./store.ts"

const MAX_WAIT_MS = 60_000

const readJson = (request: IncomingMessage): Promise<unknown> =>
  new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    request.on("data", (chunk: Buffer) => chunks.push(chunk))
    request.on("end", () => {
      const text = Buffer.concat(chunks).toString("utf8")
      try {
        resolve(text ? JSON.parse(text) : {})
      } catch (error) {
        reject(error)
      }
    })
    request.on("error", reject)
  })

const reply = (response: ServerResponse, status: number, body: unknown) => {
  const text = JSON.stringify(body)
  response.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(text),
  })
  response.end(text)
}

const filterFrom = (params: URLSearchParams): RecordFilter => {
  const since = params.get("since")
  return {
    kind: params.get("kind") ?? undefined,
    host: params.get("host")?.toLowerCase() ?? undefined,
    method: params.get("method") ?? undefined,
    pathPrefix: params.get("pathPrefix") ?? undefined,
    since: since ? Number.parseInt(since, 10) : undefined,
  }
}

export const createAdminHandler =
  (input: { caFile: string }) =>
  async (request: IncomingMessage, response: ServerResponse): Promise<void> => {
    const url = new URL(request.url ?? "/", "http://admin")
    const route = `${request.method ?? "GET"} ${url.pathname}`
    try {
      switch (route) {
        case "GET /__admin/health":
          return reply(response, 200, { ok: true })
        case "POST /__admin/reset":
          store.reset()
          return reply(response, 200, { ok: true })
        case "POST /__admin/scripts": {
          const body = await readJson(request)
          const list = Array.isArray(body) ? body : [body]
          const ids = list.map(
            (script) => store.addScript(script as Parameters<typeof store.addScript>[0]).id,
          )
          return reply(response, 200, { ids })
        }
        case "POST /__admin/dns": {
          const body = (await readJson(request)) as {
            name: string
            a?: string[]
            aaaa?: string[]
          }
          if (!body.name) {
            return reply(response, 400, { error: "name is required" })
          }
          store.setDnsOverride(body)
          return reply(response, 200, { ok: true })
        }
        case "GET /__admin/requests":
          return reply(response, 200, {
            records: filterRecords(filterFrom(url.searchParams)),
          })
        case "GET /__admin/requests/wait": {
          const filter = filterFrom(url.searchParams)
          const count = Number.parseInt(url.searchParams.get("count") ?? "1", 10)
          const timeoutMs = Math.min(
            Number.parseInt(url.searchParams.get("timeoutMs") ?? "10000", 10),
            MAX_WAIT_MS,
          )
          const ok = await store.waitFor(
            () => filterRecords(filter).length >= count,
            timeoutMs,
          )
          return reply(response, 200, {
            records: filterRecords(filter),
            timedOut: !ok,
          })
        }
        case "GET /__admin/unmatched": {
          const since = url.searchParams.get("since")
          return reply(response, 200, {
            records: filterRecords({
              unmatched: true,
              since: since ? Number.parseInt(since, 10) : undefined,
            }),
          })
        }
        case "GET /__admin/ca": {
          const pem = readFileSync(input.caFile, "utf8")
          response.writeHead(200, { "content-type": "application/x-pem-file" })
          response.end(pem)
          return
        }
        default:
          return reply(response, 404, { error: `unknown admin route ${route}` })
      }
    } catch (error) {
      return reply(response, 400, { error: String(error) })
    }
  }
