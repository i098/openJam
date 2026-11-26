// Response shapes of `/v1` routes the API has not built yet (M1 brand and prompts, M2 analytics
// and answers). They follow the source contract and this file is deleted when those slices put
// their schemas in crates/api/contract/openapi.yaml and `bun run gen:types` covers them.
// shortcut: local copy of unbuilt contract shapes, drop it when M1 and M2 land.

export type MetricValue = { value: number | null; delta: number | null; deltaAvailable: boolean }

export type AnalyticsBrand = { key: string; name: string; domain: string; isOwned: boolean }

export type TrendSeries = {
  brands: AnalyticsBrand[]
  points: Array<{ date: string; scores: Record<string, number | null> }>
  unit: string
}

export type LeaderboardEntry = {
  rank: number | null
  brand: AnalyticsBrand
  visibilityScore: number | null
  delta: number | null
  citationShare: number | null
  averagePosition: number | null
  mentions: number | null
}

export type PlatformRow = {
  providerId: string
  visibilityScore: MetricValue
  citationShare: MetricValue
  averagePosition: MetricValue
  answerCount: number
  status: 'active' | 'pending' | 'degraded'
}

export type TopicRanking = {
  rankingAvailable?: boolean
  topic: string
  promptCount: number
  eligibleObservationCount: number
  ranked: Array<{ brandKey: string; visibilityScore: number | null }>
}

export type AnalyticsOverview = {
  project: { id: string; brandName: string; siteDomain: string; status: string }
  window: {
    label: string
    comparisonLabel: string
    startDate: string
    endDate: string
    answerCount: number
    providerCount: number
  }
  promptsTracked: number
  promptsMeasured: number
  providers: Array<{ id: string; displayName: string; status: string }>
  overview: {
    visibilityScore: MetricValue
    citationShare: MetricValue
    averagePosition: MetricValue
    trend: TrendSeries
    leaderboard: LeaderboardEntry[]
    mentionCohortCount?: number | null
    providerBreakdown: PlatformRow[]
    changeFeed: Array<{ id: string; date: string; kind: string; label: string }>
    topicRankings: TopicRanking[]
  }
  sectionNotices?: Record<string, string>
  metricNotes?: Record<string, string>
}

export type Prompt = {
  id: string
  text: string
  topic: string | null
  persona: string
  intent: string | null
  list: 'tracked' | 'archived'
  status: string
  branded: boolean
  scored: boolean
  updatedAt?: string
}

export type Prompts = { prompts: Prompt[]; counts?: Record<string, number> }

export type AddPromptResult = {
  promptId: string
  existing: boolean
  activeCount: number
  cap: number
}

export type PromptMetrics = {
  visibilityRank: number | null
  visibilityScore: MetricValue
  shareOfVoice: MetricValue
  averagePosition: MetricValue
  citationShare: MetricValue
  citationRank: number | null
  executions: number
}

export type PromptAnalytics = {
  window: { label: string; answerCount: number }
  prompts: Array<{
    id: string
    text: string
    topic: string | null
    intent: string | null
    metrics: PromptMetrics
  }>
}

export type Answer = {
  id: string
  promptId: string
  promptText: string | null
  platform: string
  observedAt: string
  answer: { text: string | null; source: string | null; truncated: boolean; length: number | null }
  brandMentioned: boolean | null
  brandRecommended: boolean | null
  ownDomainCited: boolean | null
  position: number | null
  sentiment: string | null
  brandsMentioned: Array<{
    name: string
    owned: boolean
    recommended: boolean | null
    listRank: number | null
    sentiment: string | null
  }>
  citedUrls?: Array<string | { url: string; title?: string | null }>
  citedUrlCount?: number
}

export type AnswerPage = {
  answers: Answer[]
  pageInfo: { limit: number; hasNextPage: boolean; nextCursor?: string; totalCount?: number }
  nextCursor: string | null
}

export type BrandProfile = {
  version: number
  positioning: string | null
  audience: string | null
  productDescription: string | null
  competitors: string[]
  competitorRoster?: Array<{ name: string; domain: string | null; aliases: string[] }>
}

export type VisibilityRun = {
  runId: string
  projectId: string
  promptCount: number
  platformCount: number
  expectedMinutes: number
}
