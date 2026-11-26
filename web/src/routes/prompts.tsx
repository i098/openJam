import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { createRoute, Link } from '@tanstack/react-router'
import { type FormEvent, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Card, Loading, Pill, Problem, ScanButton } from '@/components/ui/kit'
import { api } from '@/lib/api'
import type { AddPromptResult, Prompt, PromptAnalytics, PromptMetrics, Prompts } from '@/lib/views'
import { shellRoute } from '@/routes/shell'

export const promptsRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: '/p/$projectId/prompts',
  component: PromptsPage,
})

const pct = (value: number | null | undefined) => (value == null ? '—' : `${value.toFixed(0)}%`)

function PromptsPage() {
  const { projectId } = promptsRoute.useParams()
  const client = useQueryClient()
  const [topic, setTopic] = useState('')
  const [search, setSearch] = useState('')
  const list = useQuery({
    queryKey: ['prompts'],
    queryFn: () => api<Prompts>('/v1/prompts?list=tracked&limit=100'),
  })
  // Metrics are optional: the list still works when analytics is not ready.
  const metrics = useQuery({
    queryKey: ['prompt-metrics'],
    queryFn: () => api<PromptAnalytics>('/v1/analytics/prompts?rangeDays=30&limit=100'),
  })
  const byId = new Map(metrics.data?.prompts.map((p) => [p.id, p.metrics]))
  const refresh = () => client.invalidateQueries({ queryKey: ['prompts'] })

  const prompts = list.data?.prompts ?? []
  const topics = [...new Set(prompts.map((p) => p.topic).filter((t): t is string => !!t))]
  const shown = prompts.filter(
    (p) =>
      (!topic || p.topic === topic) && p.text.toLowerCase().includes(search.trim().toLowerCase()),
  )

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Prompts</h1>
          {list.data?.counts && (
            <p className="text-sm">
              {list.data.counts.tracked} tracked, {list.data.counts.paused} paused
            </p>
          )}
        </div>
        <ScanButton />
      </div>

      <AddPrompt onAdded={refresh} />

      <Card>
        <div className="mb-3 flex flex-wrap gap-2">
          <Input
            aria-label="Search prompts"
            placeholder="Search prompts"
            className="sm:max-w-xs"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          <select
            aria-label="Topic"
            className="h-10 rounded-md border border-border bg-background px-2 text-sm"
            value={topic}
            onChange={(e) => setTopic(e.target.value)}
          >
            <option value="">All topics</option>
            {topics.map((t) => (
              <option key={t}>{t}</option>
            ))}
          </select>
        </div>
        {list.isPending && <Loading />}
        {list.error && <Problem error={list.error} />}
        {list.data && shown.length === 0 && (
          <p className="py-4 text-sm">
            {prompts.length === 0
              ? 'No prompts yet. Add the first one above.'
              : 'No prompt matches.'}
          </p>
        )}
        <ul className="divide-y divide-border">
          {shown.map((prompt) => (
            <PromptRow
              key={prompt.id}
              prompt={prompt}
              projectId={projectId}
              metrics={byId.get(prompt.id)}
              onChanged={refresh}
            />
          ))}
        </ul>
      </Card>
    </div>
  )
}

