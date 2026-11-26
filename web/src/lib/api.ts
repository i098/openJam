// Thin fetch wrapper for the same-origin API. Cookies carry the session.
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message)
  }
}

export async function api<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(path, {
    credentials: 'same-origin',
    headers: { 'content-type': 'application/json', ...init.headers },
    ...init,
  })
  const text = await response.text()
  const body = text ? JSON.parse(text) : null
  if (!response.ok) {
    throw new ApiError(response.status, body?.code ?? 'error', body?.error ?? response.statusText)
  }
  return body as T
}

export type SessionInfo = {
  user: { id: string; name: string | null; email: string }
  workspace_id: string
  role: 'owner' | 'member'
}

export type ProjectList = {
  projects: Array<{ id: string; name: string; domain: string; siteUrl: string; status: string }>
}
