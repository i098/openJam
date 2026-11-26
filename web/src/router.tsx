import { QueryClientProvider } from '@tanstack/react-query'
import { createRootRoute, createRouter, Outlet } from '@tanstack/react-router'
import { queryClient } from '@/lib/api'
import { dashboardRoute } from '@/routes/dashboard'
import { loginRoute } from '@/routes/login'
import { promptRoute } from '@/routes/prompt'
import { promptsRoute } from '@/routes/prompts'
import { setupRoute } from '@/routes/setup'
import { indexRoute, shellRoute } from '@/routes/shell'

export const rootRoute = createRootRoute({
  component: () => (
    <QueryClientProvider client={queryClient}>
      <Outlet />
    </QueryClientProvider>
  ),
})

const routeTree = rootRoute.addChildren([
  indexRoute,
  loginRoute,
  shellRoute.addChildren([dashboardRoute, promptsRoute, promptRoute, setupRoute]),
])

export const router = createRouter({ routeTree })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