function AddPrompt({ onAdded }: { onAdded: () => void }) {
  const [text, setText] = useState('')
  const [topic, setTopic] = useState('')
  const add = useMutation({
    mutationFn: () =>
      api<AddPromptResult>('/v1/prompts', {
        method: 'POST',
        body: JSON.stringify({
          text: text.trim(),
          ...(topic.trim() ? { topic: topic.trim() } : {}),
        }),
      }),
    onSuccess: () => {
      setText('')
      onAdded()
    },
  })
  function submit(event: FormEvent) {
    event.preventDefault()
    add.mutate()
  }
  return (
    <Card title="Add a prompt">
      <form onSubmit={submit} className="flex flex-col gap-2 sm:flex-row">
        <Input
          aria-label="Prompt text"
          placeholder="What would a customer ask an AI?"
          required
          minLength={3}
          maxLength={500}
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
        <Input
          aria-label="Topic"
          placeholder="Topic (optional)"
          className="sm:max-w-48"
          value={topic}
          onChange={(e) => setTopic(e.target.value)}
        />
        <Button type="submit" disabled={add.isPending}>
          Add
        </Button>
      </form>
      {add.data?.existing && (
        <p role="status" className="mt-2 text-sm">
          You already track this prompt.
        </p>
      )}
      {add.data && !add.data.existing && add.data.cap >= 0 && (
        <p role="status" className="mt-2 text-sm">
          {add.data.activeCount} of {add.data.cap} prompt slots used.
        </p>
      )}
      {add.error && (
        <div className="mt-2">
          <Problem error={add.error} />
        </div>
      )}
    </Card>
  )
}

function PromptRow({
  prompt,
  projectId,
  metrics,
  onChanged,
}: {
  prompt: Prompt
  projectId: string
  metrics: PromptMetrics | undefined
  onChanged: () => void
}) {
  const [editing, setEditing] = useState(false)
  const [text, setText] = useState(prompt.text)
  const paused = prompt.status === 'paused'
  const change = useMutation({
    mutationFn: (request: { path: string; method: string; body?: unknown }) =>
      api(request.path, {
        method: request.method,
        body: request.body === undefined ? undefined : JSON.stringify(request.body),
      }),
    onSuccess: () => {
      setEditing(false)
      onChanged()
    },
  })

  return (
    <li className="flex flex-col gap-2 py-3">
      {editing ? (
        <form
          className="flex flex-col gap-2 sm:flex-row"
          onSubmit={(e) => {
            e.preventDefault()
            change.mutate({
              path: `/v1/prompts/${prompt.id}`,
              method: 'PATCH',
              body: { text: text.trim() },
            })
          }}
        >
          <Input
            aria-label="Prompt text"
            required
            minLength={3}
            maxLength={500}
            value={text}
            onChange={(e) => setText(e.target.value)}
          />
          <Button type="submit" size="sm" disabled={change.isPending}>
            Save
          </Button>
          <Button type="button" size="sm" variant="ghost" onClick={() => setEditing(false)}>
            Cancel
          </Button>
        </form>
      ) : (
        <Link
          to="/p/$projectId/prompts/$promptId"
          params={{ projectId, promptId: prompt.id }}
          className="font-medium underline-offset-4 hover:underline"
        >
          {prompt.text}
        </Link>
      )}
      <div className="flex flex-wrap items-center gap-2 text-sm">
        {prompt.topic && <Pill>{prompt.topic}</Pill>}
        <Pill tone={paused ? 'warn' : 'good'}>{prompt.status}</Pill>
        {!prompt.scored && <Pill tone="warn">Not scored: names a brand</Pill>}
        {metrics && (
          <span>
            Visibility {pct(metrics.visibilityScore.value)}, citations{' '}
            {pct(metrics.citationShare.value)}, {metrics.executions} answers
          </span>
        )}
        <span className="flex-1" />
        <Button size="sm" variant="ghost" onClick={() => setEditing(true)}>
          Edit
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={change.isPending}
          onClick={() =>
            change.mutate({
              path: `/v1/prompts/${prompt.id}/pause`,
              method: 'POST',
              body: { paused: !paused },
            })
          }
        >
          {paused ? 'Resume' : 'Pause'}
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={change.isPending}
          onClick={() => {
            if (window.confirm('Remove this prompt? Its answers stay in the history.')) {
              change.mutate({ path: `/v1/prompts/${prompt.id}`, method: 'DELETE' })
            }
          }}
        >
          Remove
        </Button>
      </div>
      {change.error && <Problem error={change.error} />}
    </li>
  )
}
