// @vitest-environment node
// Generates crates/api/assets/public-spec.json (Q.P6-U5): the document of
// `apps/builder/src/app/api/public-spec.json/route.ts` without a filter, with
// the placeholders `__TENANT_NAME__` (info.title) and `__SERVER_URL__`
// (servers[0].url) that the Rust route replaces. Run: gen.sh.
import { writeFileSync } from "node:fs"
import { OpenAPIGenerator } from "@orpc/openapi"
import { ZodToJsonSchemaConverter } from "@orpc/zod/zod4"
import { test, vi } from "vitest"

vi.mock("@/middlewares/auth", () => ({
  authMiddleware: vi.fn(),
  workspaceAuthorizedMidddleware: vi.fn(),
}))
vi.mock("@chatbotx.io/database/client", () => {
  const proxy: unknown = new Proxy(() => proxy, { get: () => proxy })
  return { db: proxy }
})

test("print the public spec", async () => {
  await import("@/polyfill")
  const {
    publicSpecGenerateOptions,
    withChannelApiTokenSecurity,
    withIdempotencyKeyHeader,
    withMcpHitlPolicy,
  } = await import("@/lib/orpc/public-spec")
  const { publicRouter } = await import("@/routers/public")
  const spec = await new OpenAPIGenerator({
    schemaConverters: [new ZodToJsonSchemaConverter()],
  }).generate(publicRouter, {
    ...publicSpecGenerateOptions("__TENANT_NAME__"),
    servers: [{ url: "__SERVER_URL__" }],
  })
  const document = withMcpHitlPolicy(
    withIdempotencyKeyHeader(withChannelApiTokenSecurity(spec)),
  )
  writeFileSync(process.env.PUBLIC_SPEC_OUT ?? "/tmp/public-spec.json", JSON.stringify(document))
}, 600_000)
