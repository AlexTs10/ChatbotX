// fetch wrapper for the stack under test: base URL, cookie jar, Origin
// header on unsafe methods, one client address per client, JSON helpers.

import { config } from "./config.ts"
import { uniqueClientIp } from "./unique.ts"

export type HttpResponse<T = unknown> = {
  status: number
  headers: Headers
  text: string
  /** Parsed JSON body, or undefined when the body is not JSON. */
  json: T
  /** `Set-Cookie` values of this response. */
  setCookies: string[]
  url: string
}

export type RequestOptions = {
  headers?: Record<string, string>
  /** JSON body; sets `Content-Type: application/json`. */
  json?: unknown
  /** Raw body (string or bytes) when `json` is not used. */
  body?: string | Uint8Array
  /** `Authorization: Bearer <token>`. */
  bearer?: string
  redirect?: "follow" | "manual" | "error"
  /** Do not send cookies from the jar. */
  noCookies?: boolean
  query?: Record<string, string | number | boolean | undefined>
}

const UNSAFE_METHODS = new Set(["POST", "PUT", "PATCH", "DELETE"])

export class HttpClient {
  readonly baseUrl: string
  /** Simulated client address sent as `X-Forwarded-For`. */
  readonly clientIp: string
  private readonly cookies = new Map<string, string>()

  constructor(options: { baseUrl?: string; clientIp?: string } = {}) {
    this.baseUrl = options.baseUrl ?? config.baseUrl
    this.clientIp = options.clientIp ?? uniqueClientIp()
  }

  cookie(name: string): string | undefined {
    return this.cookies.get(name)
  }

  cookieNames(): string[] {
    return [...this.cookies.keys()]
  }

  cookieHeader(): string {
    return [...this.cookies.entries()].map(([name, value]) => `${name}=${value}`).join("; ")
  }

  clearCookies(): void {
    this.cookies.clear()
  }

  private storeCookies(setCookies: string[]): void {
    for (const line of setCookies) {
      const [pair = "", ...attributes] = line.split(";")
      const index = pair.indexOf("=")
      if (index <= 0) {
        continue
      }
      const name = pair.slice(0, index).trim()
      const value = pair.slice(index + 1).trim()
      const expired = attributes.some((attribute) => {
        const [key = "", raw = ""] = attribute.split("=")
        const lower = key.trim().toLowerCase()
        return (
          (lower === "max-age" && Number.parseInt(raw, 10) <= 0) ||
          (lower === "expires" && Date.parse(raw) < Date.now())
        )
      })
      if (expired || value === "") {
        this.cookies.delete(name)
      } else {
        this.cookies.set(name, value)
      }
    }
  }

  url(path: string, query?: RequestOptions["query"]): string {
    const url = new URL(path.startsWith("http") ? path : `${this.baseUrl}${path}`)
    for (const [key, value] of Object.entries(query ?? {})) {
      if (value !== undefined) {
        url.searchParams.set(key, String(value))
      }
    }
    return url.toString()
  }

  async request<T = unknown>(
    method: string,
    path: string,
    options: RequestOptions = {},
  ): Promise<HttpResponse<T>> {
    const upper = method.toUpperCase()
    const headers: Record<string, string> = {
      "x-forwarded-for": this.clientIp,
      ...(UNSAFE_METHODS.has(upper) ? { origin: this.baseUrl } : {}),
    }
    let body: string | Uint8Array | undefined = options.body
    if (options.json !== undefined) {
      headers["content-type"] = "application/json"
      body = JSON.stringify(options.json)
    }
    if (options.bearer) {
      headers.authorization = `Bearer ${options.bearer}`
    }
    if (!options.noCookies && this.cookies.size > 0) {
      headers.cookie = this.cookieHeader()
    }
    Object.assign(headers, options.headers ?? {})
    const response = await fetch(this.url(path, options.query), {
      method: upper,
      headers,
      body,
      redirect: options.redirect ?? "manual",
    })
    const setCookies = response.headers.getSetCookie()
    this.storeCookies(setCookies)
    const text = await response.text()
    let json: unknown
    try {
      json = text.length > 0 ? JSON.parse(text) : undefined
    } catch {
      json = undefined
    }
    return {
      status: response.status,
      headers: response.headers,
      text,
      json: json as T,
      setCookies,
      url: response.url,
    }
  }

  get<T = unknown>(path: string, options?: RequestOptions) {
    return this.request<T>("GET", path, options)
  }

  post<T = unknown>(path: string, json?: unknown, options: RequestOptions = {}) {
    return this.request<T>("POST", path, { ...options, json: json ?? options.json })
  }

  put<T = unknown>(path: string, json?: unknown, options: RequestOptions = {}) {
    return this.request<T>("PUT", path, { ...options, json: json ?? options.json })
  }

  patch<T = unknown>(path: string, json?: unknown, options: RequestOptions = {}) {
    return this.request<T>("PATCH", path, { ...options, json: json ?? options.json })
  }

  delete<T = unknown>(path: string, options?: RequestOptions) {
    return this.request<T>("DELETE", path, options)
  }
}

/** Client of the public API with a workspace token (`/api/v1/...`). */
export const apiClient = (token: string, options: { clientIp?: string } = {}) => {
  const http = new HttpClient(options)
  const withToken = (request: RequestOptions = {}): RequestOptions => ({
    ...request,
    bearer: token,
    noCookies: true,
  })
  return {
    http,
    get: <T = unknown>(path: string, request?: RequestOptions) =>
      http.get<T>(`/api${path}`, withToken(request)),
    post: <T = unknown>(path: string, json?: unknown, request?: RequestOptions) =>
      http.post<T>(`/api${path}`, json, withToken(request)),
    put: <T = unknown>(path: string, json?: unknown, request?: RequestOptions) =>
      http.put<T>(`/api${path}`, json, withToken(request)),
    patch: <T = unknown>(path: string, json?: unknown, request?: RequestOptions) =>
      http.patch<T>(`/api${path}`, json, withToken(request)),
    delete: <T = unknown>(path: string, request?: RequestOptions) =>
      http.delete<T>(`/api${path}`, withToken(request)),
  }
}
