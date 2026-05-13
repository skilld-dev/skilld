/**
 * GitHub Actions OIDC exchange. Auto-detected via `ACTIONS_ID_TOKEN_REQUEST_TOKEN`;
 * fetches a short-lived JWT against `audience=skilld.dev` and trades it for a
 * session token. No browser, no prompt, no refresh.
 */

import type { TokenResponse } from './types.ts'
import { ofetch } from 'ofetch'

interface GhaOidcResponse {
  value: string
  count?: number
}

export function isGhaOidcAvailable(): boolean {
  return !!(process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN && process.env.ACTIONS_ID_TOKEN_REQUEST_URL)
}

export async function runOidcExchange(opts: { registryBase: string, audience?: string }): Promise<TokenResponse> {
  const token = process.env.ACTIONS_ID_TOKEN_REQUEST_TOKEN
  const url = process.env.ACTIONS_ID_TOKEN_REQUEST_URL
  if (!token || !url)
    throw new Error('Not running in GitHub Actions with id-token: write permission')

  const audience = opts.audience ?? 'skilld.dev'
  const idToken = await ofetch<GhaOidcResponse>(`${url}&audience=${encodeURIComponent(audience)}`, {
    headers: { Authorization: `Bearer ${token}` },
  })

  return ofetch<TokenResponse>(`${opts.registryBase}/cli/oidc/exchange`, {
    method: 'POST',
    body: { id_token: idToken.value },
  })
}
