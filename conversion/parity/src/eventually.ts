// Retry an assertion until it passes. Use this for every asynchronous effect
// (worker jobs, mail, realtime fan-out); never use a fixed sleep.

export type EventuallyOptions = {
  timeoutMs?: number
  intervalMs?: number
  /** Shown in the error when the timeout expires. */
  description?: string
}

export const eventually = async <T>(
  fn: () => T | Promise<T>,
  options: EventuallyOptions = {},
): Promise<T> => {
  const timeoutMs = options.timeoutMs ?? 10_000
  const intervalMs = options.intervalMs ?? 100
  const deadline = Date.now() + timeoutMs
  let lastError: unknown
  let attempts = 0
  for (;;) {
    attempts += 1
    try {
      return await fn()
    } catch (error) {
      lastError = error
    }
    if (Date.now() >= deadline) {
      break
    }
    await new Promise((resolve) =>
      setTimeout(resolve, Math.min(intervalMs, Math.max(deadline - Date.now(), 0))),
    )
  }
  const label = options.description ? ` (${options.description})` : ""
  const error = new Error(
    `eventually${label}: still failing after ${timeoutMs} ms and ${attempts} attempts: ${
      lastError instanceof Error ? lastError.message : String(lastError)
    }`,
  )
  error.cause = lastError
  throw error
}
