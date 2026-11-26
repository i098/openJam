import { useQuery } from '@tanstack/react-query'
import { createRoute, Link } from '@tanstack/react-router'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { Card, Loading, Pill, Problem, ScanButton, TableBox, td, th } from '@/components/ui/kit'
import { api } from '@/lib/api'
import { cn } from '@/lib/cn'
import type {
  AnalyticsOverview,
  LeaderboardEntry,
  MetricValue,
  TopicRanking,
  TrendSeries,
} from '@/lib/views'
import { shellRoute } from '@/routes/shell'

export const dashboardRoute = createRoute({
  getParentRoute: () => shellRoute,
  path: '/p/$projectId',
  component: Dashboard,
})

const RANGES = [7, 30, 90] as const
const SERIES = [
  'var(--color-series-1)',
  'var(--color-series-2)',
  'var(--color-series-3)',
  'var(--color-series-4)',
]

const num = (value: number | null | undefined, digits = 0, unit = '') =>
  value == null ? '—' : `${value.toFixed(digits)}${unit}`

function Dashboard() {
  const { projectId } = dashboardRoute.useParams()
  const [days, setDays] = useState<(typeof RANGES)[number]>(30)
  const overview = useQuery({
    queryKey: ['overview', days],
    queryFn: () => api<AnalyticsOverview>(`/v1/analytics/overview?rangeDays=${days}`),
  })
  const data = overview.data

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0">
          <h1 className="truncate text-2xl font-semibold tracking-tight">
            {data?.project.brandName ?? 'Dashboard'}
          </h1>
          {data && <p className="text-sm">{data.project.siteDomain}</p>}
        </div>
        <div className="flex flex-wrap items-start gap-3">
          <fieldset className="flex min-w-0 rounded-md border border-border p-0.5">
            <legend className="sr-only">Range</legend>
            {RANGES.map((range) => (
              <button
                key={range}
                type="button"
                aria-pressed={days === range}
                onClick={() => setDays(range)}
                className={cn(
                  'h-8 whitespace-nowrap rounded px-3 text-sm font-medium',
                  days === range ? 'bg-primary text-primary-foreground' : 'hover:bg-surface',
                )}
              >
                {range} days
              </button>
            ))}
          </fieldset>
          <ScanButton onStarted={() => overview.refetch()} />
        </div>
      </div>

      {overview.isPending && <Loading />}
      {overview.error && <Problem error={overview.error} />}
      {data && <Body data={data} projectId={projectId} />}
    </div>
  )
}

