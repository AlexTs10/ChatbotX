import "./app/globals.css"
import "./app/themes.css"
import { QueryClientProvider } from "@tanstack/react-query"
import { RouterProvider } from "@tanstack/react-router"
import { StrictMode, Suspense } from "react"
import { createRoot } from "react-dom/client"
import { getQueryClient } from "@/lib/query/query-client"
import { syncTimezoneCookie } from "@/lib/timezone"
import { createAppRouter } from "@/router"

syncTimezoneCookie()

const queryClient = getQueryClient()
const router = createAppRouter(queryClient)

const root = document.getElementById("root")
if (root) {
  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <Suspense>
          <RouterProvider router={router} />
        </Suspense>
      </QueryClientProvider>
    </StrictMode>,
  )
}
