// HTTP(S) handler for intercepted third-party traffic: record the request,
// then answer from the newest matching script, a built-in default, or 599.

import type { IncomingMessage, ServerResponse } from "node:http"
import type { TLSSocket } from "node:tls"
import { defaultHandlers } from "./defaults.ts"
import { type ScriptResponse, store } from "./store.ts"

export const UNMATCHED_STATUS = 599

const readBody = (request: IncomingMessage): Promise<Buffer> =>
  new Promise((resolve, reject) => {
    const chunks: Buffer[] = []
    request.on("data", (chunk: Buffer) => chunks.push(chunk))
    request.on("end", () => resolve(Buffer.concat(chunks)))
    request.on("error", reject)
  })

const hostOf = (request: IncomingMessage): string => {
  const servername = (request.socket as TLSSocket).servername
  const raw: string =
    request.headers.host ||
    (typeof servername === "string" ? servername : "") ||
    request.socket.localAddress ||
    ""
  // Strip the port; keep IPv6 literals intact.
  const host = raw.startsWith("[") ? raw.slice(0, raw.indexOf("]") + 1) : raw.split(":")[0]
  return (host ?? "").toLowerCase()
}

const flattenHeaders = (request: IncomingMessage): Record<string, string> => {
  const headers: Record<string, string> = {}
  for (const [key, value] of Object.entries(request.headers)) {
    if (value !== undefined) {
      headers[key] = Array.isArray(value) ? value.join(", ") : value
    }
  }
  return headers
}

const queryObject = (params: URLSearchParams): Record<string, string | string[]> => {
  const result: Record<string, string | string[]> = {}
  for (const key of new Set(params.keys())) {
    const values = params.getAll(key)
    result[key] = values.length === 1 ? (values[0] ?? "") : values
  }
  return result
}

const send = async (
  response: ServerResponse,
  answer: ScriptResponse,
): Promise<void> => {
  if (answer.delayMs) {
    await new Promise((resolve) => setTimeout(resolve, answer.delayMs))
  }
  const headers: Record<string, string> = { ...(answer.headers ?? {}) }
  const lowerKeys = new Set(Object.keys(headers).map((key) => key.toLowerCase()))
  let body: Buffer
  if (answer.base64 !== undefined) {
    body = Buffer.from(answer.base64, "base64")
  } else if (answer.text !== undefined) {
    body = Buffer.from(answer.text, "utf8")
    if (!lowerKeys.has("content-type")) {
      headers["content-type"] = "text/plain; charset=utf-8"
    }
  } else if (answer.json !== undefined) {
    body = Buffer.from(JSON.stringify(answer.json), "utf8")
    if (!lowerKeys.has("content-type")) {
      headers["content-type"] = "application/json"
    }
  } else {
    body = Buffer.alloc(0)
  }
  headers["content-length"] = String(body.length)
  response.writeHead(answer.status, headers)
  response.end(body)
}

export const createUpstreamHandler =
  (scheme: "http" | "https") =>
  async (request: IncomingMessage, response: ServerResponse): Promise<void> => {
    try {
      const body = await readBody(request)
      const url = new URL(request.url ?? "/", "http://placeholder")
      const host = hostOf(request)
      const method = (request.method ?? "GET").toUpperCase()
      const bodyText = body.toString("utf8")
      const script = store.findScript({ host, method, path: url.pathname, bodyText })
      let answer: ScriptResponse | null = script?.respond ?? null
      let matchedScript = script?.id ?? null
      if (!answer) {
        for (const handler of defaultHandlers) {
          answer = handler.handle({
            host,
            method,
            path: url.pathname,
            query: url.searchParams,
            headers: flattenHeaders(request),
            body,
            remoteAddress: request.socket.remoteAddress,
          })
          if (answer) {
            matchedScript = handler.name
            break
          }
        }
      }
      store.addRecord({
        kind: "http",
        scheme,
        host,
        method,
        path: url.pathname,
        rawQuery: url.search.replace(/^\?/, ""),
        query: queryObject(url.searchParams),
        headers: flattenHeaders(request),
        bodyText,
        bodyBase64: body.toString("base64"),
        matchedScript,
        unmatched: answer === null,
        remoteAddress: request.socket.remoteAddress,
      })
      await send(
        response,
        answer ?? {
          status: UNMATCHED_STATUS,
          json: { error: "parity fake upstream: no script for this request", host, method, path: url.pathname },
        },
      )
    } catch (error) {
      if (!response.headersSent) {
        response.writeHead(500, { "content-type": "text/plain" })
      }
      response.end(`fake upstream error: ${String(error)}`)
    }
  }
