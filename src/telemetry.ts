/**
 * Anonymous telemetry → `POST <registry>/events/cli`. Fire-and-forget; never
 * throws into a caller, never blocks shutdown.
 *
 * Opt-out: `SKILLD_TELEMETRY=0`, `DISABLE_TELEMETRY=1`, or `DO_NOT_TRACK=1`.
 * Auto-disabled in CI.
 *
 * Wire shape: `CliEventInput` from `skilld-protocol/wire`. CLI-level callers
 * use the `TelemetryPayload` alias (which omits `cliVersion` — `track` fills
 * it from the bundled CLI version).
 */

import type { TelemetryEvent, TelemetrySurface } from 'skilld-protocol/constants'
import type { CliEventInput } from 'skilld-protocol/wire'
import { isCI } from 'std-env'
import { getRegistryBase } from './registry/client.ts'
import { version } from './version.ts'

export type { TelemetryEvent, TelemetrySurface }

export type TelemetryPayload = Omit<CliEventInput, 'cliVersion'>

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

  const body: CliEventInput = {
    ...payload,
    cliVersion: version,
  }

  fetch(`${getRegistryBase()}/events/cli`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ ...body, ...(isCI && { ci: true }) }),
  }).catch(() => {})
}
