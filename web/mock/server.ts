// Mock of the openJam API for the web app: `bun mock/server.ts`, then `OPENJAM_API_URL=http://127.0.0.1:8090 bun run dev`.
// It serves fixtures shaped like the contract. `MOCK_EMPTY=1` starts a workspace with no project.
import type {
  AnalyticsOverview,
  Answer,
  PlatformRow,
  Prompt,
  PromptAnalytics,
} from '../src/lib/views'

const port = Number(process.env.MOCK_PORT ?? 8090)
const day = 86_400_000
const now = Date.now()

const OWNED = {
  key: 'owned:northwind',
  name: 'Northwind Cycles',
  domain: 'northwindcycles.com',
  isOwned: true,
}
const RIVALS = [
  { key: 'canyon', name: 'Canyon', domain: 'canyon.com', isOwned: false },
  { key: 'trek', name: 'Trek', domain: 'trekbikes.com', isOwned: false },
  { key: 'specialized', name: 'Specialized', domain: 'specialized.com', isOwned: false },
]
const BRANDS = [OWNED, ...RIVALS]
const PLATFORMS = [
  { id: 'openai', displayName: 'ChatGPT', status: 'active' },
  { id: 'claude', displayName: 'Claude', status: 'active' },
  { id: 'perplexity', displayName: 'Perplexity', status: 'active' },
  { id: 'gemini', displayName: 'Gemini', status: 'off' },
]
const ANSWERING = PLATFORMS.filter((p) => p.status === 'active')

const empty = process.env.MOCK_EMPTY === '1'
let projects = empty
  ? []
  : [
      {
        id: 'a3f1c2d4-0000-4000-8000-000000000001',
        name: 'Northwind Cycles',
        domain: 'northwindcycles.com',
        siteUrl: 'https://northwindcycles.com',
        status: 'active',
        createdAt: new Date(now - 40 * day).toISOString(),
      },
    ]
const seedPrompts: Array<[string, string, string]> = empty
  ? []
  : [
      ['What is the best commuter e-bike under 2000 euros?', 'E-bikes', 'active'],
      ['Which brands make a reliable gravel bike for beginners?', 'Gravel', 'active'],
      ['How do I choose a bike for a 15 km daily commute?', 'Commuting', 'active'],
      ['Best folding bikes for train travel', 'Commuting', 'active'],
      ['Are direct-to-consumer bike brands worth it?', 'Buying', 'active'],
      ['Top e-bike brands with a long battery range', 'E-bikes', 'paused'],
      ['Northwind Cycles review', 'Brand', 'active'],
    ]
let prompts: Prompt[] = seedPrompts.map(([text, topic, status], i) => ({
  id: `b0000000-0000-4000-8000-00000000000${i + 1}`,
  text,
  topic,
  persona: 'discoverer',
  intent: null,
  list: 'tracked',
  status,
  branded: topic === 'Brand',
  scored: topic !== 'Brand',
}))

function rand(seed: number) {
  const x = Math.sin(seed * 9301 + 49297) * 233280
  return x - Math.floor(x)
}

