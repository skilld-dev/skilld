/**
 * RFC 8628 device flow. Used when --device is passed, no browser is detected,
 * or PKCE bind fails.
 */

import type { TokenResponse } from './types.ts'
import { ofetch } from 'ofetch'

export interface DeviceStartResponse {
  device_code: string
  user_code: string
  verification_uri: string
  interval: number
  expires_in: number
}

export interface DeviceFlowOptions {
  registryBase: string
  cliVersion: string
  machineHint?: string
  /** Hook called once the user_code is known so the CLI can prompt the user. */
  onUserCode: (info: { userCode: string, verificationUri: string }) => void
  /** Override polling interval for tests. */
  intervalMs?: number
}

interface PollResponse {
  status: 'pending' | 'authorized' | 'expired' | 'denied'
  tokens?: TokenResponse
}

export async function runDeviceFlow(opts: DeviceFlowOptions): Promise<TokenResponse> {
  const start = await ofetch<DeviceStartResponse>(`${opts.registryBase}/cli/device/start`, {
    method: 'POST',
    body: { cli_version: opts.cliVersion, machine_hint: opts.machineHint },
  })

  opts.onUserCode({ userCode: start.user_code, verificationUri: start.verification_uri })

  const deadline = Date.now() + start.expires_in * 1000
  const interval = opts.intervalMs ?? start.interval * 1000

  while (Date.now() < deadline) {
    await new Promise(resolve => setTimeout(resolve, interval))
    const poll = await ofetch<PollResponse>(`${opts.registryBase}/cli/device/poll`, {
      method: 'POST',
      body: { device_code: start.device_code },
    }).catch(() => null)

    if (!poll || poll.status === 'pending')
      continue
    if (poll.status === 'expired')
      throw new Error('Device code expired before authorization')
    if (poll.status === 'denied')
      throw new Error('Device authorization denied')
    if (poll.status === 'authorized' && poll.tokens)
      return poll.tokens
  }

  throw new Error('Device authorization timed out')
}
