// Client of the fake upstream admin API (see fake-upstream/admin.ts).

import { config } from "./config.ts"

export type UpstreamRecord = {
  id: number
  ts: string
  kind: "http" | "dns" | "tls"
  scheme: "http" | "https" | "dns"
  host: string
  method: string
  path: string
  rawQuery: string
  query: Record<string, string | string[]>
  headers: Record<string, string>
  bodyText: string
  bodyBase64: string
  matchedScript: string | null
  unmatched: boolean
  unknownSni?: string
  dnsType?: string
  dnsTransport?: "udp" | "tcp" | "doh"
  remoteAddress?: string
}

export type ScriptInput = {
  match: {
    host: string
    method?: string
    path?: string
    pathPrefix?: string
    pathRegex?: string
    bodyIncludes?: string
  }
  respond: {
    status: number
    headers?: Record<string, string>
    json?: unknown
    text?: string
    base64?: string
    delayMs?: number
  }
  times?: number
}

export type RecordQuery = {
  kind?: "http" | "dns" | "tls"
  host?: string
  method?: string
  pathPrefix?: string
  since?: number
}

const admin = async <T>(path: string, init?: RequestInit): Promise<T> => {
  const response = await fetch(`${config.fakeUpstreamAdminUrl}${path}`, init)
  const text = await response.text()
  if (!response.ok) {
    throw new Error(`fake upstream ${path}: ${response.status} ${text}`)
  }
  return (text ? JSON.parse(text) : {}) as T
}

const queryString = (query: Record<string, string | number | undefined>): string => {
  const params = new URLSearchParams()
  for (const [key, value] of Object.entries(query)) {
    if (value !== undefined) {
      params.set(key, String(value))
    }
  }
  const text = params.toString()
  return text ? `?${text}` : ""
}

export const fake = {
  health: () => admin<{ ok: boolean }>("/__admin/health"),

  /** Clears all records and scripts. Global setup only; tests filter by owned hosts. */
  reset: () => admin("/__admin/reset", { method: "POST" }),

  script: async (...scripts: ScriptInput[]): Promise<string[]> =>
    (
      await admin<{ ids: string[] }>("/__admin/scripts", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(scripts),
      })
    ).ids,

  /** DNS and DoH answers for one name (for example a private IP for SSRF tests). */
  dns: (input: { name: string; a?: string[]; aaaa?: string[] }) =>
    admin("/__admin/dns", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
    }),

  requests: async (query: RecordQuery = {}): Promise<UpstreamRecord[]> =>
    (await admin<{ records: UpstreamRecord[] }>(`/__admin/requests${queryString(query)}`))
      .records,

  /** Waits until at least `count` records match, then returns all matches. */
  waitFor: async (
    query: RecordQuery,
    options: { count?: number; timeoutMs?: number } = {},
  ): Promise<UpstreamRecord[]> => {
    const count = options.count ?? 1
    const result = await admin<{ records: UpstreamRecord[]; timedOut: boolean }>(
      `/__admin/requests/wait${queryString({ ...query, count, timeoutMs: options.timeoutMs ?? 10_000 })}`,
    )
    if (result.timedOut) {
      throw new Error(
        `fake upstream: ${result.records.length}/${count} record(s) for ${JSON.stringify(query)}`,
      )
    }
    return result.records
  },

  unmatched: async (since?: number): Promise<UpstreamRecord[]> =>
    (await admin<{ records: UpstreamRecord[] }>(`/__admin/unmatched${queryString({ since })}`))
      .records,

  /** Id of the newest record; pass it as `since` to see only later records. */
  cursor: async (): Promise<number> => {
    const records = await fake.requests()
    return records.at(-1)?.id ?? 0
  },
}
