// Thin fetch wrapper for the same-origin API. Cookies carry the dashboard session.

import { QueryClient } from '@tanstack/react-query'
import type { components } from '@/lib/api-types'

export type Me = components['schemas']['Me']
export type ProjectSummary = components['schemas']['ProjectSummary']

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message)
  }
}

/** True for a `/v1` route the API has not built yet: the UI shows the reason, not a crash. */
export function isNotImplemented(error: unknown): boolean {
  return error instanceof ApiError && error.status === 501
}

export function errorText(error: unknown): string {
  if (error instanceof ApiError) return error.message
  return 'The request failed. Check the API and try again.'
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(path, {
    credentials: 'same-origin',
    ...init,
    headers: { 'content-type': 'application/json', ...init.headers },
  })
  const text = await response.text()
  let body: { error?: string; code?: string } | null = null
  try {
    body = text ? JSON.parse(text) : null
  } catch {
    body = null
  }
  if (!response.ok) {
    throw new ApiError(response.status, body?.code ?? 'error', body?.error ?? response.statusText)
  }
  return body as T
}

// `/auth` is outside the `/v1` contract, so its bodies are typed here.
export type SessionInfo = {
  user: { id: string; name: string | null; email: string }
  workspace_id: string
  role: string
}

export function session(): Promise<Me> {
  return api<Me>('/auth/me')
}

export const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false, staleTime: 15_000 } },
})
