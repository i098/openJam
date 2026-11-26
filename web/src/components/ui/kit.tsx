import { useMutation } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { Button } from '@/components/ui/button'
import { api, errorText, isNotImplemented } from '@/lib/api'
import { cn } from '@/lib/cn'
import type { VisibilityRun } from '@/lib/views'

export function Card({
  title,
  action,
  children,
  className,
}: {
  title?: string
  action?: ReactNode
  children: ReactNode
  className?: string
}) {
  return (
    <section className={cn('rounded-lg border border-border bg-background p-4 sm:p-5', className)}>
      {(title || action) && (
        <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
          {title && <h2 className="text-base font-semibold">{title}</h2>}
          {action}
        </div>
      )}
      {children}
    </section>
  )
}

export function Loading({ label = 'Loading' }: { label?: string }) {
  return (
    <p role="status" className="py-8 text-sm text-muted">
      {label}…
    </p>
  )
}

/** A failed request. A 501 means the API route is not built yet, which is not a user error. */
export function Problem({ error }: { error: unknown }) {
  const pending = isNotImplemented(error)
  return (
    <div
      role="alert"
      className={cn(
        'rounded-md border px-3 py-2 text-sm',
        pending
          ? 'border-warning/40 bg-warning/10 text-foreground'
          : 'border-danger/40 bg-danger/10 text-foreground',
      )}
    >
      {pending
        ? 'This API route is not built yet. The screen fills in when the API ships it.'
        : errorText(error)}
    </div>
  )
}

export function Pill({
  children,
  tone = 'neutral',
}: {
  children: ReactNode
  tone?: 'neutral' | 'good' | 'warn' | 'bad'
}) {
  const tones = {
    neutral: 'bg-surface text-foreground border-border',
    good: 'bg-success/10 text-success border-success/30',
    warn: 'bg-warning/10 text-warning border-warning/30',
    bad: 'bg-danger/10 text-danger border-danger/30',
  }
  return (
    <span
      className={cn(
        'ml-1.5 inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium',
        tones[tone],
      )}
    >
      {children}
    </span>
  )
}

/** A table that reads as stacked label: value rows on a phone, so no column is cut off. */
export function TableBox({ children }: { children: ReactNode }) {
  return (
    <table className="w-full text-left text-sm max-sm:[&_thead]:sr-only max-sm:[&_tr]:block max-sm:[&_tr]:border-t max-sm:[&_tr]:border-border max-sm:[&_tr]:py-2 max-sm:[&_tbody]:block">
      {children}
    </table>
  )
}

export const th =
  'whitespace-nowrap py-2 pr-4 text-xs font-medium uppercase tracking-wide text-muted'

/** `label` is the column name, shown beside the value on a phone. */
export function Td({
  label,
  className,
  children,
}: {
  label?: string
  className?: string
  children: ReactNode
}) {
  return (
    <td
      data-label={label}
      className={cn(
        'border-t border-border py-2 pr-4 max-sm:flex max-sm:items-baseline max-sm:gap-3 max-sm:border-0 max-sm:py-0.5 max-sm:pr-0',
        label &&
          'max-sm:justify-between max-sm:before:text-xs max-sm:before:uppercase max-sm:before:text-muted max-sm:before:content-[attr(data-label)]',
        className,
      )}
    >
      {children}
    </td>
  )
}

export function ScanButton({ onStarted }: { onStarted?: () => void }) {
  const scan = useMutation({
    mutationFn: () => api<VisibilityRun>('/v1/visibility-runs', { method: 'POST', body: '{}' }),
    onSuccess: () => onStarted?.(),
  })
  return (
    <div className="flex flex-col items-start gap-1">
      <Button size="sm" disabled={scan.isPending} onClick={() => scan.mutate()}>
        {scan.isPending ? 'Starting…' : 'Run scan'}
      </Button>
      {scan.data && (
        <p role="status" className="text-sm">
          Scan queued: {scan.data.promptCount} prompts on {scan.data.platformCount} platforms. Most
          answers arrive in about {scan.data.expectedMinutes} min.
        </p>
      )}
      {scan.error && <Problem error={scan.error} />}
    </div>
  )
}