function answersFor(prompt: Prompt): Answer[] {
  const out: Answer[] = []
  ANSWERING.forEach((platform, pi) => {
    for (let k = 0; k < 4; k++) {
      const seed = prompt.id.length * 7 + pi * 13 + k * 5 + Number(prompt.id.slice(-1))
      const mentioned = rand(seed) > 0.45
      const order = [...RIVALS].sort(
        (a, b) => rand(seed + a.key.length) - rand(seed + b.key.length),
      )
      const list = mentioned ? [...order.slice(0, 1), OWNED, ...order.slice(1)] : order
      const cites = mentioned && rand(seed + 3) > 0.4
      out.push({
        id: `${prompt.id}-${platform.id}-${k}`,
        promptId: prompt.id,
        promptText: prompt.text,
        platform: platform.id,
        observedAt: new Date(now - (k * 6 + pi) * day).toISOString(),
        answer: {
          text: `For "${prompt.text}" the usual picks are:\n${list.map((b, i) => `${i + 1}. ${b.name}: a solid option with good value and wide dealer support.`).join('\n')}\nCheck the warranty, the frame size range and the service network before you buy.`,
          source: 'full',
          truncated: false,
          length: 320,
        },
        brandMentioned: mentioned,
        brandRecommended: mentioned && rand(seed + 1) > 0.5,
        ownDomainCited: cites,
        position: mentioned ? 2 : null,
        sentiment: mentioned ? 'positive' : null,
        brandsMentioned: list.map((b, i) => ({
          name: b.name,
          owned: b.isOwned,
          recommended: i === 0,
          listRank: i + 1,
          sentiment: null,
        })),
        citedUrls: [
          ...(cites ? [`https://${OWNED.domain}/e-bikes`] : []),
          `https://www.${RIVALS[0].domain}/guides/commuting`,
          'https://www.bikeradar.com/advice/buyers-guides/best-commuter-bikes/',
        ],
        citedUrlCount: cites ? 3 : 2,
      })
    }
  })
  return out.sort((a, b) => b.observedAt.localeCompare(a.observedAt))
}

function overview(rangeDays: number): AnalyticsOverview {
  const project = projects[0]
  const answerCount = prompts.length === 0 ? 0 : rangeDays * 9
  const metric = (value: number, delta: number) => ({ value, delta, deltaAvailable: true })
  const scores: Record<string, number> = { [OWNED.key]: 42, canyon: 58, trek: 51, specialized: 37 }
  const length = Math.min(rangeDays, 30)
  const points = Array.from({ length }, (_, i) => {
    const date = new Date(now - (length - 1 - i) * day).toISOString().slice(0, 10)
    return {
      date,
      scores: Object.fromEntries(
        BRANDS.map((b, bi) => [
          b.key,
          Math.max(
            0,
            Math.min(
              100,
              Math.round(
                scores[b.key] -
                  (length - i) * (b.isOwned ? -0.25 : 0.05) +
                  (rand(i * 7 + bi) - 0.5) * 8,
              ),
            ),
          ),
        ]),
      ),
    }
  })
  return {
    project: {
      id: project?.id ?? '',
      brandName: project?.name ?? '',
      siteDomain: project?.domain ?? '',
      status: 'active',
    },
    window: {
      label: `Last ${rangeDays} days`,
      comparisonLabel: `Previous ${rangeDays} days`,
      startDate: '',
      endDate: '',
      answerCount,
      providerCount: ANSWERING.length,
    },
    promptsTracked: prompts.length,
    promptsMeasured: Math.max(0, prompts.length - 2),
    providers: PLATFORMS,
    overview: {
      visibilityScore: metric(42, 6.4),
      citationShare: metric(27, -2.1),
      averagePosition: metric(2.4, -0.3),
      trend: { brands: BRANDS, points, unit: 'percent' },
      leaderboard: BRANDS.map((b, i) => ({
        brand: b,
        visibilityScore: scores[b.key],
        delta: [6.4, -1.2, 0.8, -0.5][i],
        citationShare: [27, 44, 39, 21][i],
        averagePosition: [2.4, 1.6, 2.1, 3.2][i],
        mentions: [Math.round(answerCount * 0.42), 52, 46, 33][i],
      }))
        .sort((a, b) => b.visibilityScore - a.visibilityScore)
        .map((row, i) => ({ ...row, rank: i + 1 })),
      mentionCohortCount: answerCount,
      providerBreakdown: PLATFORMS.map((p, i): PlatformRow => {
        if (p.status === 'off') {
          const none = { value: null, delta: null, deltaAvailable: false }
          return {
            providerId: p.id,
            visibilityScore: none,
            citationShare: none,
            averagePosition: none,
            answerCount: 0,
            status: 'off',
          }
        }
        return {
          providerId: p.id,
          visibilityScore: metric([48, 39, 41][i], 1),
          citationShare: metric([30, 18, 33][i], 1),
          averagePosition: metric(2.2, 0),
          answerCount: Math.round(answerCount / 3),
          status: 'active',
        }
      }),
      changeFeed: [
        {
          id: '1',
          date: new Date(now - 2 * day).toISOString(),
          kind: 'gain',
          label: 'Northwind Cycles entered the top 3 for "Gravel".',
        },
        {
          id: '2',
          date: new Date(now - 9 * day).toISOString(),
          kind: 'loss',
          label: 'Citation share on Claude fell by 4 points.',
        },
      ],
      topicRankings: ['E-bikes', 'Gravel', 'Commuting'].map((topic, i) => ({
        topic,
        promptCount: 2 + i,
        answerCount: 40,
        visibilityScore: {
          value: Math.round(scores[BRANDS[0].key] + i * 3),
          delta: null,
          deltaAvailable: false,
        },
      })),
    },
  }
}

