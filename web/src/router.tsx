import { createRootRoute, createRouter, Outlet } from '@tanstack/react-router'
import { dashboardRoute } from '@/routes/dashboard'
import { loginRoute } from '@/routes/login'

export const rootRoute = createRootRoute({ component: () => <Outlet /> })

const routeTree = rootRoute.addChildren([dashboardRoute, loginRoute])

export const router = createRouter({ routeTree })

declare module '@tanstack/react-router' {
  interface Register {
    router: typeof router
  }
}
