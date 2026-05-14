/**
 * `withAuth(fetcher)` — wraps an ofetch-like call with the current session.
 * Adds `Authorization: Bearer …`, refreshes on 401, re-reads the marker file
 * before refreshing so concurrent CLI invocations can share a rotated token.
 *
 * Refresh is never preemptive. SKILLD_TOKEN env scheme is treated as hard
 * expiry: a 401 propagates instead of triggering refresh.
 */

import type { TokenResponse } from './types.ts'
import { ofetch } from 'ofetch'
import { getRegistryBase } from '../registry/client.ts'
import { loadSession, saveSession } from './store.ts'

export interface AuthedFetcher {
  <T>(url: string, init?: Parameters<typeof ofetch<T>>[1]): Promise<T>
}

async function refreshSession(refreshToken: string): Promise<TokenResponse | null> {
  const base = getRegistryBase()
  return ofetch<TokenResponse>(`${base}/cli/oauth/refresh`, {
    method: 'POST',
    body: { refresh_token: refreshToken },
  }).catch(() => null)
}

export function withAuth(): AuthedFetcher {
  return async <T>(url: string, init?: Parameters<typeof ofetch<T>>[1]): Promise<T> => {
    const session = await loadSession()
    if (!session)
      throw new Error('auth required')

    const send = (token: string): Promise<T> => ofetch<T>(url, {
      ...init,
      headers: { ...(init?.headers as any), Authorization: `Bearer ${token}` },
    })

    const fail401Codes = new Set([401, 403])

    const firstAttempt = await send(session.accessToken).catch((err: { statusCode?: number } & Error) => err)
    if (!(firstAttempt instanceof Error) || !fail401Codes.has((firstAttempt as { statusCode?: number }).statusCode ?? 0))
      return firstAttempt as T

    if (session.scheme === 'env' || !session.refreshToken)
      throw firstAttempt

    // Re-read marker; another process may have already rotated.
    const fresh = await loadSession()
    const candidateRefresh = fresh?.refreshToken ?? session.refreshToken
    if (fresh && fresh.accessToken !== session.accessToken) {
      return send(fresh.accessToken)
    }

    const rotated = await refreshSession(candidateRefresh)
    if (!rotated)
      throw firstAttempt

    await saveSession({
      login: rotated.login,
      accessToken: rotated.accessToken,
      refreshToken: rotated.refreshToken,
      expiresAt: rotated.expiresAt,
      tokens: { accessToken: rotated.accessToken, refreshToken: rotated.refreshToken },
    })

    return send(rotated.accessToken)
  }
}
