/**
 * Anonymous telemetry → `POST <registry>/events/cli`. Fire-and-forget; never
 * throws into a caller, never blocks shutdown.
 *
 * Opt-out: `SKILLD_TELEMETRY=0`, `DISABLE_TELEMETRY=1`, or `DO_NOT_TRACK=1`.
 * Auto-disabled in CI.
 */

import { isCI } from 'std-env'
import { getRegistryBase } from './registry/client.ts'
import { version } from './version.ts'

export type TelemetryEvent
  = | 'install'
    | 'install-failed'
    | 'update'
    | 'audit-warn'
    | 'audit-fail'
    | 'audit-blocked'
    | 'auth-flow'
    | 'pull-checklist'

export type TelemetrySurface
  = | 'cli:add'
    | 'cli:pull'
    | 'cli:prepare'
    | 'cli:update'
    | 'cli:wizard'
    | 'cli:auth'

export interface TelemetryPayload {
  event: TelemetryEvent
  surface: TelemetrySurface
  sourceKind?: 'npm' | 'gh' | 'crate' | 'collection' | 'curator'
  slug?: string
  agent?: string
  durationMs?: number
  userId?: number
  /** auth-flow only */
  flow?: 'pkce' | 'device' | 'oidc'
}

function isEnabled(): boolean {
  if (process.env.SKILLD_TELEMETRY === '0')
    return false
  if (process.env.DISABLE_TELEMETRY || process.env.DO_NOT_TRACK)
    return false
  return true
}

export function track(payload: TelemetryPayload): void {
  if (!isEnabled())
    return

  const body = {
    ...payload,
    cliVersion: version,
    ...(isCI && { ci: true }),
  }

  fetch(`${getRegistryBase()}/events/cli`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  }).catch(() => {})
}
