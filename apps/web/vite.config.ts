import { fileURLToPath } from "node:url"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { defineConfig } from "vite"

const src = (path: string) =>
  fileURLToPath(new URL(`./src/${path}`, import.meta.url))

// The Rust server (`chatbotx-server`) in dev; override with CHATBOTX_SERVER_URL.
const server = process.env.CHATBOTX_SERVER_URL ?? "http://localhost:3000"

export default defineConfig({
  // ponytail: served from the builder until apps/builder is removed; then move the folder here.
  publicDir: "../builder/public",
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: [
      // Next.js APIs used by the copied builder code and by packages/ui.
      {
        find: /^next\/navigation$/,
        replacement: src("shims/next-navigation.ts"),
      },
      { find: /^next\/link$/, replacement: src("shims/next-link.tsx") },
      { find: /^next\/image$/, replacement: src("shims/next-image.tsx") },
      { find: /^next\/dynamic$/, replacement: src("shims/next-dynamic.tsx") },
      { find: /^next\/script$/, replacement: src("shims/next-script.tsx") },
      { find: /^next-intl$/, replacement: src("shims/next-intl.ts") },
      {
        find: /^nuqs\/adapters\/next\/app$/,
        replacement: "nuqs/adapters/tanstack-router",
      },
      { find: /^@\//, replacement: src("") },
    ],
  },
  server: {
    port: 3123,
    proxy: Object.fromEntries(
      ["/rpc", "/api", "/files", "/realtime"].map((path) => [
        path,
        { target: server, changeOrigin: false, ws: path === "/realtime" },
      ]),
    ),
  },
  build: {
    chunkSizeWarningLimit: 4096,
  },
})
