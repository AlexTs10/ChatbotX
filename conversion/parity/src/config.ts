// Endpoints of the stack under test. Tests use only these values and the
// fixture driver, so the same tests run against every stack.

const read = (name: string, fallback: string): string => {
  const value = process.env[name]
  return value && value.length > 0 ? value.replace(/\/+$/, "") : fallback
}

export const config = {
  /** Edge origin: the app, `/api/*`, `/rpc/*`, `/ws/*`, `/storage/*`. Port 80 only. */
  baseUrl: read("PARITY_BASE_URL", read("BASE_URL", "http://app")),
  /** Realtime WebSocket base through the edge. */
  rtUrl: read("RT_URL", "ws://app/ws"),
  /** Admin API of the fake third-party upstream. */
  fakeUpstreamAdminUrl: read("FAKE_UPSTREAM_ADMIN_URL", "http://fake-upstream:4010"),
  /** MailHog HTTP API (captures all outgoing mail). */
  mailhogUrl: read("MAILHOG_URL", "http://mailhog:8025"),
  /** Name of the folder under `stacks/` that holds the fixture adapter. */
  stack: read("STACK", "original"),
  /** Time budget for the stack to become ready in global setup. */
  readyTimeoutMs: Number.parseInt(read("PARITY_READY_TIMEOUT_MS", "300000"), 10),
} as const

export type ParityConfig = typeof config
