/**
 * `withAuth(baseUrl)` wraps an ofetch-like call with the current session.
 * Adds `Authorization: Bearer …`, refreshes on 401, re-reads the marker file
 * before refreshing so concurrent CLI invocations can share a rotated token.
 *
 * Refresh is never preemptive. SKILLD_TOKEN env scheme is treated as hard
 * expiry: a 401 propagates instead of triggering refresh.
 */

import type { StorageScheme, StoredSession } from './store.ts'
import type { TokenResponse } from './types.ts'
import { ofetch } from 'ofetch'
import { loadSession, saveSession } from './store.ts'

export interface AuthedFetcher {
  <T>(url: string, init?: Parameters<typeof ofetch<T>>[1]): Promise<T>
}

interface AuthenticatedFetchDependencies {
  baseUrl: string
  fetch: AuthedFetcher
  loadSession: () => Promise<StoredSession | null>
  saveSession: (session: Parameters<typeof saveSession>[0]) => Promise<StorageScheme>
}

type FetchAttempt<T>
  = | { _tag: 'Ok', value: T }
    | { _tag: 'Err', error: unknown }

function isAuthFailure(error: unknown): boolean {
  if (typeof error !== 'object' || error === null || !('statusCode' in error))
    return false
  const statusCode = (error as { statusCode?: unknown }).statusCode
  return statusCode === 401 || statusCode === 403
}

export function createAuthenticatedFetch(deps: AuthenticatedFetchDependencies): AuthedFetcher {
  return async <T>(url: string, init?: Parameters<typeof ofetch<T>>[1]): Promise<T> => {
    const session = await deps.loadSession()
    if (!session)
      throw new Error('auth required')

    const send = (token: string): Promise<T> => deps.fetch<T>(url, {
      ...init,
      headers: { ...(init?.headers as any), Authorization: `Bearer ${token}` },
    })

    const firstAttempt: FetchAttempt<T> = await send(session.accessToken)
      .then(value => ({ _tag: 'Ok' as const, value }))
      .catch(error => ({ _tag: 'Err' as const, error }))
    if (firstAttempt._tag === 'Ok')
      return firstAttempt.value
    if (!isAuthFailure(firstAttempt.error))
      throw firstAttempt.error

    if (session.scheme === 'env' || !session.refreshToken)
      throw firstAttempt.error

    // Re-read marker; another process may have already rotated.
    const fresh = await deps.loadSession()
    const candidateRefresh = fresh?.refreshToken ?? session.refreshToken
    if (fresh && fresh.accessToken !== session.accessToken)
      return send(fresh.accessToken)

    const rotated = await deps.fetch<TokenResponse>(`${deps.baseUrl}/cli/oauth/refresh`, {
      method: 'POST',
      body: { refresh_token: candidateRefresh },
    })

    await deps.saveSession({
      login: rotated.login,
      accessToken: rotated.accessToken,
      refreshToken: rotated.refreshToken,
      expiresAt: rotated.expiresAt,
      host: session.host,
      tokens: { accessToken: rotated.accessToken, refreshToken: rotated.refreshToken },
    })

    return send(rotated.accessToken)
  }
}

export function withAuth(baseUrl: string): AuthedFetcher {
  return createAuthenticatedFetch({
    baseUrl,
    fetch: ofetch,
    loadSession,
    saveSession,
  })
}
