// oRPC RPC protocol codec for the internal `/rpc/<router>/<procedure>` API.
// Request body: {"json": <input>, "meta": [...]}. Response: {"json": <output>,
// "meta": [...]}; `meta` marks values that JSON cannot carry (Date, BigInt,
// undefined, ...). GET carries the same object in `?data=`.

import type { HttpClient, HttpResponse } from "./http.ts"

export type RpcMeta = Array<[number, ...(string | number)[]]>

export type RpcResult<T> = {
  status: number
  /** Output on success, or the error object ({defined, code, status, message, data}). */
  json: T
  meta: RpcMeta
  ok: boolean
  response: HttpResponse
}

export type RpcError = {
  defined: boolean
  code: string
  status: number
  message: string
  data?: unknown
}

export const rpc = async <T = unknown>(
  http: HttpClient,
  procedure: string,
  input: unknown = {},
  options: { method?: "GET" | "POST"; headers?: Record<string, string> } = {},
): Promise<RpcResult<T>> => {
  const path = `/rpc/${procedure.replace(/^\/+/, "")}`
  const payload = { json: input, meta: [] }
  const response =
    options.method === "GET"
      ? await http.get(path, {
          query: { data: JSON.stringify(payload) },
          headers: options.headers,
        })
      : await http.post(path, payload, { headers: options.headers })
  const body = (response.json ?? {}) as { json?: T; meta?: RpcMeta }
  return {
    status: response.status,
    json: body.json as T,
    meta: body.meta ?? [],
    ok: response.status >= 200 && response.status < 300,
    response,
  }
}