function Body({ data, projectId }: { data: AnalyticsOverview; projectId: string }) {
  const { overview, window } = data
  const notices = Object.values(data.sectionNotices ?? {})
  const owned = overview.trend.brands.find((b) => b.isOwned)?.key
  const providerName = Object.fromEntries(data.providers.map((p) => [p.id, p.displayName]))

  if (window.answerCount === 0) {
    return (
      <Card title="No answers yet">
        <p className="text-sm">
          Add prompts, then run a scan. Answers from {data.providers.length} platforms fill this
          page.
        </p>
        <Link
          to="/p/$projectId/prompts"
          params={{ projectId }}
          className="mt-3 inline-block text-sm font-medium text-primary underline underline-offset-4"
        >
          Go to prompts
        </Link>
      </Card>
    )
  }

  return (
    <>
      {data.project.id !== projectId && (
        <p
          role="status"
          className="rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-sm"
        >
          The API reports only the newest project of this workspace. These numbers are for{' '}
          {data.project.brandName}.
        </p>
      )}
      {notices.map((text) => (
        <p
          key={text}
          role="status"
          className="rounded-md border border-warning/40 bg-warning/10 px-3 py-2 text-sm"
        >
          {text}
        </p>
      ))}
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <MetricCard
          label="Visibility score"
          metric={overview.visibilityScore}
          unit="%"
          hint="Answers that name your brand"
        />
        <MetricCard
          label="Citation share"
          metric={overview.citationShare}
          unit="%"
          hint="Answers that cite your domain"
        />
        <MetricCard
          label="Average position"
          metric={overview.averagePosition}
          digits={1}
          lowerIsBetter
          hint="Place in lists, 1 is first"
        />
      </div>
      <p className="text-sm">
        {window.label}: {window.answerCount} answers on {window.providerCount} platforms,{' '}
        {data.promptsMeasured} of {data.promptsTracked} prompts measured.
      </p>

      <Card title="Visibility over time">
        <TrendChart trend={overview.trend} />
      </Card>

      <Card
        title="Leaderboard"
        action={
          <Button size="sm" variant="ghost" onClick={() => downloadCsv(overview.leaderboard)}>
            Export CSV
          </Button>
        }
      >
        <TableBox>
          <thead>
            <tr>
              <th className={th}>#</th>
              <th className={th}>Brand</th>
              <th className={th}>Visibility</th>
              <th className={th}>Citations</th>
              <th className={th}>Position</th>
              <th className={th}>Mentions</th>
            </tr>
          </thead>
          <tbody>
            {overview.leaderboard.map((row) => (
              <tr key={row.brand.key} className={row.brand.isOwned ? 'bg-primary-soft' : ''}>
                <td className={cn(td, 'pl-2')}>{row.rank ?? '—'}</td>
                <td className={cn(td, 'font-medium')}>
                  {row.brand.name} {row.brand.isOwned && <Pill tone="good">You</Pill>}
                </td>
                <td className={td}>
                  {num(row.visibilityScore, 0, '%')} <Delta value={row.delta} unit=" pts" />
                </td>
                <td className={td}>{num(row.citationShare, 0, '%')}</td>
                <td className={td}>{num(row.averagePosition, 1)}</td>
                <td className={td}>
                  {row.mentions ?? '—'}
                  {overview.mentionCohortCount ? ` of ${overview.mentionCohortCount}` : ''}
                </td>
              </tr>
            ))}
          </tbody>
        </TableBox>
      </Card>

      <div className="grid grid-cols-1 gap-5 lg:grid-cols-2">
        <Card title="By platform">
          <TableBox>
            <thead>
              <tr>
                <th className={th}>Platform</th>
                <th className={th}>Visibility</th>
                <th className={th}>Citations</th>
                <th className={th}>Answers</th>
              </tr>
            </thead>
            <tbody>
              {overview.providerBreakdown.map((row) => (
                <tr key={row.providerId}>
                  <td className={cn(td, 'font-medium')}>
                    {providerName[row.providerId] ?? row.providerId}{' '}
                    {row.status !== 'active' && (
                      <Pill tone={row.status === 'degraded' ? 'bad' : 'warn'}>{row.status}</Pill>
                    )}
                  </td>
                  <td className={td}>{num(row.visibilityScore.value, 0, '%')}</td>
                  <td className={td}>{num(row.citationShare.value, 0, '%')}</td>
                  <td className={td}>{row.answerCount}</td>
                </tr>
              ))}
            </tbody>
          </TableBox>
        </Card>

        <Card title="Topics">
          <TopicTable
            topics={overview.topicRankings}
            owned={owned}
            brands={overview.trend.brands}
          />
        </Card>
      </div>

      {overview.changeFeed.length > 0 && (
        <Card title="Recent changes">
          <ul className="divide-y divide-border">
            {overview.changeFeed.map((item) => (
              <li key={item.id} className="flex flex-wrap gap-x-3 py-2 text-sm">
                <time dateTime={item.date} className="font-medium">
                  {item.date.slice(0, 10)}
                </time>
                <span>{item.label}</span>
              </li>
            ))}
          </ul>
        </Card>
      )}
    </>
  )
}

function MetricCard({
  label,
  metric,
  unit = '',
  digits = 0,
  lowerIsBetter = false,
  hint,
}: {
  label: string
  metric: MetricValue
  unit?: string
  digits?: number
  lowerIsBetter?: boolean
  hint: string
}) {
  return (
    <div className="rounded-lg border border-border bg-surface p-4">
      <p className="text-sm font-medium">{label}</p>
      <p className="mt-1 text-4xl font-semibold tracking-tight">
        {num(metric.value, digits, unit)}
      </p>
      <p className="mt-1 text-sm">
        {metric.deltaAvailable ? (
          <Delta
            value={metric.delta}
            unit={unit ? ' pts' : ' places'}
            lowerIsBetter={lowerIsBetter}
          />
        ) : (
          'No earlier window to compare'
        )}
      </p>
      <p className="mt-2 text-xs text-muted">{hint}</p>
    </div>
  )
}

function Delta({
  value,
  unit,
  lowerIsBetter = false,
}: {
  value: number | null
  unit: string
  lowerIsBetter?: boolean
}) {
  if (value == null || value === 0) return null
  const good = lowerIsBetter ? value < 0 : value > 0
  return (
    <span className={cn('text-xs font-medium', good ? 'text-success' : 'text-danger')}>
      {value > 0 ? '+' : '−'}
      {Math.abs(value).toFixed(1)}
      {unit}
    </span>
  )
}