function promptAnalytics(): PromptAnalytics {
  const m = (value: number) => ({ value, delta: null, deltaAvailable: false })
  return {
    window: { label: 'Last 30 days', answerCount: prompts.length * 12 },
    prompts: prompts.map((p, i) => ({
      id: p.id,
      text: p.text,
      topic: p.topic,
      intent: null,
      metrics: {
        visibilityRank: (i % 4) + 1,
        visibilityScore: m(25 + ((i * 11) % 60)),
        shareOfVoice: m(20),
        averagePosition: m(2),
        citationShare: m(10 + ((i * 7) % 40)),
        citationRank: 2,
        executions: 12,
      },
    })),
  }
}

const json = (body: unknown, status = 200, headers: Record<string, string> = {}) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', ...headers },
  })
const fail = (status: number, code: string, error: string) =>
  json({ error, code, request_id: crypto.randomUUID(), docs: 'https://example.test/docs' }, status)

const me = () => ({
  user: { id: 'u1', name: 'Demo owner', email: 'owner@example.test' },
  workspace_id: 'w1',
  project: projects[0] ? { id: projects[0].id, domain: projects[0].domain } : null,
})

type Body = {
  name?: string
  domain?: string
  text?: string
  topic?: string
  paused?: boolean
  prompts?: Array<{ text: string }>
}
type Ctx = { url: URL; body: Body }

const newPrompt = (text: string, topic: string | null = null): Prompt => ({
  id: crypto.randomUUID(),
  text,
  topic,
  persona: 'discoverer',
  intent: null,
  list: 'tracked',
  status: 'active',
  branded: false,
  scored: true,
})

const login = () =>
  json({ user: me().user, workspace_id: 'w1', role: 'owner' }, 200, {
    'set-cookie': 'mock_session=1; Path=/; HttpOnly; SameSite=Lax',
  })
const logout = () => json(null, 204, { 'set-cookie': 'mock_session=; Path=/; Max-Age=0' })

function createProject({ body }: Ctx) {
  const { name = '', domain = '' } = body
  if (!domain.includes('.'))
    return fail(400, 'invalid_request', 'domain must be a host name, for example acme.com')
  const created = {
    id: crypto.randomUUID(),
    name,
    domain,
    siteUrl: `https://${domain}`,
    status: 'setup',
    createdAt: new Date().toISOString(),
  }
  projects = [created, ...projects]
  return json(created, 201)
}

const startRun = () =>
  json(
    {
      runId: crypto.randomUUID(),
      projectId: projects[0]?.id ?? '',
      promptCount: prompts.length,
      platformCount: ANSWERING.length,
      expectedMinutes: 8,
    },
    202,
  )

function listPrompts() {
  const paused = prompts.filter((p) => p.status === 'paused').length
  return json({ prompts, counts: { tracked: prompts.length, paused, archived: 0 } })
}

function addPrompt({ body }: Ctx) {
  const text = String(body.text)
  const found = prompts.find((p) => p.text === text)
  if (found)
    return json({ promptId: found.id, existing: true, activeCount: prompts.length, cap: -1 })
  const created = newPrompt(text, body.topic ?? null)
  prompts = [created, ...prompts]
  return json({ promptId: created.id, existing: false, activeCount: prompts.length, cap: -1 }, 201)
}

