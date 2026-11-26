import { createRoute, redirect, useLoaderData } from '@tanstack/react-router'
import { api, type ProjectList, type SessionInfo } from '@/lib/api'
import { rootRoute } from '@/router'

export const dashboardRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  loader: async () => {
    try {
      const [me, projects] = await Promise.all([
        api<SessionInfo & { user: { email: string } }>('/auth/me'),
        api<ProjectList>('/v1/projects'),
      ])
      return { me, projects }
    } catch {
      throw redirect({ to: '/login' })
    }
  },
  component: DashboardPage,
})

function DashboardPage() {
  const { me, projects } = useLoaderData({ from: '/' })
  return (
    <main className="mx-auto w-full max-w-3xl px-6 py-10">
      <header className="flex items-baseline justify-between gap-4">
        <h1 className="text-xl font-semibold tracking-tight">openJam</h1>
        <span className="text-sm text-muted">{me.user.email}</span>
      </header>
      <section className="mt-8 rounded-lg border border-border bg-surface p-6">
        <h2 className="text-base font-medium">Projects</h2>
        {projects.projects.length === 0 ? (
          <p className="mt-2 text-sm text-muted">
            No projects yet. Add one with the API: <code>POST /v1/projects</code>.
          </p>
        ) : (
          <ul className="mt-3 divide-y divide-border">
            {projects.projects.map((project) => (
              <li key={project.id} className="flex items-center justify-between py-2 text-sm">
                <span className="font-medium">{project.name}</span>
                <span className="tabular-nums text-muted">{project.domain}</span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </main>
  )
}
