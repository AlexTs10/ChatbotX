// `@chatbotx.io/business/error-log` `listErrorLogs` (the error-logs page) in
// the Rust server.
import type { listErrorLogs as listErrorLogsService } from "@chatbotx.io/business/error-log"
import { serverQuery } from "@/lib/server-action"

export const listErrorLogs = (
  input: Parameters<typeof listErrorLogsService>[0],
) =>
  serverQuery<Awaited<ReturnType<typeof listErrorLogsService>>>(
    "listErrorLogs",
    input.workspaceId,
    [input],
  )