function bulkPrompts({ body }: Ctx) {
  const created = (body.prompts ?? []).map((item) => newPrompt(item.text))
  prompts = [...created, ...prompts]
  return json(
    {
      results: created.map((p) => ({ promptId: p.id, created: true })),
      activeCount: prompts.length,
      cap: -1,
    },
    201,
  )
}

function answerPage(prompt: Prompt, { url }: Ctx) {
  const platform = url.searchParams.get('platform')
  const cited = url.searchParams.get('cited') === 'true'
  const limit = Number(url.searchParams.get('limit') ?? 10)
  const offset = Number(url.searchParams.get('cursor') ?? 0)
  const all = answersFor(prompt).filter(
    (a) => (!platform || a.platform === platform) && (!cited || a.ownDomainCited),
  )
  const nextCursor = offset + limit < all.length ? String(offset + limit) : null
  return json({
    answers: all.slice(offset, offset + limit),
    pageInfo: {
      limit,
      hasNextPage: nextCursor !== null,
      totalCount: all.length,
      ...(nextCursor ? { nextCursor } : {}),
    },
    nextCursor,
  })
}

function promptItem(prompt: Prompt, suffix: string | undefined, method: string, ctx: Ctx) {
  if (suffix === '/answers') return answerPage(prompt, ctx)
  if (suffix === '/pause') {
    prompt.status = ctx.body.paused ? 'paused' : 'active'
    return json({ promptId: prompt.id, status: prompt.status })
  }
  if (method === 'PATCH') {
    prompt.text = ctx.body.text ?? prompt.text
    return json({ promptId: prompt.id, changed: true })
  }
  if (method === 'DELETE') {
    prompts = prompts.filter((p) => p.id !== prompt.id)
    return json({ promptId: prompt.id })
  }
  return json(prompt)
}

const routes: Record<string, (ctx: Ctx) => Response> = {
  'GET /auth/me': () => json(me()),
  'GET /v1/projects': () => json({ projects }),
  'POST /v1/projects': createProject,
  'GET /v1/brand': () => json({ ok: true }),
  'PATCH /v1/brand': () => json({ ok: true }),
  'POST /v1/brand/competitors': () => json({ ok: true }),
  'POST /v1/visibility-runs': startRun,
  'GET /v1/analytics/overview': ({ url }) =>
    json(overview(Number(url.searchParams.get('rangeDays') ?? 30))),
  'GET /v1/analytics/prompts': () => json(promptAnalytics()),
  'GET /v1/prompts': listPrompts,
  'POST /v1/prompts': addPrompt,
  'POST /v1/prompts/bulk': bulkPrompts,
}

function handle(request: Request, body: Body): Response {
  const url = new URL(request.url)
  const { method } = request
  if (url.pathname === '/auth/login' || url.pathname === '/auth/signup') return login()
  if (url.pathname === '/auth/logout') return logout()
  if (!(request.headers.get('cookie') ?? '').includes('mock_session=1'))
    return fail(401, 'unauthorized', 'Sign in to continue.')
  const ctx = { url, body }
  const route = routes[`${method} ${url.pathname}`]
  if (route) return route(ctx)
  const match = url.pathname.match(/^\/v1\/prompts\/([^/]+)(\/answers|\/pause)?$/)
  if (!match) return fail(501, 'not_implemented', 'This route is not built yet.')
  const prompt = prompts.find((p) => p.id === match[1])
  return prompt
    ? promptItem(prompt, match[2], method, ctx)
    : fail(404, 'not_found', 'No such prompt.')
}

Bun.serve({
  port,
  async fetch(request) {
    const body: Body =
      request.method === 'GET' || request.method === 'DELETE'
        ? {}
        : await request.json().catch(() => ({}))
    return handle(request, body)
  },
})
console.log(`openJam mock API on http://127.0.0.1:${port}${empty ? ' (empty workspace)' : ''}`)
