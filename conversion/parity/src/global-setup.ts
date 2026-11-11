// Runs once before all test files: wait until the stack answers, then clear
// the fake upstream so the interception self-check sees only test traffic.

import { config } from "./config.ts"
import { eventually } from "./eventually.ts"
import { fake } from "./fake.ts"

const expectOk = async (url: string, expectedText?: string): Promise<void> => {
  const response = await fetch(url, { signal: AbortSignal.timeout(5_000) })
  const text = await response.text()
  if (!response.ok || (expectedText !== undefined && text.trim() !== expectedText)) {
    throw new Error(`${url}: ${response.status} ${text.slice(0, 200)}`)
  }
}

export default async function setup(): Promise<void> {
  const options = { timeoutMs: config.readyTimeoutMs, intervalMs: 1_000 }
  await eventually(() => fake.health(), { ...options, description: "fake upstream" })
  await eventually(() => expectOk(`${config.mailhogUrl}/api/v2/messages?limit=1`), {
    ...options,
    description: "mailhog",
  })
  await eventually(() => expectOk(`${config.baseUrl}/api/health`), {
    ...options,
    description: "app /api/health",
  })
  await eventually(() => expectOk(`${config.baseUrl}/ws/ready`, "ready"), {
    ...options,
    description: "realtime /ws/ready",
  })
  await fake.reset()
}
