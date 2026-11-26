import { createRoute, useNavigate } from '@tanstack/react-router'
import { type FormEvent, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { ApiError, api, type SessionInfo } from '@/lib/api'
import { rootRoute } from '@/router'

export const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/login',
  component: LoginPage,
})

function LoginPage() {
  const navigate = useNavigate()
  const [mode, setMode] = useState<'signin' | 'signup'>('signin')
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [name, setName] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  async function submit(event: FormEvent) {
    event.preventDefault()
    setBusy(true)
    setError(null)
    try {
      const path = mode === 'signin' ? '/auth/login' : '/auth/signup'
      await api<SessionInfo>(path, {
        method: 'POST',
        body: JSON.stringify({ email, password, ...(mode === 'signup' ? { name } : {}) }),
      })
      await navigate({ to: '/' })
    } catch (e) {
      setError(e instanceof ApiError ? e.message : 'Network error. Try again.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="mx-auto flex min-h-dvh w-full max-w-md flex-col justify-center px-6 py-12">
      <h1 className="text-2xl font-semibold tracking-tight">openJam</h1>
      <p className="mt-2 text-sm text-muted">
        See how AI assistants answer your customers' questions.
      </p>
      <form onSubmit={submit} className="mt-8 flex flex-col gap-3">
        {mode === 'signup' && (
          <label htmlFor="name" className="flex flex-col gap-1.5 text-sm">
            Name
            <Input
              id="name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              autoComplete="name"
            />
          </label>
        )}
        <label htmlFor="email" className="flex flex-col gap-1.5 text-sm">
          Email
          <Input
            id="email"
            type="email"
            required
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            autoComplete="email"
          />
        </label>
        <label htmlFor="password" className="flex flex-col gap-1.5 text-sm">
          Password
          <Input
            id="password"
            type="password"
            required
            minLength={8}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete={mode === 'signin' ? 'current-password' : 'new-password'}
          />
        </label>
        {error && <p className="text-sm text-danger">{error}</p>}
        <Button type="submit" disabled={busy} className="mt-2">
          {busy ? 'Working…' : mode === 'signin' ? 'Sign in' : 'Create account'}
        </Button>
      </form>
      <button
        type="button"
        className="mt-6 text-sm text-muted underline underline-offset-4"
        onClick={() => setMode(mode === 'signin' ? 'signup' : 'signin')}
      >
        {mode === 'signin' ? 'New here? Create an account' : 'Have an account? Sign in'}
      </button>
    </main>
  )
}
