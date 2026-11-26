import { useInfiniteQuery, useQuery } from '@tanstack/react-query'
import { createRoute, Link } from '@tanstack/react-router'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Card, Loading, Pill, Problem } from '@/components/ui/kit'
import { api } from '@/lib/api'
import { type Answer, type AnswerPage, type Prompt, platformName } from '@/lib/views'
import { shellRoute } from '@/routes/shell'

export const promptRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: '/p/$projectId/prompts/$promptId',
  component: PromptDetail,
})

const PLATFORMS = ['openai', 'claude', 'perplexity', 'gemini']
const PAGE = 10

function PromptDetail() {
  const { projectId, promptId } = promptRoute.useParams()
  const [platform, setPlatform] = useState('')
  const [cited, setCited] = useState(false)
  const [full, setFull] = useState(false)

  const prompt = useQuery({
    queryKey: ['prompt', promptId],
    queryFn: () => api<Prompt>(`/v1/prompts/${promptId}`),
  })
  const answers = useInfiniteQuery({
    queryKey: ['answers', promptId, platform, cited, full],
    initialPageParam: '',
    queryFn: ({ pageParam }) => {
      const query = new URLSearchParams({ limit: String(PAGE) })
      if (platform) query.set('platform', platform)
      if (cited) query.set('cited', 'true')
      if (full) query.set('full', 'true')
      if (pageParam) query.set('cursor', pageParam)
      return api<AnswerPage>(`/v1/prompts/${promptId}/answers?${query}`)
    },
    getNextPageParam: (last) => last.pageInfo.nextCursor ?? undefined,
  })
  const rows = answers.data?.pages.flatMap((page) => page.answers) ?? []
  const total = answers.data?.pages[0]?.pageInfo.totalCount

  return (
    <div className="flex flex-col gap-5">
      <Link
        to="/p/$projectId/prompts"
        params={{ projectId }}
        className="text-sm font-medium text-primary underline underline-offset-4"
      >
        All prompts
      </Link>
      {prompt.isPending && <Loading />}
      {prompt.error && <Problem error={prompt.error} />}
      {prompt.data && (
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">{prompt.data.text}</h1>
          <div className="mt-2 flex flex-wrap gap-2">
            {prompt.data.topic && <Pill>{prompt.data.topic}</Pill>}
            <Pill tone={prompt.data.status === 'paused' ? 'warn' : 'good'}>
              {prompt.data.status}
            </Pill>
            {!prompt.data.scored && <Pill tone="warn">Not scored: names a brand</Pill>}
          </div>
        </div>
      )}

      <Card title={total == null ? 'Answers' : `Answers (${total})`}>
        <div className="mb-4 flex flex-wrap items-center gap-x-4 gap-y-2 text-sm">
          <select
            aria-label="Platform"
            className="h-9 rounded-md border border-border bg-background px-2"
            value={platform}
            onChange={(e) => setPlatform(e.target.value)}
          >
            <option value="">All platforms</option>
            {PLATFORMS.map((p) => (
              <option key={p} value={p}>
                {platformName(p)}
              </option>
            ))}
          </select>
          <label className="inline-flex items-center gap-2">
            <input type="checkbox" checked={cited} onChange={(e) => setCited(e.target.checked)} />
            Cites your domain
          </label>
          <label className="inline-flex items-center gap-2">
            <input type="checkbox" checked={full} onChange={(e) => setFull(e.target.checked)} />
            Full text
          </label>
        </div>
        {answers.isPending && <Loading />}
        {answers.error && <Problem error={answers.error} />}
        {answers.data && rows.length === 0 && (
          <p className="py-4 text-sm">No answers match. Run a scan or relax the filters.</p>
        )}
        <ul className="flex flex-col gap-4">
          {rows.map((answer) => (
            <AnswerCard key={answer.id} answer={answer} />
          ))}
        </ul>
        {answers.hasNextPage && (
          <Button
            variant="ghost"
            className="mt-4 w-full border border-border"
            disabled={answers.isFetchingNextPage}
            onClick={() => answers.fetchNextPage()}
          >
            {answers.isFetchingNextPage ? 'Loading…' : 'Load more answers'}
          </Button>
        )}
      </Card>
    </div>
  )
}

function AnswerCard({ answer }: { answer: Answer }) {
  const sources = (answer.citedUrls ?? []).map((u) =>
    typeof u === 'string' ? { url: u, title: null } : u,
  )
  return (
    <li className="rounded-lg border border-border p-4">
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="text-sm font-semibold">{platformName(answer.platform)}</h3>
        <time dateTime={answer.observedAt} className="text-sm">
          {new Date(answer.observedAt).toLocaleString(undefined, {
            dateStyle: 'medium',
            timeStyle: 'short',
          })}
        </time>
        <span className="flex-1" />
        <Pill tone={answer.brandMentioned ? 'good' : 'bad'}>
          {answer.brandMentioned ? 'Mentions you' : 'Does not mention you'}
        </Pill>
        {answer.brandRecommended && <Pill tone="good">Recommends you</Pill>}
        {answer.ownDomainCited && <Pill tone="good">Cites your domain</Pill>}
        {answer.position != null && <Pill>Position {answer.position}</Pill>}
        {answer.sentiment && <Pill>{answer.sentiment}</Pill>}
      </div>
      {answer.brandsMentioned.length > 0 && (
        <p className="mt-3 flex flex-wrap gap-1.5 text-sm">
          {answer.brandsMentioned.map((brand) => (
            <Pill key={brand.name} tone={brand.owned ? 'good' : 'neutral'}>
              {brand.listRank ? `${brand.listRank}. ` : ''}
              {brand.name}
            </Pill>
          ))}
        </p>
      )}
      <p className="mt-3 whitespace-pre-line break-words text-sm leading-relaxed">
        {answer.answer.text ?? 'No text stored for this answer.'}
        {answer.answer.truncated && '…'}
      </p>
      {sources.length > 0 && (
        <details className="mt-3 text-sm">
          <summary className="cursor-pointer font-medium">Sources ({sources.length})</summary>
          <ul className="mt-2 flex flex-col gap-1">
            {sources.map((s) => (
              <li key={s.url} className="break-all">
                <a
                  href={s.url}
                  target="_blank"
                  rel="noreferrer"
                  className="text-primary underline underline-offset-4"
                >
                  {s.title || s.url}
                </a>
              </li>
            ))}
          </ul>
        </details>
      )}
    </li>
  )
}