function TrendChart({ trend }: { trend: TrendSeries }) {
  const { points, brands } = trend
  if (points.length < 2) return <p className="text-sm">Not enough days to draw a trend yet.</p>
  const width = 640
  const height = 200
  const x = (i: number) => (i / (points.length - 1)) * width
  const y = (score: number) => height - (score / 100) * height
  // A missing score breaks the line instead of drawing a zero.
  const path = (key: string) => {
    let d = ''
    let pen = false
    points.forEach((p, i) => {
      const score = p.scores[key]
      if (score == null) {
        pen = false
        return
      }
      d += `${pen ? 'L' : 'M'}${x(i).toFixed(1)},${y(score).toFixed(1)} `
      pen = true
    })
    return d
  }

  return (
    <figure>
      <svg
        viewBox={`0 0 ${width} ${height}`}
        className="h-auto w-full overflow-visible"
        role="img"
        aria-label={`Visibility score by day for ${brands.map((b) => b.name).join(', ')}`}
      >
        {[0, 25, 50, 75, 100].map((tick) => (
          <line
            key={tick}
            x1={0}
            x2={width}
            y1={y(tick)}
            y2={y(tick)}
            stroke="var(--color-border)"
            strokeWidth={1}
          />
        ))}
        {brands.map((brand, i) => (
          <path
            key={brand.key}
            d={path(brand.key)}
            fill="none"
            stroke={SERIES[i % SERIES.length]}
            strokeWidth={brand.isOwned ? 3 : 2}
            strokeLinejoin="round"
            vectorEffect="non-scaling-stroke"
          />
        ))}
      </svg>
      <div className="mt-1 flex justify-between text-xs">
        <span>{points[0].date.slice(5, 10)}</span>
        <span>Score 0 to 100</span>
        <span>{points[points.length - 1].date.slice(5, 10)}</span>
      </div>
      <figcaption className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-sm">
        {brands.map((brand, i) => (
          <span key={brand.key} className="inline-flex items-center gap-1.5">
            <span
              className="size-2.5 rounded-full"
              style={{ background: SERIES[i % SERIES.length] }}
            />
            {brand.name}
          </span>
        ))}
      </figcaption>
    </figure>
  )
}

function TopicTable({
  topics,
  owned,
  brands,
}: {
  topics: TopicRanking[]
  owned: string | undefined
  brands: TrendSeries['brands']
}) {
  const name = (key: string) => brands.find((b) => b.key === key)?.name ?? key
  if (topics.length === 0)
    return <p className="text-sm">No topics yet. Tag prompts with a topic.</p>
  return (
    <TableBox>
      <thead>
        <tr>
          <th className={th}>Topic</th>
          <th className={th}>You</th>
          <th className={th}>Leader</th>
        </tr>
      </thead>
      <tbody>
        {topics.map((topic) => {
          const ranked = [...topic.ranked].sort(
            (a, b) => (b.visibilityScore ?? -1) - (a.visibilityScore ?? -1),
          )
          const mine = ranked.find((r) => r.brandKey === owned)
          const leader = ranked[0]
          return (
            <tr key={topic.topic}>
              <td className={cn(td, 'font-medium')}>
                {topic.topic}{' '}
                <span className="text-xs text-muted">{topic.promptCount} prompts</span>
              </td>
              <td className={td}>{num(mine?.visibilityScore, 0, '%')}</td>
              <td className={td}>
                {leader ? `${name(leader.brandKey)} ${num(leader.visibilityScore, 0, '%')}` : '—'}
              </td>
            </tr>
          )
        })}
      </tbody>
    </TableBox>
  )
}

function downloadCsv(rows: LeaderboardEntry[]) {
  const lines = [
    'rank,brand,visibility_score,citation_share,average_position,mentions',
    ...rows.map((r) =>
      [
        r.rank,
        JSON.stringify(r.brand.name),
        r.visibilityScore,
        r.citationShare,
        r.averagePosition,
        r.mentions,
      ]
        .map((v) => v ?? '')
        .join(','),
    ),
  ]
  const url = URL.createObjectURL(new Blob([lines.join('\n')], { type: 'text/csv' }))
  const link = Object.assign(document.createElement('a'), {
    href: url,
    download: 'leaderboard.csv',
  })
  link.click()
  URL.revokeObjectURL(url)
}
