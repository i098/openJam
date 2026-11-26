import { useMutation } from '@tanstack/react-query'
import { createRoute, Link } from '@tanstack/react-router'
import { type FormEvent, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, Problem, ScanButton } from '@/components/ui/kit'
import { api, type ProjectSummary, queryClient } from '@/lib/api'
import { cn } from '@/lib/cn'
import { shellRoute } from '@/routes/shell'

export const setupRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: '/start',
  component: Setup,
})

const STEPS = ['Project', 'Brand', 'Competitors', 'Prompts', 'First scan'] as const
const lines = (text: string) =>
  text
    .split(/[\n,]/)
    .map((line) => line.trim())
    .filter(Boolean)
const post = (path: string, body: unknown, method = 'POST') =>
  api(path, { method, body: JSON.stringify(body) })

function Field({ id, label, children }: { id: string; label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5 text-sm font-medium">
      <label htmlFor={id}>{label}</label>
      {children}
    </div>
  )
}

const textarea =
  'min-h-28 w-full rounded-md border border-border bg-background px-3 py-2 text-sm font-normal outline-none focus:border-primary focus:ring-2 focus:ring-primary/20'

function Setup() {
  const [step, setStep] = useState(0)
  const [project, setProject] = useState<ProjectSummary>()
  const [form, setForm] = useState({
    name: '',
    domain: '',
    positioning: '',
    productDescription: '',
    audience: '',
    competitors: '',
    prompts: '',
  })
  const set = (key: keyof typeof form) => (e: { target: { value: string } }) =>
    setForm({ ...form, [key]: e.target.value })

  const next = useMutation({
    mutationFn: async () => {
      if (step === 0 && !project) {
        const created = await api<ProjectSummary>('/v1/projects', {
          method: 'POST',
          body: JSON.stringify({ name: form.name.trim(), domain: form.domain.trim() }),
        })
        setProject(created)
        await queryClient.invalidateQueries({ queryKey: ['projects'] })
        await queryClient.invalidateQueries({ queryKey: ['me'] })
      }
      if (step === 1) {
        await post(
          '/v1/brand',
          {
            positioning: form.positioning.trim(),
            ...(form.productDescription.trim()
              ? { productDescription: form.productDescription.trim() }
              : {}),
            ...(form.audience.trim() ? { audience: form.audience.trim() } : {}),
          },
          'PATCH',
        )
      }
      if (step === 2 && lines(form.competitors).length > 0) {
        await post('/v1/brand/competitors', { competitors: lines(form.competitors) })
      }
      if (step === 3 && lines(form.prompts).length > 0) {
        await post('/v1/prompts/bulk', { prompts: lines(form.prompts).map((text) => ({ text })) })
      }
    },
    onSuccess: () => setStep(step + 1),
  })

  function submit(event: FormEvent) {
    event.preventDefault()
    next.mutate()
  }
  const skip = () => {
    next.reset()
    setStep(step + 1)
  }

  return (
    <div className="mx-auto flex w-full max-w-xl flex-col gap-5">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Set up a project</h1>
        <ol className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-sm" aria-label="Steps">
          {STEPS.map((name, i) => (
            <li
              key={name}
              aria-current={i === step ? 'step' : undefined}
              className={cn(
                i === step
                  ? 'font-semibold text-primary'
                  : i < step
                    ? 'text-foreground'
                    : 'text-muted',
              )}
            >
              {i + 1}. {name}
            </li>
          ))}
        </ol>
      </div>

      <Card>
        {step < 4 ? (
          <form onSubmit={submit} className="flex flex-col gap-4">
            {step === 0 && (
              <>
                <Field id="name" label="Brand name">
                  <Input
                    id="name"
                    required
                    maxLength={80}
                    value={form.name}
                    onChange={set('name')}
                    disabled={!!project}
                    autoComplete="organization"
                  />
                </Field>
                <Field id="domain" label="Website domain">
                  <Input
                    id="domain"
                    required
                    placeholder="acme.com"
                    value={form.domain}
                    onChange={set('domain')}
                    disabled={!!project}
                    autoComplete="url"
                  />
                </Field>
                {project && (
                  <p role="status" className="text-sm">
                    Project {project.name} is created.
                  </p>
                )}
              </>
            )}
            {step === 1 && (
              <>
                <Field id="positioning" label="What does the brand stand for?">
                  <textarea
                    id="positioning"
                    className={textarea}
                    required
                    minLength={3}
                    maxLength={2000}
                    value={form.positioning}
                    onChange={set('positioning')}
                  />
                </Field>
                <Field id="product" label="What does it sell?">
                  <Input
                    id="product"
                    value={form.productDescription}
                    onChange={set('productDescription')}
                  />
                </Field>
                <Field id="audience" label="Who buys it?">
                  <Input id="audience" value={form.audience} onChange={set('audience')} />
                </Field>
              </>
            )}
            {step === 2 && (
              <Field id="competitors" label="Competitors, one on each line">
                <textarea
                  id="competitors"
                  className={textarea}
                  placeholder={'Canyon\nTrek'}
                  value={form.competitors}
                  onChange={set('competitors')}
                />
              </Field>
            )}
            {step === 3 && (
              <Field id="prompts" label="Prompts, one on each line">
                <textarea
                  id="prompts"
                  className={textarea}
                  placeholder="What is the best commuter e-bike under 2000 euros?"
                  value={form.prompts}
                  onChange={set('prompts')}
                />
              </Field>
            )}
            {next.error && <Problem error={next.error} />}
            <div className="flex flex-wrap gap-2">
              <Button type="submit" disabled={next.isPending}>
                {next.isPending ? 'Saving…' : 'Continue'}
              </Button>
              {step > 0 && (
                <Button type="button" variant="ghost" onClick={skip}>
                  Skip this step
                </Button>
              )}
            </div>
          </form>
        ) : (
          <div className="flex flex-col gap-4">
            <p className="text-sm">
              The first scan asks every prompt on every platform. Run it now or later from the
              dashboard.
            </p>
            <ScanButton />
            {project && (
              <Link
                to="/p/$projectId"
                params={{ projectId: project.id }}
                className="text-sm font-medium text-primary underline underline-offset-4"
              >
                Open the dashboard
              </Link>
            )}
          </div>
        )}
      </Card>
    </div>
  )
}
