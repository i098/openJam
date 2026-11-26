import { useQuery } from '@tanstack/react-query'
import { createRoute, Link, Outlet, redirect, useNavigate, useParams } from '@tanstack/react-router'
import { Button } from '@/components/ui/button'
import { api, type Me, type ProjectSummary, queryClient, session } from '@/lib/api'
import { rootRoute } from '@/router'

/** Loads the session, or sends the visitor to the sign-in page. */
export async function requireSession(): Promise<Me> {
  try {
    return await queryClient.fetchQuery({ queryKey: ['me'], queryFn: session, staleTime: 30_000 })
  } catch {
    throw redirect({ to: '/login' })
  }
}

export const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  beforeLoad: async () => {
    const me = await requireSession()
    throw me.project
      ? redirect({ to: '/p/$projectId', params: { projectId: me.project.id } })
      : redirect({ to: '/start' })
  },
})

export const shellRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: 'shell',
  loader: requireSession,
  component: Shell,
})

const navLink =
  'rounded-md px-3 py-1.5 text-sm font-medium hover:bg-surface aria-[current=page]:bg-primary-soft aria-[current=page]:text-primary'

function Shell() {
  const me = shellRoute.useLoaderData()
  const { projectId } = useParams({ strict: false })
  const navigate = useNavigate()
  const projects = useQuery({
    queryKey: ['projects'],
    queryFn: () => api<{ projects: ProjectSummary[] }>('/v1/projects'),
  })

  async function signOut() {
    await api('/auth/logout', { method: 'POST' }).catch(() => null)
    queryClient.clear()
    await navigate({ to: '/login' })
  }

  return (
    <div className="min-h-dvh">
      <header className="border-b border-border bg-background">
        <div className="mx-auto flex w-full max-w-6xl flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3 sm:px-6">
          <Link to="/" className="text-lg font-semibold tracking-tight">
            openJam
          </Link>
          <select
            aria-label="Project"
            className="h-9 min-w-0 max-w-[14rem] flex-1 truncate text-ellipsis sm:flex-none rounded-md border border-border bg-background py-0 pl-2 pr-6 text-sm"
            value={projectId ?? ''}
            onChange={(e) =>
              e.target.value === '+'
                ? navigate({ to: '/start' })
                : navigate({ to: '/p/$projectId', params: { projectId: e.target.value } })
            }
          >
            {!projectId && <option value="">Choose a project</option>}
            {projects.data?.projects.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
            <option value="+">New project…</option>
          </select>
          <span className="ml-auto hidden truncate text-sm sm:inline">{me.user.email}</span>
          <Button variant="ghost" size="sm" className="shrink-0 max-sm:ml-auto" onClick={signOut}>
            Sign out
          </Button>
          {projectId && (
            <nav aria-label="Project" className="flex w-full gap-1">
              <Link
                to="/p/$projectId"
                params={{ projectId }}
                activeOptions={{ exact: true }}
                className={navLink}
              >
                Dashboard
              </Link>
              <Link to="/p/$projectId/prompts" params={{ projectId }} className={navLink}>
                Prompts
              </Link>
            </nav>
          )}
        </div>
      </header>
      <main className="mx-auto w-full max-w-6xl px-4 py-6 sm:px-6 sm:py-8">
        <Outlet />
      </main>
    </div>
  )
}
